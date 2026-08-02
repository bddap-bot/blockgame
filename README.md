# blockgame

A Minecraft-like voxel world in Rust, built to be extended. Bevy for rendering, iroh for
multiplayer.

Dig up blocks, make things out of them, build with a friend. No mobs and no save file yet.
What gets added on top is `design/`'s business.

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
| hotbar         | `1`–`9`, `Q`/`E` | d-pad                |
| craft          | `C`              | `X`                  |
| quit           | `Esc`            | `Select` + `Start`   |

You start in the air, flying, with nothing. Press `F` to drop into walking, then break
something — it is yours.

## Making things

You start empty-handed. Breaking a block puts it in your pocket; placing one spends it.

The hotbar along the bottom is everything there is to hold, and it is also the crafting
menu — there is no second screen and no grid to arrange. Walk the cursor onto a thing and
the line above tells you what it is made of; press craft and, if you have the parts, you
have one.

|             | made of                    |
|-------------|----------------------------|
| nail        | 1 stone                    |
| cushion     | 4 leaves + 1 wood          |
| hammer      | 2 wood + 1 nail            |
| handgun     | 1 wood + 2 nails           |
| rifle       | 2 wood + 3 nails           |
| drill       | 1 wood + 2 stone + 2 nails |
| parachute   | 6 leaves + 2 nails         |
| car         | 6 wood + 2 stone + 8 nails |

The cushion is a block, so you can build with it. The rest you can make and carry —
everyone else sees what is in your hand — but none of them *does* anything yet. That is
the next job, one item at a time.

![the hotbar after digging up ten stone and making three nails](docs/hotbar.png)

## Who you are

<img src="docs/spaceman.png" alt="a blocky white spaceman with teal trim, a dark visor and a rocket on his chest" width="280">

Everyone in the world is this spaceman, drawn from
[`design/spaceman-avatar.jpg`](design/spaceman-avatar.jpg). He is one table of boxes in
`src/avatar.rs`; `blockgame portrait` re-renders the picture above from it, and
`blockgame portrait --holding hammer` shows him carrying something — what everyone else
sees when you swap what you are holding.

## How it fits together

| file                | what it owns                                                    |
|---------------------|-----------------------------------------------------------------|
| `src/registry.rs`   | **every block and item in the game** — start here to add content |
| `src/inventory.rs`  | what a player has, and what crafting spends                      |
| `src/hud.rs`        | crosshair, status line, and the hotbar you craft from            |
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

Everything lives in `src/registry.rs`, in two tables.

A **new thing to craft** is one row: an `Item` variant, an arm in `Item::def` naming its
class and its recipe, and an entry in `Item::ALL`. It appears in the hotbar, is craftable,
and travels over the network with no other file touched.

A **new block** is the same plus its own half: a `Block` variant, an arm in `Block::def`
giving it a colour, and an item of class `Block` that places it. Colour lives in the mesh,
so there are no assets to make, and a variant with no arm fails to compile rather than
crashing the first time somebody names it.

What a tool, a vehicle or something wearable *does* is still nothing. Adding that is the
work; having somewhere to put it is not.

## Develop

```sh
nix-shell --run 'cargo fmt --check'
nix-shell --run 'cargo clippy --all-targets -- --deny warnings'
nix-shell --run 'cargo test -- --test-threads=2'
```

## License

MIT — see [LICENSE](LICENSE).
