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

You land on a title screen: **Start a new world**, and under it every world being hosted
on this network, by the name of the machine hosting it. D-pad to choose, `A` to go. No
typing, which is the point — the players are on Steam Decks in gaming mode.

![the join menu, listing a world hosted on the LAN](docs/join-menu.png)

A friend who is *not* on this network needs the ticket. Pause (`Start` / `Esc`) →
**Share the join ticket**: it goes on the clipboard and into
`~/.local/share/blockgame/ticket.txt`, which is how you get it off a Deck over ssh.

![the pause menu, having just shared the join ticket](docs/share-ticket.png)

Then:

```sh
nix-shell --run 'cargo run --release -- join <TICKET>'
```

The LAN list is discovery and nothing more — a name and an address, shouted over UDP
broadcast and answered. What happens next is the same iroh connection a ticket makes.

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
| swing / fire   | hold left click  | hold `R2`            |
| place block    | right click      | `L2`                 |
| pick a thing   | arrows           | d-pad                |
| open the rig   | `C`              | `X`                  |
| walk the rig   | arrows, `WASD`   | d-pad                |
| make one       | `C`              | `X`                  |
| stand on it    | `Enter`          | `A`                  |
| leave the rig  | `Esc` / `R`      | `B` / `Start`        |
| get in / out   | `R`              | `B`                  |
| pause menu     | `Esc`            | `Start`              |
| menus          | arrows, `Enter`  | d-pad / stick, `A`   |

Quitting is a row on the pause menu, along with sharing the join ticket. It used to be a
`Select`+`Start` chord, which nobody who had not read the source could find.

In the car the left stick is the whole of it: forward and back is the throttle, left and
right is the steering. Everything else works from the seat.

You start in the air, flying, with nothing. Press `F` to drop into walking, then break
something — it is yours.

## Tools and guns

Breaking a block takes a moment, and what is in your hand is how long it takes. A bare
hand is half a second; a **hammer** is quicker and a **drill** is quicker still, both swung
at arm's length. The **handgun** and the **rifle** break blocks at a distance instead —
there is nothing else in the world to shoot at, and the rifle reaches furthest and scopes
in while you hold the trigger. A **nail** is a crafting part and does nothing on its own.

Each of those is three numbers on one row of the registry — how far, how fast, how much it
zooms — so the game has one code path through all of them, and a new tool is a new row.

## Making things

You start empty-handed. Breaking a block puts it in your pocket; placing one spends it.

**The constellation** is everything there is to hold. Your empty hand is in the middle of
it and every item hangs off it as a star, one press of the d-pad away or two — and those
presses *are* the thing's name: stone is down, a nail is right-up, the car is left-up. Press
a direction and the chart blooms out of your hand with the run of threads you walked lit up;
stop pressing and it folds back into the one thing you are carrying, hanging low on the
screen with its own code drawn under it as one little d-pad per press. Walk back the way you
came and you are empty-handed again, so putting a thing down needs no button either.

![the constellation open: fourteen stars around a hand, green rings on everything that could be built right now](docs/constellation.png)

There is not a word on it. How big a star is drawn says whether you own any, the bar of
notches beside it says how many, and a green ring says the pile would pay for one right now
— so "what can I build?" is a colour you scan for.

Press craft and **the rig** leans into whatever star you are standing on: the same items,
the same colours, the same rings, one neighbourhood at a time. One map at two zooms, the
same d-pad through both. `blockgame craft-film --scene chart` plays the whole loop to
itself — punch a code, lean in, build, come back out — and
[`docs/design/hotbar-constellation.gif`](docs/design/hotbar-constellation.gif) is what it
wrote.

![the crafting rig: a car above wood, nails and stone, wired together by beaded strings](docs/design/crafting-forge.gif)

Nothing in it is written. The thing you are making hangs in the middle, what it is made of
hangs below it, and what it goes into hangs above. Every recipe is a glowing string with
one bead on it per unit that recipe asks for — lit for the ones you have, dark for the ones
you still owe — so "eight nails, you have three" is five dark beads and not a sentence. The
bar of notches beside a thing is how many you own, and the ring around the cursor is green
when the button in your hand would make one right now.

The d-pad walks it as it walks the chart: left and right along a row, up and down between
rows. Craft makes one; `A` re-centres the rig on whatever you are pointing at, and whatever
*that* is made of unfolds under it.

One press makes one thing, deepest first, so holding craft on a car makes its eight nails
one at a time and then the car — the multi-step build, with the wait replaced by watching
it happen.

It is a **graph**, not a tree: a part two things need is one node with two strings leaving
it, which today's stone already is — the car needs stone, and so does every nail the car
needs.

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

The cushion is a block you build with, the parachute changes how you fall, and the car you
drive. The nail is spent in recipes and does nothing on its own — everyone else still sees
it in your hand.

## Falling

Landing hard hurts. A drop of four blocks is free — a jump never costs anything — and past
that it costs hearts in proportion to how fast you hit the ground, with a thirty-block
cliff taking a whole player. The bar is in the top-left corner.

Running out is not death. There is nothing to lose and nowhere to wake up: you are flat on
your back for three seconds, and then you get up where you landed with what those seconds
healed. Hearts come back on their own the whole time. Nothing else in the game hurts you.

Two things make a fall safe, and they are the two the sheets asked for:

A **cushion** is a block, so you put it where you are going to land. Whatever height you
fell from, hitting one costs nothing, and it throws you back up about a sixth of the way —
it is a trampoline as much as a safety mat. One boot on it is enough.

A **parachute** is held, not buckled on: punch its code and the canopy is open. You
come down at six blocks a second, slower than a hop lands at, and the stick carries you
further sideways than you fall — so a jump off a tower is a glide to somewhere else. Pick
it *before* you jump; a fall is over in a second or two.

Both are one row of the registry rather than a rule in the game loop. A block says how much
of a landing it gives back; an equippable says how fast you fall wearing it and what the
stick is worth on the way down.

## The car

<img src="docs/car.png" alt="a blocky blue car with the spaceman standing at the wheel" width="520">

Six wood, two stone and eight nails, and it is the fastest thing you own — comfortably
quicker than a sprint. Hold it — left, up — and press place to stand it in front of you;
press it again to put it back in your pocket. `B` gets you in, and `B` gets you out on the
driver's side.

Driving is the left stick and nothing else. Steering turns the car, which turns you, which
turns the camera — so the view follows the car without there being a second camera to keep
in step. Look up and down is still yours, and so is the trigger: a hill can be shot at
from the driver's seat.

It hovers over whatever is under its four wheels rather than simulating any. A step up to
a block high is a hill and it drives up it; anything higher is a wall and stops it dead.
Drive off a ledge and it falls. Drive to the edge of the world and it stops, because there
is no ground out there to be on yet.

Your friends see it move with you in it. A car rides the same pose stream your body does —
it is the same kind of fact, "where this player is right now" — which is also what bounds
them: one car each, never more of them in the world than there are people in it. And the
host clears the car off anybody's pose who never built one, because the item is the one
half of a car the host actually owns.

## Who you are

<img src="docs/spaceman.png" alt="a blocky white spaceman with teal trim, a dark visor and a rocket on his chest" width="280">

Everyone in the world is this spaceman, drawn from
[`design/spaceman-avatar.jpg`](design/spaceman-avatar.jpg). He is one table of boxes in
`src/avatar.rs`; `blockgame portrait` re-renders the picture above from it,
`blockgame portrait --holding hammer` shows him carrying something — what everyone else
sees when you swap what you are holding — and `blockgame portrait --car --out docs/car.png`
puts him at the wheel, which is how the seat above got checked.

## How it fits together

| file                | what it owns                                                    |
|---------------------|-----------------------------------------------------------------|
| `src/registry.rs`   | **every block and item in the game** — start here to add content |
| `src/inventory.rs`  | what a player has, and what crafting spends                      |
| `src/chart.rs`      | **the constellation** — where each item is, and its d-pad code   |
| `src/icons.rs`      | what a thing looks like off the ground, for both surfaces        |
| `src/hud.rs`        | the crosshair, and the one line the game ever says               |
| `src/forge.rs`      | **the crafting rig** — the recipe graph, drawn without words     |
| `src/film.rs`       | `craft-film`: either surface, driven by a script, one PNG a frame |
| `src/world.rs`      | chunk storage and seed-deterministic terrain                     |
| `src/mesh.rs`       | turning voxels into geometry                                     |
| `src/player.rs`     | movement, collision, and what a landing costs                    |
| `src/vehicle.rs`    | the car: driving, and getting in and out of one                  |
| `src/raycast.rs`    | what you're looking at                                           |
| `src/avatar.rs`     | **the models** — the spaceman and the car, tables of boxes       |
| `src/portrait.rs`   | `blockgame portrait` — renders those models to `docs/`           |
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
class and its recipe, and an entry in `Item::ALL`. It takes the next free exit on the
constellation — so it arrives with a code of its own and moves nobody else's — is craftable,
and travels over the network with no other file touched.

A **new block** is the same plus its own half: a `Block` variant, an arm in `Block::def`
giving it a colour, and an item of class `Block` that places it. Colour lives in the mesh,
so there are no assets to make, and a variant with no arm fails to compile rather than
crashing the first time somebody names it. A row that says nothing about bouncing is plain
ground; say `bounce` and it is soft to land on.

What a tool does is three numbers on its row, and what a wearable does is two more on its
own — how fast you fall in it, and what the stick is worth while you do. What a vehicle
does is `src/vehicle.rs`, and a second one would be that file's constants and a second
table in `src/avatar.rs`.

## Develop

```sh
nix-shell --run 'cargo fmt --check'
nix-shell --run 'cargo clippy --all-targets -- --deny warnings'
nix-shell --run 'cargo test -- --test-threads=2'
```

## License

MIT — see [LICENSE](LICENSE).
