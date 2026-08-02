//! Finding a game on the LAN, so joining one takes no typing.
//!
//! This is a *discovery* layer and nothing else. What a [`Search`] finds is an
//! [`EndpointAddr`], handed straight to the same [`super::boot`] dial a pasted ticket
//! goes through — so a game found on the LAN and a game joined from a ticket are the
//! same join over the same iroh transport, and there is no second netcode to keep in
//! step.
//!
//! Ask-and-answer rather than a periodic beacon. A host binds [`PORT`] and answers; a
//! player at the join menu asks from a port of its own. That way exactly one socket per
//! machine wants the well-known port, a list appears the moment the menu opens instead
//! of one beacon-interval later, and a host nobody is looking for says nothing at all.

use anyhow::{Context, Result};
use iroh::{EndpointAddr, TransportAddr};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use super::PlayerId;
use super::wire;

/// The port a host answers on. Fixed, because a broadcast question has nowhere to look
/// it up.
pub const PORT: u16 = 47331;

/// A host that has not answered this recently has stopped hosting, and drops off the
/// list. Long enough that one lost question does not make a game flicker, short enough
/// that a game somebody closed is gone before a child walks the cursor onto it.
const FORGET_AFTER: Duration = Duration::from_secs(5);

/// Where a question is shouted.
///
/// Linux delivers a subnet or limited broadcast to *other* machines only — a socket on
/// the sending machine never sees its own — so the loopback network's broadcast address
/// is asked separately. That second entry is what lets a host and a joiner on one
/// machine find each other, which is how this is tested.
const SHOUT_AT: [Ipv4Addr; 2] = [Ipv4Addr::BROADCAST, Ipv4Addr::new(127, 255, 255, 255)];

/// Bytes a single question or answer may occupy. One datagram, comfortably inside any
/// LAN's MTU, so an answer is never a fragment that a switch may drop half of.
const MAX_PACKET: usize = 1024;

/// What travels between a searching player and a host.
///
/// The [`super::ALPN`] tag rides along and is checked on receipt, so builds whose wire
/// format has moved on cannot see each other: a game that shows up in the list is a game
/// this build can actually play.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
enum Packet {
    /// "Who is hosting?" — broadcast by a player at the join menu.
    Question { alpn: Vec<u8> },
    /// A host's answer: what to call it, and enough to dial it without a relay, a
    /// nameserver or an internet.
    Answer {
        alpn: Vec<u8>,
        name: String,
        host: PlayerId,
        /// The host's direct addresses. A [`PlayerId`] alone is dialable only via the
        /// address-lookup services, which want an internet; the whole point here is a
        /// front room with a switch and no uplink.
        ips: Vec<SocketAddr>,
    },
}

/// A game somebody on this network is hosting.
#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    /// What the join menu calls it — the host machine's name.
    pub name: String,
    /// Dialed exactly as a ticket is.
    pub addr: EndpointAddr,
}

/// The host's half: answers questions with where to find this game.
///
/// Answers are unicast back to whoever asked, so nothing here depends on a broadcast
/// reaching a socket on the asking machine.
pub struct Beacon {
    sock: UdpSocket,
    name: String,
}

impl Beacon {
    /// Claims [`PORT`] and starts answering. Fails if something else already holds it —
    /// a second host on one machine is the usual reason, and the caller says so out loud
    /// rather than hosting a game that silently cannot be found.
    pub fn open(name: String, port: u16) -> Result<Self> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
            .with_context(|| format!("listening for join questions on port {port}"))?;
        sock.set_nonblocking(true)
            .context("making the discovery socket non-blocking")?;
        Ok(Beacon { sock, name })
    }

    /// Answers everything asked since the last call, and returns how many. Never blocks,
    /// so this is a frame's worth of work whatever the network is doing.
    pub fn answer(&self, addr: &EndpointAddr) -> usize {
        let reply = match wire::encode(&Packet::Answer {
            alpn: super::ALPN.to_vec(),
            name: self.name.clone(),
            host: addr.id,
            ips: addr.ip_addrs().copied().collect(),
        }) {
            Ok(bytes) if bytes.len() <= MAX_PACKET => bytes,
            // An answer nobody can receive is not one to send. Both arms are the same
            // bug — this host's own address list outgrew a datagram — and neither is
            // worth a per-frame complaint.
            _ => return 0,
        };
        let mut answered = 0;
        let mut buf = [0u8; MAX_PACKET];
        while let Ok((n, asker)) = self.sock.recv_from(&mut buf) {
            if !matches!(wire::decode::<Packet>(&buf[..n]), Ok(Packet::Question { alpn }) if alpn == super::ALPN)
            {
                continue;
            }
            if self.sock.send_to(&reply, asker).is_ok() {
                answered += 1;
            }
        }
        answered
    }
}

/// The joining player's half: asks who is hosting and keeps the answers.
pub struct Search {
    sock: UdpSocket,
    port: u16,
    /// Keyed by host so two answers from one host — the LAN broadcast and the loopback
    /// one both reach a host on this machine — are one entry in the list.
    seen: HashMap<PlayerId, (Game, Instant)>,
}

impl Search {
    /// Opens a socket to ask from. The port is the system's to choose: only hosts need a
    /// port anyone can guess.
    pub fn open(port: u16) -> Result<Self> {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .context("opening a socket to look for games with")?;
        sock.set_broadcast(true)
            .context("asking for permission to broadcast")?;
        sock.set_nonblocking(true)
            .context("making the discovery socket non-blocking")?;
        Ok(Search {
            sock,
            port,
            seen: HashMap::new(),
        })
    }

    /// Shouts the question. Each destination is tried independently: a machine with no
    /// default route cannot send to [`Ipv4Addr::BROADCAST`] at all, and that must not
    /// cost it the loopback question — nor the other way round.
    pub fn ask(&self) {
        let Ok(question) = wire::encode(&Packet::Question {
            alpn: super::ALPN.to_vec(),
        }) else {
            return;
        };
        for net in SHOUT_AT {
            let _ = self.sock.send_to(&question, (net, self.port));
        }
    }

    /// Takes in whatever answers have arrived and forgets hosts that have gone quiet.
    /// Never blocks.
    pub fn collect(&mut self) {
        let mut buf = [0u8; MAX_PACKET];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            let Ok(Packet::Answer {
                alpn,
                name,
                host,
                ips,
            }) = wire::decode::<Packet>(&buf[..n])
            else {
                continue;
            };
            if alpn != super::ALPN {
                continue;
            }
            let game = Game {
                name: readable(&name),
                addr: EndpointAddr::from_parts(host, dialable(&ips, from.ip())),
            };
            self.seen.insert(host, (game, Instant::now()));
        }
        self.seen
            .retain(|_, (_, heard)| heard.elapsed() < FORGET_AFTER);
    }

    /// The games to offer, in a stable order — a list that reshuffles under the cursor
    /// is a list a child presses A on and joins the wrong game.
    pub fn games(&self) -> Vec<Game> {
        let mut games: Vec<Game> = self.seen.values().map(|(game, _)| game.clone()).collect();
        games.sort_by(|a, b| (&a.name, a.addr.id.as_bytes()).cmp(&(&b.name, b.addr.id.as_bytes())));
        games
    }
}

/// The addresses to actually dial a host on.
///
/// A host lists every address it has bound, which on a machine with docker bridges is
/// mostly addresses that go nowhere from here. The address the answer *came from* is the
/// one address known to work in this direction, so it is always offered — paired with
/// each port the host claims, since the answer arrived on the discovery port rather than
/// on iroh's.
fn dialable(claimed: &[SocketAddr], answered_from: IpAddr) -> Vec<TransportAddr> {
    let mut addrs: Vec<TransportAddr> = claimed.iter().copied().map(TransportAddr::Ip).collect();
    let ports: Vec<u16> = claimed.iter().map(|a| a.port()).collect();
    for port in ports {
        let proven = SocketAddr::new(answered_from, port);
        if !claimed.contains(&proven) {
            addrs.push(TransportAddr::Ip(proven));
        }
    }
    addrs
}

/// A host's name as it is safe to draw: one line, bounded, and never empty.
///
/// The name arrives over the network from a machine nobody here vouches for, so a
/// kilobyte of newlines must not become a menu that fills the screen.
fn readable(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect::<String>()
        .trim()
        .to_string();
    if cleaned.is_empty() {
        "a nameless world".to_string()
    } else {
        cleaned
    }
}

/// What to call this machine's game: its hostname, which is what its owner already calls
/// it. Linux keeps it in `/proc`; anywhere else, and a game is still hostable under a
/// name that says so.
pub fn this_machine() -> String {
    readable(&std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> PlayerId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    /// The whole point of the layer: a host answers, a searcher lists it, and what it
    /// lists is dialable. Both halves are on this machine, which is the case the
    /// loopback entry of [`SHOUT_AT`] exists for.
    #[test]
    fn a_host_on_this_machine_is_found() {
        let port = 47411;
        let beacon = Beacon::open("test-box".into(), port).unwrap();
        let mut search = Search::open(port).unwrap();
        let host = EndpointAddr::from_parts(
            id(7),
            [TransportAddr::Ip("192.168.9.9:5000".parse().unwrap())],
        );

        search.ask();
        // At least one, not exactly one: [`SHOUT_AT`] asks twice, and whether a machine
        // hears its own limited broadcast as well as its loopback one is the kernel's
        // business and the routing table's. A host answering the same asker twice costs
        // a datagram and is deduplicated by `collect`.
        assert!(beacon.answer(&host) >= 1, "the question went unanswered");
        // Give the loopback datagram a moment; it is a syscall away, not a network away.
        std::thread::sleep(Duration::from_millis(50));
        search.collect();

        let games = search.games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "test-box");
        assert_eq!(games[0].addr.id, id(7));
        assert!(
            games[0]
                .addr
                .ip_addrs()
                .any(|a| a == &"192.168.9.9:5000".parse().unwrap()),
            "the host's own address was dropped"
        );
    }

    /// A build whose wire format has moved on must not appear in the list: it would be a
    /// game you can see and cannot join.
    #[test]
    fn a_stranger_protocol_is_ignored() {
        let port = 47412;
        let beacon = Beacon::open("test-box".into(), port).unwrap();
        let asker = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap();
        asker.set_broadcast(true).unwrap();
        let question = wire::encode(&Packet::Question {
            alpn: b"bddap-bot/blockgame/999".to_vec(),
        })
        .unwrap();
        asker
            .send_to(&question, (Ipv4Addr::new(127, 255, 255, 255), port))
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            beacon.answer(&EndpointAddr::new(id(3))),
            0,
            "answered a build that cannot play this game"
        );
    }

    /// The address an answer arrived from is the one address proven to reach that host
    /// from here — a host behind a docker bridge lists addresses that are unreachable
    /// from this side, and dialing only those is dialing nothing.
    #[test]
    fn the_address_that_answered_is_always_dialable() {
        let claimed: Vec<SocketAddr> = vec!["172.17.0.1:5000".parse().unwrap()];
        let addrs = dialable(&claimed, "192.168.1.20".parse().unwrap());
        assert!(addrs.contains(&TransportAddr::Ip("172.17.0.1:5000".parse().unwrap())));
        assert!(
            addrs.contains(&TransportAddr::Ip("192.168.1.20:5000".parse().unwrap())),
            "the address the answer came from was not offered"
        );
    }

    /// A name is drawn on a menu, so a hostile host must not be able to draw the menu.
    #[test]
    fn a_name_is_one_bounded_line() {
        assert_eq!(readable("living-room"), "living-room");
        assert_eq!(readable("  spaced  "), "spaced");
        assert_eq!(readable(""), "a nameless world");
        assert_eq!(readable("a\nb"), "ab");
        assert_eq!(readable(&"x".repeat(500)).chars().count(), 32);
    }

    /// Discovery is a datagram, and a datagram that outgrows the MTU is one a switch may
    /// deliver half of.
    #[test]
    fn an_answer_fits_in_one_datagram() {
        let ips: Vec<SocketAddr> = (0..8)
            .map(|n| SocketAddr::new(IpAddr::from([10, 0, 0, n]), 5000))
            .collect();
        let packet = wire::encode(&Packet::Answer {
            alpn: super::super::ALPN.to_vec(),
            name: "x".repeat(32),
            host: id(1),
            ips,
        })
        .unwrap();
        assert!(packet.len() <= MAX_PACKET, "{} bytes", packet.len());
    }
}
