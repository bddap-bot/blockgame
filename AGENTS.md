# Working on blockgame

Edit by subtraction: resolve a problem by deleting code; a tactical patch over a symptom is not accepted. One implementation per thing, never two alive.

Delete code comments; keep only a why the code cannot show.

## Communicate without reading

Express everything a player needs through shape, color, motion and position. Text
is optional convenience, never the sole cue. For example, eight required nails
with three available can be five dark beads. When a hint becomes wrong, first ask
whether it can be removed.

## Checks

Use [shell.nix](shell.nix):

```sh
nix-shell --run 'cargo fmt --check'
nix-shell --run 'cargo clippy --all-targets -- --deny warnings'
nix-shell --run 'cargo test -- --test-threads=2'
```

CI runs these checks; keep all three green before pushing code. Handhelds pull
`main` on launch.

## Content and motion

Follow [README.md](README.md#adding-things) for adding content. Keep additions in
`src/registry.rs`, with holdable slots in `src/code.rs` and pictures in
`src/glyph.rs`; exhaustive checks catch missing slots and glyphs. Changes elsewhere
to make content appear should prompt a design check.

Review movement from the running game: `blockgame craft-film` drives the real rig
systems and writes PNG frames. Assemble those into a GIF; a mock-up does not
verify behavior.

## Boundaries

Keep this project independent. Reference other projects only as declared, versioned
dependencies, exposing names and versions rather than internals. Give shared services
neutral project-owned names. Exclude deployment-specific paths, addresses, service
or queue names, credentials, camera frames and private renders. Before landing,
inspect the diff for undeclared project references and deployment details.
