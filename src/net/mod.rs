//! Multiplayer transport: an iroh message bus that knows nothing about the game.
//!
//! There is no single-player code path. `blockgame` with no arguments is a host with zero
//! peers connected; `blockgame join <TICKET>` is a peer. The same systems run either way —
//! see [`crate::game`] for the one place [`Role`] is consulted.
//!
//! The bus itself is role-agnostic: it owns a set of links and moves [`Msg`]s over them.
//! A host has N links, a peer has exactly one (to the host), and `Target::All` means the
//! same thing to both.

pub mod wire;

use anyhow::{Context, Result, anyhow};
use iroh::Endpoint;
use iroh::endpoint::{Connection, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use std::collections::HashMap;
use std::sync::Arc;
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
/// A peer that can't absorb a reliable frame in this long is treated as gone. Without it,
/// one wedged connection would stall sends to everybody.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which side of the host-authoritative relationship this process is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Simulates the world and is the source of truth for every edit.
    Host,
    /// Sends intents, applies what the host says.
    Peer,
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

struct Link {
    conn: Connection,
    send: SendStream,
}

struct Bus {
    links: Mutex<HashMap<PlayerId, Link>>,
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
        links: Mutex::new(HashMap::new()),
        inbox: inbox_tx,
    });

    let (me, router, seed, edits) = rt.block_on(async {
        let endpoint = Endpoint::builder(presets::N0)
            .bind()
            .await
            .context("binding the iroh endpoint")?;
        let me = endpoint.id();

        tokio::spawn(dispatch(bus.clone(), out_rx));
        let router = Router::builder(endpoint.clone())
            .accept(ALPN, Proto { bus: bus.clone() })
            .spawn();

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
                await_welcome(&mut inbox_rx).await?
            }
        };
        anyhow::Ok((me, router, seed, edits))
    })?;

    Ok(Boot {
        role: if join.is_some() {
            Role::Peer
        } else {
            Role::Host
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
) -> Result<(u64, Vec<(BlockPos, Block)>)> {
    let wait = async {
        loop {
            match inbox.recv().await {
                Some(Event::Message(_, Msg::Welcome { seed, edits })) => {
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
        if let Err(e) = serve_link(connection, false, self.bus.clone()).await {
            eprintln!("peer connection ended: {e:#}");
        }
        Ok(())
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
        write_frame(&mut send, &Msg::Hello).await?;
    }

    bus.links.lock().await.insert(
        peer,
        Link {
            conn: conn.clone(),
            send,
        },
    );

    let datagrams = tokio::spawn(read_datagrams(conn.clone(), peer, bus.clone()));
    let result = read_stream(recv, peer, bus.clone()).await;

    datagrams.abort();
    bus.links.lock().await.remove(&peer);
    let _ = bus.inbox.send(Event::Left(peer));
    // The datagram task held a Connection clone, so an explicit close is what actually
    // ends the link rather than waiting out the idle timeout.
    conn.close(0u32.into(), b"link closed");
    result
}

async fn read_stream(
    mut recv: iroh::endpoint::RecvStream,
    peer: PlayerId,
    bus: Arc<Bus>,
) -> Result<()> {
    loop {
        let mut len = [0u8; 4];
        if recv.read_exact(&mut len).await.is_err() {
            return Ok(()); // peer hung up
        }
        let len = u32::from_le_bytes(len) as usize;
        if len > wire::MAX_FRAME_LEN {
            anyhow::bail!("peer {peer} sent an oversized frame: {len} bytes");
        }
        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf)
            .await
            .context("short read on a framed message")?;
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

async fn write_frame(send: &mut SendStream, msg: &Msg) -> Result<()> {
    let payload = wire::encode(msg)?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    send.write_all(&frame).await.context("writing a frame")?;
    Ok(())
}

/// The single outbound path: one task, so stream writes are serialized per link without
/// every caller needing a lock.
async fn dispatch(bus: Arc<Bus>, mut rx: mpsc::UnboundedReceiver<Outbound>) {
    while let Some(out) = rx.recv().await {
        let payload = match wire::encode(&out.msg) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("dropping unencodable message: {e:#}");
                continue;
            }
        };
        let unreliable = out.msg.via_datagram();
        let mut links = bus.links.lock().await;
        let targets: Vec<PlayerId> = match out.to {
            Target::All => links.keys().copied().collect(),
            Target::One(id) => vec![id],
        };
        for id in targets {
            let Some(link) = links.get_mut(&id) else {
                continue;
            };
            let delivered = if unreliable {
                debug_assert!(
                    payload.len() <= wire::MAX_DATAGRAM_LEN,
                    "datagram-class message outgrew the guaranteed datagram size"
                );
                // Fire and forget: under congestion QUIC drops the oldest datagram, which
                // for a pose stream is exactly the right thing to lose.
                link.conn.send_datagram(payload.clone().into()).is_ok()
            } else {
                let mut frame = Vec::with_capacity(payload.len() + 4);
                frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                frame.extend_from_slice(&payload);
                matches!(
                    tokio::time::timeout(WRITE_TIMEOUT, link.send.write_all(&frame)).await,
                    Ok(Ok(()))
                )
            };
            if !delivered {
                // Closing makes the link's reader loop exit, which is the one place a
                // departure is announced.
                link.conn.close(1u32.into(), b"send failed");
            }
        }
    }
}
