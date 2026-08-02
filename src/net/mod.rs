//! Multiplayer transport: an iroh message bus that moves [`Msg`]s between endpoints.
//!
//! There is no single-player code path. `blockgame` with no arguments is a host with zero
//! peers connected; `blockgame join <TICKET>` is a peer. The same systems run either way.
//!
//! The bus owns a set of links. A host listens, so it has N links; a peer never listens,
//! so it has exactly one — the host it dialled, named by [`Role::Peer`] — and `Target::All`
//! means the same thing to both. That is the transport half of the trust boundary: a
//! stranger cannot get a message *in* to a peer at all. The other half is
//! [`crate::game::authorized`], which decides what a sender is entitled to say.

pub mod wire;

use anyhow::{Context, Result, anyhow};
use iroh::Endpoint;
use iroh::endpoint::{Connection, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};

use crate::registry::Block;
use crate::world::BlockPos;
pub use wire::{Msg, PlayerId, Pose};

/// Bumped whenever the wire format changes incompatibly — mismatched builds then fail to
/// connect instead of desyncing silently.
pub const ALPN: &[u8] = b"bddap-bot/blockgame/1";

/// How long a joining peer waits for the host's `Welcome` before giving up.
const JOIN_TIMEOUT: Duration = Duration::from_secs(20);
/// A peer that can't absorb a reliable frame in this long is dropped. Each link has its
/// own writer task, so this bounds only that link's own life — a wedged peer never delays
/// anybody else's traffic, whatever it does with its window.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Frames a link may have queued before it is considered wedged and dropped. Sends never
/// block the game, so a peer that stops reading has to be bounded somewhere; here, where
/// the memory is.
const SEND_QUEUE: usize = 64;
/// Inbound connections a host will hold at once. Dialing is cheap and each link costs a
/// read buffer and two tasks, so this is the ceiling on what a stranger can make a host
/// allocate.
const MAX_INBOUND_LINKS: usize = 32;
/// Bytes read from a stream per syscall. The frame header is a peer's *claim* about bytes
/// it has not sent; reading in bounded bites means memory tracks what actually arrived
/// rather than what was promised.
const READ_CHUNK: usize = 64 * 1024;

/// Which side of the host-authoritative relationship this process is on.
///
/// A peer carries the host's id rather than a bare marker: every peer-side trust decision
/// is "is this the host?", and there is no moment in a peer's life when the answer is
/// unknown — [`boot`] does not return until the host has answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Simulates the world and is the source of truth for every edit.
    Host,
    /// Sends intents, applies what the host says.
    Peer { host: PlayerId },
}

/// What the game loop sees coming off the wire.
#[derive(Debug)]
pub enum Event {
    Message(PlayerId, Msg),
    /// A link went away. On the host this means a player left; on a peer it means the
    /// host is gone.
    Left(PlayerId),
}

#[derive(Debug, Clone, Copy)]
pub enum Target {
    All,
    One(PlayerId),
}

/// Everything [`boot`] hands the game: a live session plus the world it should build.
pub struct Boot {
    pub session: Session,
    pub role: Role,
    pub seed: u64,
    pub edits: Vec<(BlockPos, Block)>,
}

/// A live network session. Dropping it tears the endpoint down.
pub struct Session {
    /// The tokio runtime the connection tasks live on. Held so they keep running; the
    /// game never blocks on it — sends and receives are non-blocking channel ops.
    _rt: tokio::runtime::Runtime,
    _router: Router,
    me: PlayerId,
    inbox: mpsc::UnboundedReceiver<Event>,
    outbox: mpsc::UnboundedSender<Outbound>,
}

impl Session {
    /// This player's id — and, for a host, the ticket a friend needs to join.
    pub fn me(&self) -> PlayerId {
        self.me
    }

    pub fn ticket(&self) -> String {
        self.me.to_string()
    }

    /// Non-blocking: returns everything that arrived since the last call.
    pub fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(ev) = self.inbox.try_recv() {
            out.push(ev);
        }
        out
    }

    /// Queues a message. Never blocks and never fails loudly: with no peers connected
    /// (single player) every send is simply dropped by the dispatcher.
    pub fn send(&self, to: Target, msg: Msg) {
        let _ = self.outbox.send(Outbound { to, msg });
    }
}

struct Outbound {
    to: Target,
    msg: Msg,
}

/// Identifies one connection for its lifetime. Handed out by [`Links::insert`] and never
/// reused, so a stale link can only ever remove itself.
type LinkId = u64;

/// Generic in the connection only so the bookkeeping below can be tested without a
/// network — `C` is always [`Connection`] in the game.
struct Link<C> {
    peer: PlayerId,
    conn: C,
    /// Frames waiting for this link's writer task. Bounded — see [`SEND_QUEUE`].
    outbound: mpsc::Sender<Vec<u8>>,
}

/// The live connections, keyed by connection rather than by player.
///
/// Keying by [`PlayerId`] loses a reconnect race: the new link overwrites the old entry,
/// and when the *old* one's reader finally exits it removes the live entry and announces
/// a departure for a player who is still here. A link can only remove its own id, and a
/// player is gone exactly when their last link is.
struct Links<C = Connection> {
    next: LinkId,
    by_id: HashMap<LinkId, Link<C>>,
}

impl<C> Default for Links<C> {
    fn default() -> Self {
        Self {
            next: 0,
            by_id: HashMap::new(),
        }
    }
}

impl<C: Clone> Links<C> {
    fn insert(&mut self, link: Link<C>) -> LinkId {
        let id = self.next;
        self.next += 1;
        self.by_id.insert(id, link);
        id
    }

    /// Removes a link and reports whether that was the departing player's last one.
    fn remove(&mut self, id: LinkId) -> Option<(PlayerId, bool)> {
        let link = self.by_id.remove(&id)?;
        let still_here = self.by_id.values().any(|l| l.peer == link.peer);
        Some((link.peer, !still_here))
    }

    /// Everything a send needs, cloned out so the caller can let go of the lock before it
    /// touches the network.
    fn targets(&self, to: Target) -> Vec<(C, mpsc::Sender<Vec<u8>>)> {
        self.by_id
            .values()
            .filter(|l| match to {
                Target::All => true,
                Target::One(id) => l.peer == id,
            })
            .map(|l| (l.conn.clone(), l.outbound.clone()))
            .collect()
    }
}

struct Bus {
    links: Mutex<Links>,
    /// Inbound connections currently being served, counted separately from `links`
    /// because the limit has to bite before a connection is far enough along to be one.
    inbound: AtomicUsize,
    inbox: mpsc::UnboundedSender<Event>,
}

/// Starts the network. `join` is `None` to host, or the ticket of a host to join.
///
/// Joining blocks until the host's `Welcome` arrives, so the game is built with a
/// complete world in hand and no "world not ready yet" state exists to handle.
pub fn boot(join: Option<PlayerId>, seed: u64) -> Result<Boot> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the network runtime")?;

    let (inbox_tx, mut inbox_rx) = mpsc::unbounded_channel();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let bus = Arc::new(Bus {
        links: Mutex::new(Links::default()),
        inbound: AtomicUsize::new(0),
        inbox: inbox_tx,
    });

    let (me, router, seed, edits) = rt.block_on(async {
        let endpoint = Endpoint::builder(presets::N0)
            .bind()
            .await
            .context("binding the iroh endpoint")?;
        let me = endpoint.id();

        tokio::spawn(dispatch(bus.clone(), out_rx));
        // Only a host listens. A peer that accepted connections would be a second,
        // unauthenticated way into its world — and it has nothing to serve anyone.
        let router = match join {
            None => Router::builder(endpoint.clone())
                .accept(ALPN, Proto { bus: bus.clone() })
                .spawn(),
            Some(_) => Router::builder(endpoint.clone()).spawn(),
        };

        let (seed, edits) = match join {
            None => (seed, Vec::new()),
            Some(host) => {
                let conn = endpoint
                    .connect(host, ALPN)
                    .await
                    .with_context(|| format!("connecting to host {host}"))?;
                let bus = bus.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_link(conn, true, bus).await {
                        eprintln!("connection to host ended: {e:#}");
                    }
                });
                await_welcome(&mut inbox_rx, host).await?
            }
        };
        anyhow::Ok((me, router, seed, edits))
    })?;

    Ok(Boot {
        role: match join {
            None => Role::Host,
            Some(host) => Role::Peer { host },
        },
        seed,
        edits,
        session: Session {
            _rt: rt,
            _router: router,
            me,
            inbox: inbox_rx,
            outbox: out_tx,
        },
    })
}

async fn await_welcome(
    inbox: &mut mpsc::UnboundedReceiver<Event>,
    host: PlayerId,
) -> Result<(u64, Vec<(BlockPos, Block)>)> {
    let wait = async {
        loop {
            match inbox.recv().await {
                // Only the endpoint we dialled gets to say what world this is. Anyone
                // else's `Welcome` would hand a stranger the seed and the whole edit log.
                Some(Event::Message(from, Msg::Welcome { seed, edits })) if from == host => {
                    return Ok((seed, edits));
                }
                // Anything else this early is pre-handshake chatter; the world isn't
                // built yet, so there is nothing that could consume it.
                Some(_) => continue,
                None => return Err(anyhow!("host disconnected during the handshake")),
            }
        }
    };
    tokio::time::timeout(JOIN_TIMEOUT, wait)
        .await
        .map_err(|_| anyhow!("host did not answer within {JOIN_TIMEOUT:?}"))?
}

#[derive(Clone, Debug)]
struct Proto {
    bus: Arc<Bus>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Bus")
    }
}

impl ProtocolHandler for Proto {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let Some(_slot) = InboundSlot::claim(&self.bus) else {
            connection.close(2u32.into(), b"too many connections");
            return Ok(());
        };
        if let Err(e) = serve_link(connection, false, self.bus.clone()).await {
            eprintln!("peer connection ended: {e:#}");
        }
        Ok(())
    }
}

/// One of the [`MAX_INBOUND_LINKS`] inbound slots, released when the connection ends.
///
/// Claimed before the connection is served rather than counted from the link map: the
/// expensive part of a hostile connection — the read buffer, the two tasks — happens
/// before the link is ever inserted, so counting links would bound the wrong thing.
struct InboundSlot(Arc<Bus>);

impl InboundSlot {
    fn claim(bus: &Arc<Bus>) -> Option<Self> {
        bus.inbound
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_INBOUND_LINKS).then_some(n + 1)
            })
            .ok()
            .map(|_| InboundSlot(bus.clone()))
    }
}

impl Drop for InboundSlot {
    fn drop(&mut self) {
        self.0.inbound.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Runs one link for its whole life. Identical for the dialing and accepting sides —
/// only who opens the stream differs.
async fn serve_link(conn: Connection, dialed: bool, bus: Arc<Bus>) -> Result<()> {
    let peer = conn.remote_id();
    let (mut send, recv) = if dialed {
        conn.open_bi().await.context("opening the stream")?
    } else {
        conn.accept_bi().await.context("accepting the stream")?
    };

    // QUIC doesn't surface an opened stream to the acceptor until the dialer writes, so
    // the dialer's first act is to say hello. That is also what asks for the world.
    if dialed {
        send.write_all(&wire::encode_frame(&Msg::Hello)?)
            .await
            .context("saying hello")?;
    }

    let (outbound, frames) = mpsc::channel(SEND_QUEUE);
    let id = bus.links.lock().await.insert(Link {
        peer,
        conn: conn.clone(),
        outbound,
    });

    let writer = tokio::spawn(write_link(send, frames, conn.clone()));
    let datagrams = tokio::spawn(read_datagrams(conn.clone(), peer, bus.clone()));
    let result = read_stream(recv, peer, bus.clone()).await;

    datagrams.abort();
    writer.abort();
    if let Some((peer, last)) = bus.links.lock().await.remove(id)
        && last
    {
        let _ = bus.inbox.send(Event::Left(peer));
    }
    // The datagram task held a Connection clone, so an explicit close is what actually
    // ends the link rather than waiting out the idle timeout.
    conn.close(0u32.into(), b"link closed");
    result
}

/// One link's outbound half. Every reliable write happens here and nowhere else, so a
/// peer that stops reading stalls only its own queue.
async fn write_link(mut send: SendStream, mut frames: mpsc::Receiver<Vec<u8>>, conn: Connection) {
    while let Some(frame) = frames.recv().await {
        match tokio::time::timeout(WRITE_TIMEOUT, send.write_all(&frame)).await {
            Ok(Ok(())) => {}
            // Closing is what makes the reader loop exit, which is the one place a
            // departure is announced.
            _ => {
                conn.close(1u32.into(), b"send failed");
                return;
            }
        }
    }
}

async fn read_stream(
    mut recv: iroh::endpoint::RecvStream,
    peer: PlayerId,
    bus: Arc<Bus>,
) -> Result<()> {
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        let mut header = [0u8; wire::LEN_PREFIX];
        if recv.read_exact(&mut header).await.is_err() {
            return Ok(()); // peer hung up
        }
        let len = wire::frame_len(header).with_context(|| format!("from peer {peer}"))?;
        // Grow with what arrives instead of allocating what the header claims: the claim
        // costs a peer four bytes, and the buffer would cost the host the whole frame.
        let mut buf = Vec::new();
        while buf.len() < len {
            let want = (len - buf.len()).min(READ_CHUNK);
            recv.read_exact(&mut chunk[..want])
                .await
                .context("short read on a framed message")?;
            buf.extend_from_slice(&chunk[..want]);
        }
        let msg = wire::decode(&buf).context("undecodable message")?;
        if bus.inbox.send(Event::Message(peer, msg)).is_err() {
            return Ok(()); // game is gone
        }
    }
}

async fn read_datagrams(conn: Connection, peer: PlayerId, bus: Arc<Bus>) {
    while let Ok(bytes) = conn.read_datagram().await {
        match wire::decode(&bytes) {
            Ok(msg) => {
                if bus.inbox.send(Event::Message(peer, msg)).is_err() {
                    return;
                }
            }
            Err(e) => eprintln!("undecodable datagram from {peer}: {e:#}"),
        }
    }
}

/// Fans one outbound message out to its links. Encodes once, hands each link a frame, and
/// never touches the network under the lock — so no peer's socket can delay another's, nor
/// hold up a link joining or leaving.
async fn dispatch(bus: Arc<Bus>, mut rx: mpsc::UnboundedReceiver<Outbound>) {
    while let Some(out) = rx.recv().await {
        let unreliable = out.msg.via_datagram();
        // Datagrams carry a bare payload; the stream needs its length prefix.
        let bytes = if unreliable {
            wire::encode(&out.msg)
        } else {
            wire::encode_frame(&out.msg)
        };
        let bytes = match bytes {
            Ok(b) => b,
            Err(e) => {
                eprintln!("dropping unencodable message: {e:#}");
                continue;
            }
        };
        let targets = bus.links.lock().await.targets(out.to);
        for (conn, outbound) in targets {
            let delivered = if unreliable {
                debug_assert!(
                    bytes.len() <= wire::MAX_DATAGRAM_LEN,
                    "datagram-class message outgrew the guaranteed datagram size"
                );
                // Fire and forget: under congestion QUIC drops the oldest datagram, which
                // for a pose stream is exactly the right thing to lose.
                conn.send_datagram(bytes.clone().into()).is_ok()
            } else {
                // A full queue means the peer has stopped absorbing frames; dropping it is
                // the only alternative to growing the host's memory on its behalf.
                outbound.try_send(bytes.clone()).is_ok()
            };
            if !delivered {
                // Closing makes the link's reader loop exit, which is the one place a
                // departure is announced.
                conn.close(1u32.into(), b"send failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> PlayerId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    /// A link whose "connection" is just a tag, which is all the bookkeeping touches.
    fn link(links: &mut Links<u8>, peer: PlayerId, conn: u8) -> LinkId {
        let (outbound, _held) = mpsc::channel(1);
        std::mem::forget(_held);
        links.insert(Link {
            peer,
            conn,
            outbound,
        })
    }

    /// The reconnect race: a peer's second connection arrives before its first has torn
    /// down. Keyed by player, the new link would be evicted by the old one's exit and a
    /// still-connected player announced as gone.
    #[test]
    fn a_second_connection_does_not_evict_the_first() {
        let mut links = Links::<u8>::default();
        let peer = id(1);
        let first = link(&mut links, peer, 1);
        let second = link(&mut links, peer, 2);
        assert_ne!(first, second, "each connection gets its own key");
        assert_eq!(links.targets(Target::One(peer)).len(), 2);

        assert_eq!(
            links.remove(first),
            Some((peer, false)),
            "the older link ending leaves the player here"
        );
        assert_eq!(
            links.remove(second),
            Some((peer, true)),
            "the last link ending is the departure"
        );
        assert!(links.remove(second).is_none(), "a link leaves only once");
    }

    #[test]
    fn a_send_to_one_player_reaches_only_their_links() {
        let mut links = Links::<u8>::default();
        let (a, b) = (id(1), id(2));
        link(&mut links, a, 1);
        link(&mut links, b, 2);
        let only_b: Vec<u8> = links
            .targets(Target::One(b))
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(only_b, vec![2]);
        assert_eq!(links.targets(Target::All).len(), 2);
    }
}
