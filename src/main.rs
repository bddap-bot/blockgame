//! `blockgame` — a Minecraft-like voxel game built to be extended.
//!
//! Run it with no arguments to play your own world. That world is already a server with
//! nobody connected: the ticket it prints is all a friend needs to join it, and the same
//! code runs either way.

mod avatar;
mod game;
mod hud;
mod input;
mod inventory;
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
    /// World seed. Omit for a new world every time.
    // Deliberately not `global`: a joiner is told which world it is in, so a seed there is
    // a request nothing can honour — better refused by the parser than silently dropped.
    #[arg(long)]
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
    // `--seed` before the subcommand still parses, and a joiner is told which world it is
    // in, so honouring it is impossible and ignoring it is a lie.
    if join.is_some() && cli.seed.is_some() {
        anyhow::bail!("--seed picks a world to host; a joiner gets the host's world");
    }
    let seed = cli.seed.unwrap_or_else(fresh_seed);
    game::run(net::boot(join, seed)?)
}

/// A different world each launch. Only the host ever picks one — everyone else is told
/// which world they are in, so this never has to agree across machines.
///
/// A clock before 1970 is not a world seed to fall back from: silently substituting a
/// fixed one would hand everybody on that machine the same world for ever, looking like a
/// worldgen bug rather than a broken clock.
fn fresh_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system clock reads before 1970")
        .as_nanos() as u64
}
