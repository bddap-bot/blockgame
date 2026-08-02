//! `blockgame` — a Minecraft-like voxel game built to be extended.
//!
//! Run it with no arguments to play your own world. That world is already a server with
//! nobody connected: the ticket it prints is all a friend needs to join it, and the same
//! code runs either way.

mod avatar;
mod game;
mod input;
mod mesh;
mod net;
mod player;
mod portrait;
mod raycast;
mod registry;
mod world;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "blockgame",
    version,
    about = "A voxel world you can build in, together"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// World seed. Omit for a new world every time. Ignored when joining — the host's
    /// world is the one you get.
    #[arg(long, global = true)]
    seed: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// Join a friend's world with the ticket they printed.
    Join {
        #[arg(value_name = "TICKET")]
        ticket: net::PlayerId,
    },
    /// Render the player model to a PNG. Regenerates `docs/spaceman.png`.
    Portrait {
        #[arg(long, default_value = "docs/spaceman.png")]
        out: std::path::PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let join = match cli.command {
        Some(Command::Portrait { out }) => return portrait::run(out),
        Some(Command::Join { ticket }) => Some(ticket),
        None => None,
    };
    let seed = cli.seed.unwrap_or_else(fresh_seed);
    game::run(net::boot(join, seed)?)
}

/// A different world each launch. Only the host ever picks one — everyone else is told
/// which world they are in, so this never has to agree across machines.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x5EED)
}
