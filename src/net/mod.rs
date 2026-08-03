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

pub mod lan;
pub mod wire;

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::{Connection, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};

use crate::registry::Block;
use crate::world::{BLOCKS_PER_CHUNK, BlockPos};
#[cfg(test)]
use crate::world::{CHUNK_SIZE, ChunkPos, WORLD_HEIGHT};
pub use wire::{Msg, PlayerId, Pose};

/// Bumped whenever the wire format changes incompatibly — mismatched builds then fail to
/// connect instead of desyncing silently. `2` was inventories: [`Msg::Pose`] grew a held
/// item, and crafting added two messages a build on `1` cannot decode. `3` is cars, which
/// grew the pose again. `4` is presence: a build on `3` announces departures the other
/// side has no message for, and never says who is here at all.
pub const ALPN: &[u8] = b"bddap-bot/blockgame/4";

/// How long the whole join handshake may take: the host's answer *and* the world it then
/// sends. Not per message — a clock the host restarts by sending something bounds nothing,
/// and every part it sends is memory the joiner keeps. Generous because it now has the
/// transfer to cover too, and a well-built world over a relay is a slow minute, not a
/// hostile one. It bites only on a host that answered and then went quiet: an unreachable
/// one fails the dial long before this.
const JOIN_TIMEOUT: Duration = Duration::from_secs(60);
/// The most world a joining peer will take in before it decides the host is lying.
///
/// `Welcome`'s `parts` is the host's *claim*, and nothing on the wire relates it to
/// anything the joiner can check — so a host that keeps sending parts grows the joiner's
/// buffer until it dies. Sixty-four megabytes of edits is orders of magnitude past any
/// played-in world (a chunk holds at most [`BLOCKS_PER_CHUNK`] edits and play touches a
/// handful of blocks per chunk), so this bounds the lie and never the game.
const MAX_WORLD_EDITS: usize = 64 * 1024 * 1024 / size_of::<(BlockPos, Block)>();
/// How long an accepted connection has to open its stream before its slot is taken back.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// A peer that can't absorb a reliable frame in this long is dropped. Each link has its
/// own writer task, so this bounds only that link's own life — a wedged peer never delays
/// anybody else's traffic, whatever it does with its window.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes a link may have queued before it is considered wedged and dropped. Sends never
/// block the game, so a peer that stops reading has to be bounded somewhere; here, where
/// the memory is. Bytes rather than frames because that is what actually costs the host,
/// and because a joining peer is handed one frame per edited chunk at once.
const SEND_QUEUE_BYTES: usize = 32 * 1024 * 1024;
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
    /// A player's first link arrived. On the host this means somebody joined; on a peer
    /// it is the host, which it already knows it dialled.
    Joined(PlayerId),
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
    /// Answers "who is hosting?" on the LAN. A host has one unless the discovery port
    /// was already taken; a peer never does — it has nothing to serve anyone.
    pub beacon: Option<lan::Beacon>,
}

/// How long leaving may take. [`Endpoint::close`] bounds itself at about three seconds on
/// a bad path; this bounds the wait on the way out of the game, where the peer being said
/// goodbye to may itself already be gone.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// A live network session. Dropping it says goodbye and tears the endpoint down.
pub struct Session {
    /// The tokio runtime the connection tasks live on. Held so they keep running; the
    /// game never blocks on it — sends and receives are non-blocking channel ops.
    rt: tokio::runtime::Runtime,
    router: Router,
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

    /// Where this endpoint can currently be reached — read afresh each time, because
    /// iroh learns addresses after the endpoint is up and a stale list is a game nobody
    /// on the LAN can dial.
    pub fn addr(&self) -> EndpointAddr {
        self.router.endpoint().addr()
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

/// Quitting says so rather than going quiet.
///
/// Closing the endpoint is what makes the other end's reader exit, and that is the one
/// place a departure is announced. Without it a player who quits is only noticed when
/// their connection times out — half a minute of their avatar standing in everybody
/// else's world, which is the ghost this protocol is built against, on a fuse.
impl Drop for Session {
    fn drop(&mut self) {
        let router = &self.router;
        // The timeout is built inside `block_on`: a timer has to be made from within the
        // runtime it will run on.
        let _ = self
            .rt
            .block_on(async { tokio::time::timeout(CLOSE_TIMEOUT, router.shutdown()).await });
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
    outbound: Outbox,
}

/// One link's send queue: frames on their way to its writer task, and the bytes they hold.
///
/// The count is what bounds the queue — see [`SEND_QUEUE_BYTES`] — and it is shared with
/// the writer, which subtracts each frame as it goes out.
#[derive(Clone)]
struct Outbox {
    frames: mpsc::UnboundedSender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
}

impl Outbox {
    /// Queues a frame, or reports that this link is too far behind to keep.
    fn push(&self, frame: Vec<u8>) -> bool {
        let n = frame.len();
        if self.queued.fetch_add(n, Ordering::AcqRel) + n > SEND_QUEUE_BYTES {
            self.queued.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        if self.frames.send(frame).is_err() {
            self.queued.fetch_sub(n, Ordering::AcqRel);
            return false;
        }
        true
    }
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
    /// Adds a link and reports whether it was that player's first — the mirror of
    /// [`Links::remove`]'s last, and for the same reason: a player arrives with their
    /// first link and is gone with their last, whatever happens in between.
    fn insert(&mut self, link: Link<C>) -> (LinkId, bool) {
        let first = !self.by_id.values().any(|l| l.peer == link.peer);
        let id = self.next;
        self.next += 1;
        self.by_id.insert(id, link);
        (id, first)
    }

    /// Who is speaking on this link, or `None` if it is already gone.
    fn sender(&self, id: LinkId) -> Option<PlayerId> {
        self.by_id.get(&id).map(|l| l.peer)
    }

    /// Removes a link and reports whether that was the departing player's last one.
    fn remove(&mut self, id: LinkId) -> Option<(PlayerId, bool)> {
        let link = self.by_id.remove(&id)?;
        let still_here = self.by_id.values().any(|l| l.peer == link.peer);
        Some((link.peer, !still_here))
    }

    /// Everything a send needs, cloned out so the caller can let go of the lock before it
    /// touches the network.
    fn targets(&self, to: Target) -> Vec<(C, Outbox)> {
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

/// Generic in the connection for the same reason [`Links`] is: the ordering below is the
/// part worth testing, and it needs no network. `C` is always [`Connection`] in the game.
struct Bus<C = Connection> {
    links: Mutex<Links<C>>,
    /// Inbound connections currently being served, counted separately from `links`
    /// because the limit has to bite before a connection is far enough along to be one.
    inbound: AtomicUsize,
    inbox: mpsc::UnboundedSender<Event>,
}

impl<C: Clone> Bus<C> {
    /// Hands the game a message from a link, and reports whether that link should keep
    /// reading. The link is what names the sender, so no caller can attribute a message to
    /// a player who did not send it.
    ///
    /// Finding the sender is also the liveness check, and that is what makes a departure
    /// final. Both this and [`Bus::end_link`] hold the links lock across their send, so a
    /// message either goes out while its link is live — strictly before that link's `Left` —
    /// or not at all. Without it, a datagram decoded in the moment a link died lands *after*
    /// the departure, the game spawns the player it has just forgotten, and nothing ever
    /// removes it again.
    async fn deliver(&self, link: LinkId, msg: Msg) -> bool {
        let links = self.links.lock().await;
        let Some(peer) = links.sender(link) else {
            return false;
        };
        self.inbox.send(Event::Message(peer, msg)).is_ok()
    }

    /// Starts a link, announcing the arrival if it was that player's first.
    ///
    /// Holds the links lock across the send for the same reason [`Bus::deliver`] does, the
    /// other way about: a message is delivered only while its link is live, so an arrival
    /// sent under the lock lands strictly *before* anything that link ever says. The game
    /// is what that buys — a pose can then only ever move a player it has already been
    /// told about.
    async fn begin_link(&self, link: Link<C>) -> LinkId {
        let peer = link.peer;
        let mut links = self.links.lock().await;
        let (id, first) = links.insert(link);
        if first {
            let _ = self.inbox.send(Event::Joined(peer));
        }
        id
    }

    /// Ends a link, announcing the departure if it was that player's last.
    async fn end_link(&self, link: LinkId) {
        let mut links = self.links.lock().await;
        if let Some((peer, last)) = links.remove(link)
            && last
        {
            let _ = self.inbox.send(Event::Left(peer));
        }
    }
}

/// Starts the network. `join` is `None` to host, or where to find a host to join —
/// a bare [`PlayerId`] from a pasted ticket, or one with addresses attached from
/// [`lan`]. Both are the same dial: LAN discovery fills in addresses, it does not
/// change how a peer connects.
///
/// `name` is what this machine's game is called on the LAN, and is only used when
/// hosting.
///
/// `discovery` is the port a host answers join questions on: [`lan::PORT`] for a real
/// game, since a broadcast question has nowhere to look the number up. A test passes 0
/// and reads back [`lan::Beacon::port`], so it never has to take the port off a game
/// somebody is playing.
///
/// Joining blocks until the host's `Welcome` arrives, so the game is built with a
/// complete world in hand and no "world not ready yet" state exists to handle.
pub fn boot(join: Option<EndpointAddr>, seed: u64, name: &str, discovery: u16) -> Result<Boot> {
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

        let (seed, edits) = match join.clone() {
            None => (seed, Vec::new()),
            Some(addr) => {
                let host = addr.id;
                let conn = endpoint
                    .connect(addr, ALPN)
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

    // A host that cannot claim the discovery port is still a perfectly good host — its
    // ticket works — so this says so and carries on rather than refusing to start. Said
    // out loud, because a game silently missing from every join menu on the network is
    // the kind of thing somebody spends an evening on.
    let beacon = match &join {
        Some(_) => None,
        None => match lan::Beacon::open(name.to_string(), discovery) {
            Ok(beacon) => Some(beacon),
            Err(e) => {
                eprintln!("this game will not appear on the LAN join menu: {e:#}");
                None
            }
        },
    };

    Ok(Boot {
        role: match join {
            None => Role::Host,
            Some(addr) => Role::Peer { host: addr.id },
        },
        seed,
        edits,
        beacon,
        session: Session {
            rt,
            router,
            me,
            inbox: inbox_rx,
            outbox: out_tx,
        },
    })
}

/// Collects the world: a `Welcome` saying how many parts to expect, then that many
/// `WorldPart`s.
///
/// Everything the host says here is a claim, so the cost of believing it is bounded three
/// ways: one deadline for the whole handshake, one chunk's worth of edits per part, and
/// [`MAX_WORLD_EDITS`] over all of them.
async fn await_welcome(
    inbox: &mut mpsc::UnboundedReceiver<Event>,
    host: PlayerId,
) -> Result<(u64, Vec<(BlockPos, Block)>)> {
    let deadline = tokio::time::Instant::now() + JOIN_TIMEOUT;
    let mut world: Option<(u64, u32)> = None;
    let mut edits = Vec::new();
    loop {
        let next = tokio::time::timeout_at(deadline, inbox.recv())
            .await
            .map_err(|_| anyhow!("the handshake did not finish within {JOIN_TIMEOUT:?}"))?;
        // Only the endpoint we dialled gets to say what world this is. Anyone else's
        // `Welcome` would hand a stranger the seed and every block of the world.
        let msg = match next {
            Some(Event::Message(from, msg)) if from == host => msg,
            // Waiting out the full timeout for a host that has already hung up tells the
            // player "no answer" when the truth is "the door shut".
            Some(Event::Left(id)) if id == host => {
                return Err(anyhow!("host disconnected during the handshake"));
            }
            // Anything else this early is pre-handshake chatter; the world isn't built
            // yet, so there is nothing that could consume it.
            Some(_) => continue,
            None => return Err(anyhow!("host disconnected during the handshake")),
        };
        match msg {
            Msg::Welcome { seed, parts } => {
                if world.is_some() {
                    anyhow::bail!("host sent a second Welcome");
                }
                world = Some((seed, parts));
            }
            Msg::WorldPart { edits: part } => {
                let Some((_, remaining)) = world.as_mut() else {
                    anyhow::bail!("host sent world data before saying what world it is");
                };
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or_else(|| anyhow!("host sent more world than it promised"))?;
                // A part is *one chunk's* overlay, which holds at most one edit per block.
                // Both halves matter: the count is what bounds a part against the frame
                // limit, and one chunk is what bounds what the edits then cost to keep —
                // scattered across chunks, a part's worth of edits is a per-chunk map each,
                // an order of magnitude more memory than the edits themselves.
                if let Some(((first, _), rest)) = part.split_first() {
                    let chunk = first.chunk();
                    if part.len() > BLOCKS_PER_CHUNK {
                        anyhow::bail!("a world part holds {} edits, past a chunk", part.len());
                    }
                    if rest.iter().any(|(pos, _)| pos.chunk() != chunk) {
                        anyhow::bail!("a world part spans more than one chunk");
                    }
                }
                edits.extend(part);
                // `parts` is a u32 with nothing behind it, so the promise bounds this only
                // once it has been kept. This is what bounds it while it is being made.
                if edits.len() > MAX_WORLD_EDITS {
                    anyhow::bail!("host is sending more world than {MAX_WORLD_EDITS} edits");
                }
            }
            _ => continue,
        }
        if let Some((seed, 0)) = world {
            return Ok((seed, edits));
        }
    }
}

#[derive(Clone, Debug)]
struct Proto {
    bus: Arc<Bus>,
}

impl<C> std::fmt::Debug for Bus<C> {
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
        // Bounded: the inbound slot is already claimed by the time we are here, so a
        // stranger who connects and then opens no stream would otherwise hold one of the
        // host's [`MAX_INBOUND_LINKS`] for as long as it cares to keep the connection alive.
        tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| anyhow!("peer {peer} opened no stream within {HANDSHAKE_TIMEOUT:?}"))?
            .context("accepting the stream")?
    };

    // QUIC doesn't surface an opened stream to the acceptor until the dialer writes, so
    // the dialer's first act is to say hello. That is also what asks for the world.
    if dialed {
        send.write_all(&wire::encode_frame(&Msg::Hello)?)
            .await
            .context("saying hello")?;
    }

    let (frames_tx, frames) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let outbound = Outbox {
        frames: frames_tx,
        queued: queued.clone(),
    };
    let id = bus
        .begin_link(Link {
            peer,
            conn: conn.clone(),
            outbound,
        })
        .await;

    let writer = tokio::spawn(write_link(send, frames, queued, conn.clone()));
    let datagrams = tokio::spawn(read_datagrams(conn.clone(), id, peer, bus.clone()));
    let result = read_stream(recv, id, peer, bus.clone()).await;

    datagrams.abort();
    writer.abort();
    bus.end_link(id).await;
    // The datagram task held a Connection clone, so an explicit close is what actually
    // ends the link rather than waiting out the idle timeout.
    conn.close(0u32.into(), b"link closed");
    result
}

/// One link's outbound half. Every reliable write happens here and nowhere else, so a
/// peer that stops reading stalls only its own queue.
async fn write_link(
    mut send: SendStream,
    mut frames: mpsc::UnboundedReceiver<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    conn: Connection,
) {
    while let Some(frame) = frames.recv().await {
        let result = tokio::time::timeout(WRITE_TIMEOUT, send.write_all(&frame)).await;
        queued.fetch_sub(frame.len(), Ordering::AcqRel);
        match result {
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
    id: LinkId,
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
        if !bus.deliver(id, msg).await {
            return Ok(()); // the game is gone, or this link already is
        }
    }
}

async fn read_datagrams(conn: Connection, id: LinkId, peer: PlayerId, bus: Arc<Bus>) {
    while let Ok(bytes) = conn.read_datagram().await {
        match wire::decode(&bytes) {
            Ok(msg) => {
                if !bus.deliver(id, msg).await {
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
                outbound.push(bytes.clone())
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

    /// Two games on one machine doing what the front room will do: one hosts, the other
    /// finds it on the network and joins it, with no ticket typed anywhere.
    ///
    /// End to end on purpose — real endpoints, real broadcast, real QUIC — because every
    /// part of this is a thing that works in isolation and can still fail to meet: a
    /// beacon nobody hears, an address that is not reachable from the side that got it,
    /// a handshake that needs a relay the front room does not have.
    ///
    /// The host's welcome is answered here rather than by [`crate::game`], because what
    /// is under test is the joining, not the world.
    ///
    /// Discovery runs on a port the kernel picks, not [`lan::PORT`]: on a machine where
    /// the game is actually being played that port is taken, and a test that fights the
    /// family for it fails for a reason that has nothing to do with joining.
    #[test]
    fn a_game_on_this_network_is_found_and_joined() {
        const SEED: u64 = 4242;
        let host = boot(None, SEED, "test-host", 0).expect("hosting");
        let host_id = host.session.me();
        let beacon = host.beacon.expect("a host takes the discovery port");
        let discovery = beacon.port();
        let mut session = host.session;

        let serving = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let stop = serving.clone();
        let host_side = std::thread::spawn(move || {
            while stop.load(Ordering::Relaxed) {
                beacon.answer(&session.addr());
                for event in session.drain() {
                    if let Event::Message(peer, Msg::Hello) = event {
                        session.send(
                            Target::One(peer),
                            Msg::Welcome {
                                seed: SEED,
                                parts: 0,
                            },
                        );
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });

        let mut search = lan::Search::open(discovery).expect("a socket to ask with");
        let looking_since = std::time::Instant::now();
        let game = loop {
            search.ask();
            std::thread::sleep(Duration::from_millis(100));
            search.collect();
            if let Some(game) = search.games().into_iter().find(|g| g.addr.id == host_id) {
                break game;
            }
            assert!(
                looking_since.elapsed() < Duration::from_secs(10),
                "the host never turned up on the join menu"
            );
        };
        assert_eq!(game.name, "test-host");

        let joined =
            boot(Some(game.addr), 0, "test-peer", discovery).expect("joining what was found");
        assert_eq!(joined.seed, SEED, "the joiner did not get the host's world");
        assert_eq!(joined.role, Role::Peer { host: host_id });
        assert!(
            joined.beacon.is_none(),
            "a peer has no world of its own to announce"
        );

        serving.store(false, Ordering::Relaxed);
        host_side.join().expect("the host side");
    }

    /// A link whose "connection" is just a tag, which is all the bookkeeping touches.
    fn link(peer: PlayerId, conn: u8) -> Link<u8> {
        let (frames, _writer) = mpsc::unbounded_channel();
        Link {
            peer,
            conn,
            outbound: Outbox {
                frames,
                queued: Arc::new(AtomicUsize::new(0)),
            },
        }
    }

    /// A peer that stops reading must cost the host a bounded amount of memory and then
    /// its link, never an unbounded queue.
    #[test]
    fn a_link_that_never_drains_is_bounded() {
        let (frames, _writer) = mpsc::unbounded_channel();
        let outbox = Outbox {
            frames,
            queued: Arc::new(AtomicUsize::new(0)),
        };
        let frame = vec![0u8; 1024 * 1024];
        let mut queued = 0;
        while outbox.push(frame.clone()) {
            queued += frame.len();
            assert!(queued <= SEND_QUEUE_BYTES, "queue outgrew its bound");
        }
        assert!(queued > SEND_QUEUE_BYTES / 2, "gave up far too early");
    }

    /// The reconnect race: a peer's second connection arrives before its first has torn
    /// down. Keyed by player, the new link would be evicted by the old one's exit and a
    /// still-connected player announced as gone.
    #[test]
    fn a_second_connection_does_not_evict_the_first() {
        let mut links = Links::<u8>::default();
        let peer = id(1);
        let (first, arrived) = links.insert(link(peer, 1));
        let (second, again) = links.insert(link(peer, 2));
        assert!(arrived, "the first link is the player arriving");
        assert!(!again, "a second link is not a second player");
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

    /// A bus with no network under it: everything the ordering tests touch is bookkeeping.
    fn bus() -> (Arc<Bus<u8>>, mpsc::UnboundedReceiver<Event>) {
        let (inbox, rx) = mpsc::unbounded_channel();
        let bus = Bus {
            links: Mutex::new(Links::default()),
            inbound: AtomicUsize::new(0),
            inbox,
        };
        (Arc::new(bus), rx)
    }

    fn drained(inbox: &mut mpsc::UnboundedReceiver<Event>) -> Vec<Event> {
        let mut seen = Vec::new();
        while let Ok(ev) = inbox.try_recv() {
            seen.push(ev);
        }
        seen
    }

    /// The ghost. A datagram decoded in the moment its link died, delivered after the
    /// departure, spawns the player the game has just forgotten — and nothing ever removes
    /// that avatar again: it stands there for the rest of the session, inflating the player
    /// count and, on the host, holding a spot where no block may be placed.
    #[tokio::test]
    async fn a_message_cannot_follow_its_own_departure() {
        let (bus, mut inbox) = bus();
        let peer = id(1);
        let link = bus.begin_link(link(peer, 1)).await;

        assert!(bus.deliver(link, Msg::Hello).await, "a live link");
        bus.end_link(link).await;
        assert!(!bus.deliver(link, Msg::Hello).await, "a dead link spoke");

        let seen = drained(&mut inbox);
        assert!(
            matches!(
                seen[..],
                [Event::Joined(_), Event::Message(..), Event::Left(_)]
            ),
            "{seen:?}"
        );
    }

    /// The other end of the same guarantee, and what lets a pose only ever *move* somebody:
    /// nothing a player says can arrive before the game has been told they are here.
    #[tokio::test]
    async fn nobody_speaks_before_they_have_arrived() {
        let (bus, mut inbox) = bus();
        let peer = id(1);
        let first = bus.begin_link(link(peer, 1)).await;
        let second = bus.begin_link(link(peer, 2)).await;

        assert!(bus.deliver(first, Msg::Hello).await);
        assert!(bus.deliver(second, Msg::Hello).await);

        let seen = drained(&mut inbox);
        assert!(
            matches!(
                seen[..],
                [Event::Joined(_), Event::Message(..), Event::Message(..)]
            ),
            "a second link is a second arrival: {seen:?}"
        );
    }

    /// A player is gone when their *last* link is, so a message on the surviving one still
    /// belongs to somebody who is here.
    #[tokio::test]
    async fn one_link_ending_does_not_silence_the_other() {
        let (bus, mut inbox) = bus();
        let peer = id(1);
        let first = bus.begin_link(link(peer, 1)).await;
        let second = bus.begin_link(link(peer, 2)).await;

        bus.end_link(first).await;
        assert!(!bus.deliver(first, Msg::Hello).await, "the dead one");
        assert!(bus.deliver(second, Msg::Hello).await, "the live one");
        bus.end_link(second).await;

        let seen = drained(&mut inbox);
        assert!(
            matches!(
                seen[..],
                [Event::Joined(_), Event::Message(..), Event::Left(_)]
            ),
            "{seen:?}"
        );
    }

    #[test]
    fn a_send_to_one_player_reaches_only_their_links() {
        let mut links = Links::<u8>::default();
        let (a, b) = (id(1), id(2));
        links.insert(link(a, 1));
        links.insert(link(b, 2));
        let only_b: Vec<u8> = links
            .targets(Target::One(b))
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(only_b, vec![2]);
        assert_eq!(links.targets(Target::All).len(), 2);
    }

    /// One chunk's worth of edits — every block of the chunk at the origin, which is the
    /// biggest honest part there is.
    fn a_full_chunk() -> Vec<(BlockPos, Block)> {
        (0..BLOCKS_PER_CHUNK as i32)
            .map(|i| {
                let pos = BlockPos::new(
                    i % CHUNK_SIZE,
                    (i / CHUNK_SIZE) % WORLD_HEIGHT,
                    i / (CHUNK_SIZE * WORLD_HEIGHT),
                );
                assert_eq!(pos.chunk(), ChunkPos::new(0, 0), "{pos:?} left the chunk");
                (pos, Block::Stone)
            })
            .collect()
    }

    fn say(tx: &mpsc::UnboundedSender<Event>, host: PlayerId, msg: Msg) {
        tx.send(Event::Message(host, msg))
            .expect("the joiner is up");
    }

    /// What the joiner gave up with. A failure here means it did *not* give up, and
    /// printing the world it swallowed instead would bury that under a megabyte of blocks.
    async fn refused(rx: &mut mpsc::UnboundedReceiver<Event>, host: PlayerId) -> String {
        match await_welcome(rx, host).await {
            Ok((_, edits)) => panic!("the joiner took all {} edits of it", edits.len()),
            Err(e) => format!("{e:#}"),
        }
    }

    /// The joiner's buffer is memory a host grows from the other side of the world, and
    /// `parts` is a `u32` it promises rather than anything the joiner can check. So it stops
    /// believing the promise long before keeping it costs the game.
    #[tokio::test]
    async fn a_joiner_stops_taking_more_world_than_it_will_hold() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let host = id(9);
        let chunk = a_full_chunk();
        say(
            &tx,
            host,
            Msg::Welcome {
                seed: 1,
                parts: u32::MAX,
            },
        );
        for _ in 0..MAX_WORLD_EDITS.div_ceil(BLOCKS_PER_CHUNK) + 1 {
            say(
                &tx,
                host,
                Msg::WorldPart {
                    edits: chunk.clone(),
                },
            );
        }
        drop(tx);

        let why = refused(&mut rx, host).await;
        assert!(
            why.contains("is sending more world"),
            "gave up for the wrong reason: {why}"
        );
    }

    /// A part is one chunk, and a chunk holds one edit per block. The frame limit alone
    /// would let a host put two and a half chunks' worth in each part it promised.
    #[tokio::test]
    async fn a_world_part_may_not_outgrow_a_chunk() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let host = id(9);
        let mut fat = a_full_chunk();
        fat.push((BlockPos::new(0, 1, 0), Block::Stone));
        say(&tx, host, Msg::Welcome { seed: 1, parts: 1 });
        say(&tx, host, Msg::WorldPart { edits: fat });

        let why = refused(&mut rx, host).await;
        assert!(why.contains("past a chunk"), "{why}");
    }

    /// A part scattered across chunks is inside every other bound and still the expensive
    /// lie: each edit becomes a per-chunk map of its own, an order of magnitude more memory
    /// than the edits themselves. An honest part is one chunk's overlay.
    #[tokio::test]
    async fn a_world_part_may_not_span_chunks() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let host = id(9);
        let scattered = (0..BLOCKS_PER_CHUNK as i32)
            .map(|i| (BlockPos::new(i * CHUNK_SIZE, 0, 0), Block::Stone))
            .collect();
        say(&tx, host, Msg::Welcome { seed: 1, parts: 1 });
        say(&tx, host, Msg::WorldPart { edits: scattered });

        let why = refused(&mut rx, host).await;
        assert!(why.contains("more than one chunk"), "{why}");
    }

    /// A clock the host restarts by sending something bounds nothing: a host dribbling one
    /// part just inside every timeout would hold a joiner on the loading screen forever.
    #[tokio::test(start_paused = true)]
    async fn a_dribbling_host_cannot_restart_the_join_clock() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let host = id(9);
        tokio::spawn(async move {
            // Not `say`: the joiner is expected to give up first, and the sends after that
            // are the point — a host still talking to somebody who has stopped listening.
            let speak = |msg| {
                let _ = tx.send(Event::Message(host, msg));
            };
            speak(Msg::Welcome { seed: 1, parts: 3 });
            for _ in 0..3 {
                tokio::time::sleep(JOIN_TIMEOUT * 3 / 4).await;
                speak(Msg::WorldPart { edits: Vec::new() });
            }
        });

        let started = tokio::time::Instant::now();
        refused(&mut rx, host).await;
        assert!(
            started.elapsed() <= JOIN_TIMEOUT,
            "the clock restarted: {:?}",
            started.elapsed()
        );
    }
}
