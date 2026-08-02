//! The wire protocol.
//!
//! One message enum for both directions. The host is authoritative: a peer *asks* with
//! [`Msg::Edit`] and the host *tells* with the same variant — same shape, different
//! authority, so there is exactly one edit message in the codebase rather than a
//! request/response pair that can drift apart.

use crate::registry::Block;
use crate::world::{BLOCKS_PER_CHUNK, BlockPos};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// A player's network identity, which is also their iroh address. The host's is the
/// join ticket.
pub type PlayerId = iroh::EndpointId;

/// Where a player is and which way they are facing. Sent continuously, over unreliable
/// datagrams — a lost pose is superseded by the next one a frame later.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Pose {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Msg {
    /// Peer → host, first thing on the stream. Opens the bidirectional stream (QUIC
    /// doesn't surface one to the acceptor until the dialer writes) and asks for the
    /// world.
    Hello,
    /// Host → peer, the head of the world: the seed, and how many [`Msg::WorldPart`]s
    /// follow. Terrain is a pure function of the seed, so only the edits have to travel.
    Welcome { seed: u64, parts: u32 },
    /// Host → peer: one chunk's edits, sent once per edited chunk after a [`Msg::Welcome`].
    ///
    /// The world is split per chunk because a chunk holds at most one edit per block,
    /// which bounds a part — see [`MAX_FRAME_LEN`]. Sending the whole world in one message
    /// has no such bound: past about a million edits it cannot be encoded at all, and the
    /// world becomes permanently unjoinable.
    WorldPart { edits: Vec<(BlockPos, Block)> },
    /// Peer → host it is an *intent*; host → peer it is *authoritative*. Reliable: a
    /// dropped edit would desync the world permanently.
    Edit { pos: BlockPos, block: Block },
    /// Where a player is. Everyone sends their own; the host relays each one it receives
    /// to everybody, so peers see each other without a second message type. A receiver
    /// ignores its own id.
    Pose { id: PlayerId, pose: Pose },
    /// Host → peer: somebody disconnected, drop their avatar.
    PeerLeft { id: PlayerId },
}

impl Msg {
    /// Pose traffic rides unreliable datagrams; everything else needs the ordered,
    /// reliable stream. The one place that decision is made.
    pub fn via_datagram(&self) -> bool {
        matches!(self, Msg::Pose { .. })
    }
}

/// Bytes one edit costs on the wire: three `i32` of position and the block's `u32` tag.
const EDIT_LEN: usize = 16;

/// Ceiling on a single stream frame.
///
/// [`Msg::WorldPart`] is the only message that grows with play, and it carries one chunk,
/// which holds at most [`BLOCKS_PER_CHUNK`] edits — so the ceiling is a fact about the
/// protocol rather than a hope about how much anyone builds. The assertion below is what
/// keeps it one: a bigger chunk or a wider block tag fails the build instead of quietly
/// making worlds unjoinable.
pub const MAX_FRAME_LEN: usize = 1024 * 1024;
const _: () = assert!(BLOCKS_PER_CHUNK * EDIT_LEN + 64 <= MAX_FRAME_LEN);

/// Datagram payloads must fit QUIC's guaranteed minimum datagram size, which is a little
/// over a kilobyte before path-MTU discovery grows it. Every datagram-class message is
/// fixed-size, so `datagram_messages_fit` proves this statically rather than the encoder
/// having to fall back to the reliable path at runtime.
pub const MAX_DATAGRAM_LEN: usize = 1024;

/// Bytes of length prefix in front of every stream frame.
pub const LEN_PREFIX: usize = 4;

/// The codec, in one place so the reader and the writer cannot drift.
///
/// Fixint little-endian is the format on the wire. The two properties that are *not*
/// `bincode::serialize`'s defaults are the point of naming it here: a limit, so a decode
/// cannot be talked into allocating more than a frame may hold, and reject-trailing, so a
/// frame padded out to the maximum is an error rather than a valid message with megabytes
/// of free ballast attached.
fn codec() -> impl bincode::Options {
    use bincode::Options;
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_FRAME_LEN as u64)
}

pub fn encode(msg: &Msg) -> Result<Vec<u8>> {
    use bincode::Options;
    Ok(codec().serialize(msg)?)
}

pub fn decode(bytes: &[u8]) -> Result<Msg> {
    use bincode::Options;
    Ok(codec().deserialize(bytes)?)
}

/// A whole stream frame: length prefix then payload. The one place framing is written —
/// [`frame_len`] is the one place it is read.
pub fn encode_frame(msg: &Msg) -> Result<Vec<u8>> {
    let payload = encode(msg)?;
    let mut frame = Vec::with_capacity(payload.len() + LEN_PREFIX);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Payload length a frame header claims, refusing anything past [`MAX_FRAME_LEN`].
///
/// The claim is a peer's word about bytes it has not sent yet, so the reader must not turn
/// it into an allocation — see `net::read_stream`, which grows its buffer as bytes arrive.
pub fn frame_len(header: [u8; LEN_PREFIX]) -> Result<usize> {
    let len = u32::from_le_bytes(header) as usize;
    if len > MAX_FRAME_LEN {
        bail!("oversized frame: {len} bytes");
    }
    Ok(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real player id. Not every 32-byte string is one — a public key has to be a
    /// valid curve point — so derive it from a secret instead of inventing bytes.
    fn some_id() -> PlayerId {
        iroh::SecretKey::from_bytes(&[7u8; 32]).public()
    }

    fn pose() -> Pose {
        Pose {
            pos: [1.5, 2.5, -3.5],
            yaw: 0.25,
            pitch: -0.5,
        }
    }

    #[test]
    fn round_trips() {
        let id = some_id();
        let msgs = [
            Msg::Hello,
            Msg::Welcome { seed: 7, parts: 3 },
            Msg::WorldPart {
                edits: vec![(BlockPos::new(1, 2, 3), Block::Stone)],
            },
            Msg::Edit {
                pos: BlockPos::new(-4, 70, 9),
                block: Block::Air,
            },
            Msg::Pose { id, pose: pose() },
            Msg::PeerLeft { id },
        ];
        for m in msgs {
            let back = decode(&encode(&m).unwrap()).unwrap();
            assert_eq!(format!("{m:?}"), format!("{back:?}"));
        }
    }

    /// The guarantee that lets the sender pick a transport from the message alone, with no
    /// runtime size check and no silent fallback to the reliable path.
    #[test]
    fn datagram_messages_fit() {
        let id = some_id();
        let m = Msg::Pose { id, pose: pose() };
        assert!(m.via_datagram());
        let n = encode(&m).unwrap().len();
        assert!(n <= MAX_DATAGRAM_LEN, "{m:?} encodes to {n} bytes");
    }

    #[test]
    fn only_pose_traffic_is_unreliable() {
        assert!(!Msg::Hello.via_datagram());
        assert!(
            !Msg::Edit {
                pos: BlockPos::new(0, 0, 0),
                block: Block::Air
            }
            .via_datagram()
        );
        assert!(!Msg::Welcome { seed: 0, parts: 0 }.via_datagram());
        assert!(!Msg::WorldPart { edits: vec![] }.via_datagram());
    }

    /// The bound the whole join path rests on: the biggest part a host can ever build —
    /// every block of one chunk edited — still fits a frame. A world that outgrows the
    /// frame cannot be sent, and shows up as an unexplained join timeout.
    #[test]
    fn the_largest_world_part_fits_a_frame() {
        let edits: Vec<(BlockPos, Block)> = (0..BLOCKS_PER_CHUNK)
            .map(|i| {
                let i = i as i32;
                (BlockPos::new(i % 16, i / 256, (i / 16) % 16), Block::Stone)
            })
            .collect();
        let n = encode(&Msg::WorldPart { edits }).unwrap().len();
        assert!(n <= MAX_FRAME_LEN, "a full chunk encodes to {n} bytes");
        assert!(
            n / BLOCKS_PER_CHUNK <= EDIT_LEN,
            "an edit costs more than EDIT_LEN says"
        );
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert!(decode(&[0xff; 8]).is_err());
    }

    /// A peer on a newer build must not be able to name a block this build doesn't have.
    ///
    /// The block rides last, as a little-endian u32 variant index, so the *first* of those
    /// four bytes is the id — overwriting the last one instead only ever tests a wildly
    /// out-of-range tag, which any decoder rejects for free.
    #[test]
    fn unknown_block_ids_are_rejected() {
        let mut bytes = encode(&Msg::Edit {
            pos: BlockPos::new(0, 0, 0),
            block: Block::Stone,
        })
        .unwrap();
        let id = bytes.len() - 4;
        assert_eq!(
            bytes[id..],
            [Block::Stone as u8, 0, 0, 0],
            "block tag layout"
        );
        // One past the last block this build knows: the off-by-one a newer peer would
        // send. Blocks are append-only, so the last variant declared is the highest id.
        bytes[id] = Block::Bedrock as u8 + 1;
        assert!(decode(&bytes).is_err());
    }

    /// A frame padded out with junk must not decode. Otherwise a one-byte message can be
    /// sent as a maximum-size frame, and the allocation it costs the receiver is never
    /// even punished with an error.
    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = encode(&Msg::Hello).unwrap();
        assert!(decode(&bytes).is_ok());
        bytes.extend_from_slice(&[0u8; 64]);
        assert!(decode(&bytes).is_err(), "padding decoded as a valid Hello");
    }

    #[test]
    fn a_frame_carries_its_own_length() {
        let frame = encode_frame(&Msg::Hello).unwrap();
        let header: [u8; LEN_PREFIX] = frame[..LEN_PREFIX].try_into().unwrap();
        let len = frame_len(header).unwrap();
        assert_eq!(len, frame.len() - LEN_PREFIX);
        assert!(!decode(&frame[LEN_PREFIX..]).unwrap().via_datagram());
    }

    /// The header is a peer's unverified claim, and the reader sizes its buffer from it.
    #[test]
    fn an_oversized_frame_header_is_refused() {
        assert!(frame_len((MAX_FRAME_LEN as u32).to_le_bytes()).is_ok());
        assert!(frame_len((MAX_FRAME_LEN as u32 + 1).to_le_bytes()).is_err());
        assert!(frame_len(u32::MAX.to_le_bytes()).is_err());
    }
}
