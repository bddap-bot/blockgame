# blockgame

A Minecraft-like voxel world in Rust, built to be extended. Bevy for rendering, iroh for
multiplayer.

These are the bones. There is no inventory, no crafting, no mobs, and no save file — just
a world you can walk around, dig into, build in, and share with a friend. What gets added
on top is `design/`'s business.

![a voxel world of hills, forest and beach](docs/screenshot.png)

## Play

```sh
nix-shell --run 'cargo run --release'
```

It prints a ticket. Anyone can join with it:

```sh
nix-shell --run 'cargo run --release -- join <TICKET>'
```

There is no separate server. Running `blockgame` on its own *is* hosting a world with
nobody else in it, so "single player" and "multiplayer" are the same program on the same
code path.

`--seed 12345` picks a specific world. Without it you get a new one every launch. A
joining player ignores their own seed and gets the host's world.

## Controls

|                | keyboard + mouse | gamepad (Steam Deck) |
|----------------|------------------|----------------------|
| move           | `WASD`           | left stick           |
| look           | mouse            | right stick          |
| jump / rise    | `Space`          | `A`                  |
| sprint         | `Shift`          | `R1`                 |
| sink (flying)  | `Ctrl`           | `L1`                 |
| fly on/off     | `F`              | `Y`                  |
| break block    | left click       | `R2`                 |
| place block    | right click      | `L2`                 |
| pick a block   | `1`–`6`          | d-pad left/right     |
| quit           | `Esc`            | —                    |

You start in the air, flying. Press `F` to drop into walking.

## Who you are

<img src="docs/spaceman.png" alt="a blocky white spaceman with teal trim, a dark visor and a rocket on his chest" width="280">

Everyone in the world is this spaceman, drawn from
[`design/spaceman-avatar.jpg`](design/spaceman-avatar.jpg). He is one table of boxes in
`src/avatar.rs`; `blockgame portrait` re-renders the picture above from it.

## How it fits together

| file                | what it owns                                                    |
|---------------------|-----------------------------------------------------------------|
| `src/registry.rs`   | **every block and item in the game** — start here to add content |
| `src/world.rs`      | chunk storage and seed-deterministic terrain                     |
| `src/mesh.rs`       | turning voxels into geometry                                     |
| `src/player.rs`     | movement and collision                                           |
| `src/raycast.rs`    | what you're looking at                                           |
| `src/avatar.rs`     | **the player model** — one table of boxes                        |
| `src/portrait.rs`   | `blockgame portrait` — renders that model to `docs/spaceman.png` |
| `src/input.rs`      | keyboard/mouse/gamepad → one `Intent`                            |
| `src/net/`          | the iroh message bus and wire format                             |
| `src/game.rs`       | the Bevy app that wires the above together                       |

Two design rules hold the rest up:

**The host is authoritative.** It owns the world. A joining player sends *intents* ("I'd
like to break this block") and only applies what the host sends back, so there is one
world and it can't drift.

**Terrain is a pure function of the seed.** Joining ships a `u64` and the list of blocks
anyone has changed since — never chunk data. Break something, and the only thing that
travels is which block, and what it became.

## Adding things

New block: one `Block` variant, one `BLOCKS` row, one `ITEMS` row. Nothing else — colour
lives in the mesh, so there are no assets to make.

New non-block item (a tool, a vehicle, something wearable): one `Item` variant and one
`ITEMS` row with the right `ItemKind`. The kinds are already there for what's been asked
for; the behaviour behind them isn't yet.

## Develop

```sh
nix-shell --run 'cargo fmt --check'
nix-shell --run 'cargo clippy --all-targets -- --deny warnings'
nix-shell --run 'cargo test -- --test-threads=2'
```

## License

MIT — see [LICENSE](LICENSE).
