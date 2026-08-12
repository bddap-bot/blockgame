//! `blockgame` — a Minecraft-like voxel game built to be extended.
//!
//! Run it with no arguments and pick from the title screen: start your own world, or
//! join one somebody on this network is already hosting. Your world is already a server
//! with nobody connected — the ticket it prints, and the one the pause menu shares, is
//! all a friend somewhere else needs — and the same code runs either way.

mod avatar;
mod belt;
mod film;
mod forge;
mod game;
mod hud;
mod input;
mod inventory;
mod menu;
mod mesh;
mod net;
mod pause;
mod player;
mod portrait;
mod raycast;
mod registry;
mod rig;
mod ticket;
mod title;
mod vehicle;
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
    /// Join a friend's world with the ticket they printed. The join menu covers anybody
    /// on the same network; this is for a friend who is not.
    Join {
        #[arg(value_name = "TICKET")]
        ticket: net::PlayerId,
    },
    /// Render a model to a PNG. Regenerates `docs/spaceman.png` and `docs/car.png`.
    Portrait {
        #[arg(long, default_value = "docs/spaceman.png")]
        out: std::path::PathBuf,
        /// Put something in his hand — an item name, as the hotbar spells it. Empty-handed
        /// by default, which is what `docs/spaceman.png` shows.
        #[arg(long, value_name = "ITEM")]
        holding: Option<String>,
        /// Draw the car instead, with him at the wheel.
        #[arg(long)]
        car: bool,
    },
    /// Drive the crafting rig through a scripted session and write one PNG per frame.
    /// The only honest review of a thing whose whole language is motion.
    CraftFilm {
        #[arg(long, default_value = "craft-frames")]
        out: std::path::PathBuf,
        #[arg(long, default_value_t = 480)]
        frames: u32,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let join = match cli.command {
        Some(Command::Portrait { out, holding, car }) => {
            // A name nothing answers to is refused here rather than quietly rendering an
            // empty hand and leaving you to wonder which of the two is broken.
            let holding = holding
                .map(|name| {
                    registry::Item::named(&name).ok_or_else(|| {
                        anyhow::anyhow!("no item is called {name:?} — see the hotbar for the names")
                    })
                })
                .transpose()?;
            let subject = if car {
                portrait::Subject::Car
            } else {
                portrait::Subject::Spaceman
            };
            return portrait::run(out, holding, subject);
        }
        Some(Command::CraftFilm { out, frames }) => return film::run(out, frames),
        Some(Command::Join { ticket }) => Some(ticket),
        None => None,
    };
    // `--seed` before the subcommand still parses, and a joiner is told which world it is
    // in, so honouring it is impossible and ignoring it is a lie.
    if join.is_some() && cli.seed.is_some() {
        anyhow::bail!("--seed picks a world to host; a joiner gets the host's world");
    }
    game::run(title::Start {
        seed: cli.seed.unwrap_or_else(fresh_seed),
        name: net::lan::this_machine(),
        // A ticket carries no addresses, so this is the dial that leans on iroh's
        // address lookup — which is the right one for a friend who is not on this
        // network, and the whole reason the flag stays.
        join: join.map(iroh::EndpointAddr::new),
    })
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
