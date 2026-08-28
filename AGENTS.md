# Working on blockgame

The README is the game. This file is only what a change has to obey that the README has no
reason to say.

## Nobody has to read anything

**In-game text is never load-bearing, and there should be as little of it as the design
allows.** Everything the game asks of a player has to be sayable in shape, colour, motion
and position: the player this is built for cannot read a word, and a surface that needs a
sentence to be understood is a bug in that surface, not a missing sentence. The crafting
rig is the standing example — a whole recipe graph with nothing written on it, where
"eight nails, you have three" is five dark beads.

The words that remain are a convenience for the people who can read them and never the
only copy of anything. So when a line has to change, ask first whether it can go: the fix
for a hint that has become a lie is usually less text, not better text.

## Build and check

`shell.nix` pins all of it — toolchain, bevy's linux inputs, and an X server for a box
with no display.

```sh
nix-shell --run 'cargo fmt --check'
nix-shell --run 'cargo clippy --all-targets -- --deny warnings'
nix-shell --run 'cargo test -- --test-threads=2'
```

Green on all three before anything is pushed. CI runs exactly those, and `main` is what
the handhelds pull on launch, so a red `main` is a broken game in somebody's hands.

## One seam for content

Content is added in `src/registry.rs` — two tables, and a new block or craftable is a row
in them, read by the pad, the rig and the wire alike. A holdable item also takes its slot
in `src/code.rs`'s ROSETTE and its picture in `src/glyph.rs`; both are exhaustively
checked, so leaving one out fails the build rather than hiding the item. Anything past
those three files edited to make content appear has missed the seam. (README, "Adding
things".)

## Show it running

Anything that moves is reviewed by watching it, not by describing it. `blockgame
craft-film` writes one PNG a frame while driving the rig with the same input struct the
pad fills, through the same systems the game runs — assemble those into a GIF and that is
the review artifact. A mock-up shows what somebody hoped the code does.
