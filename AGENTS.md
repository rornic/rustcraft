# Ochre

A voxel engine on Bevy 0.20. Rust workspace, no binary: the game is the `sandbox` example.

## Run

```
cargo run --release --example sandbox      # the game; reads assets/settings.toml
cargo run --release --example worldview -- [seed] [port]   # browser world viewer
cargo run --release --example bench        # single-thread stage timings
cargo test --release --workspace
cargo clippy --workspace --all-targets
```

Always `--release`: debug generation is unusably slow. Run from the repo root, since assets and shaders load by relative path. Rust 1.97.1+ (Bevy 0.20), plus `libudev-dev` and `pkg-config` on Linux. Wayland is deliberately off (X11 only).

Env vars: `OCHRE_SEED` (fix the world seed), `OCHRE_WORLD_DIR` (saved maps, default `worlds/`, gitignored), `OCHRE_SPAWN`, `OCHRE_FLYTHROUGH`, `OCHRE_STATS`, `OCHRE_TIME`, `OCHRE_LOOK`.

## Crates

Dependencies point down only. `ochre-engine` (root `src/`) re-exports all four and owns `Settings` (`assets/settings.toml`).

```
ochre-base      blocks, biomes, noise/math, TerrainSource trait, WORLD_HEIGHT, WATER_LEVEL
ochre-terrain   chunks, generation, greedy meshing, octree LODs, materials, loaders (depends on base only)
ochre-world     world map (erosion, hydrology, climate), sky, clouds, weather, ambience (base, terrain)
ochre-gameplay  player, collision, birds, critters, debug overlay, flythrough (base, terrain, world)
```

`ochre-terrain` never sees the world map: it generates from a `TerrainSource`, and `ochre-world::load_world` supplies one. Terrain's tests borrow real maps through `ochre-world`'s `test-support` feature, so only `ochre-base` types may cross back.

## Assembling a game

`examples/sandbox.rs` is the reference. Plugins: `TerrainPlugin` (loading, headless-safe), `TerrainRenderPlugin` (materials, curvature, mesh buffers, needs a GPU), `SkyPlugin` (clock, sun, moon, atmosphere, clouds, weather), `PlayerPlugin`, plus `AmbiencePlugin`, `CritterPlugin`, `BirdPlugin`, `DebugOverlayPlugin`, `FlythroughPlugin`. A Startup system then calls `ochre_world::load_world`, `spawn_player` and `insert_terrain_resources` (fed a `ShaderInputs`; the default is blank, which is what the headless `flight` and `lod_sim` use).

## Where things are

- Terrain generation: `ochre-terrain/src/generate/` (columns, regions, trees, light), meshing in `generate/generator/`.
- LODs: `ochre-terrain/src/lod/`. Chunk streaming: `chunk_loader.rs`. Both run on `TerrainPlugin`.
- World map pipeline: `ochre-world/src/worldmap/` (erosion, hydrology, water, climate, then `source.rs` as the `TerrainSource`).
- Lakes, rivers and the sea, and the rule that keeps them from leaking: `docs/water.md`. Read it before changing water.
- Shaders: `assets/shaders/`. Block textures: `assets/textures/blocks.png`, built by the `block-textures` skill.
- Examples double as tools: `worldprobe` (CLI probe of a spot), `world_map` (PNG), `water_audit`, `seams`, `lod_sim` (headless loading), `flow_sim` (headless frame times while water flows), `flight` (scripted flight report).

## Conventions

- Verify output is unchanged with `cargo run --release --example bench`: vertex counts must match exactly before and after a refactor.
- Terrain is deterministic per seed. Don't introduce thread-order or hash-order dependence in generation (`generate/generator/determinism.rs` guards it).
- Perf work follows the `profile` skill and is logged in `docs/terrain-perf.md`. A phase that regresses p99 frame time needs a recorded reason.
- Debug the world with the `worldview` skill, not ad hoc probe scripts.
- Large test modules (`generator/tests.rs`, `lod/tests.rs`) run real generation, so expect slow `cargo test`.
- Commit messages and PR titles are lowercase conventional commits. PRs target `main`. See `CONTRIBUTING.md`.
- When driving the GUI, screenshot passively. Don't grab focus or input on a shared desktop.
