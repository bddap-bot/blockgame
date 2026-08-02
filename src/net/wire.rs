//! The wire protocol.
//!
//! One message enum for both directions. The host is authoritative: a peer *asks* with
//! [`Msg::Edit`] and the host *tells* with the same variant — same shape, different
//! authority, so there is exactly one edit message in the codebase rather than a
//! request/response pair that can drift apart.

use crate::registry::Block;
use crate::world::BlockPos;
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
    /// Host → peer: everything needed to reconstruct the world. Terrain is a pure
    /// function of the seed, so only the edits have to travel.
    Welcome {
        seed: u64,
        edits: Vec<(BlockPos, Block)>,
    },
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

/// Ceiling on a single stream frame. `Welcome` is the only message that grows with play,
/// and at ~16 bytes per edit this allows a million-block build before it bites.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Datagram payloads must fit QUIC's guaranteed minimum datagram size, which is a little
/// over a kilobyte before path-MTU discovery grows it. Every datagram-class message is
/// fixed-size, so `datagram_messages_fit` proves this statically rather than the encoder
/// having to fall back to the reliable path at runtime.
pub const MAX_DATAGRAM_LEN: usize = 1024;

pub fn encode(msg: &Msg) -> Result<Vec<u8>> {
    let bytes = bincode::serialize(msg)?;
    if bytes.len() > MAX_FRAME_LEN {
        bail!("message too large to send: {} bytes", bytes.len());
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Msg> {
    Ok(bincode::deserialize(bytes)?)
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
            Msg::Welcome {
                seed: 7,
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
        assert!(
            !Msg::Welcome {
                seed: 0,
                edits: vec![]
            }
            .via_datagram()
        );
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert!(decode(&[0xff; 8]).is_err());
    }

    /// A peer on a newer build must not be able to name a block this build doesn't have.
    #[test]
    fn unknown_block_ids_are_rejected() {
        let mut bytes = encode(&Msg::Edit {
            pos: BlockPos::new(0, 0, 0),
            block: Block::Stone,
        })
        .unwrap();
        *bytes.last_mut().unwrap() = 200;
        assert!(decode(&bytes).is_err());
    }
}
