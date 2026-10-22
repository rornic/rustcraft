---
name: profile
description: Measure and profile Ochre's terrain pipeline (regions, chunk generation, chunk meshing, LOD meshing per level) and its main-thread loading, find hotspots with perf, and A/B a change. Use when asked to speed something up, check a change for a performance regression, or find where time goes.
---

# Profile

Measure before changing anything, and A/B every change against the binary before it. Single runs on a shared machine vary by ±15%, so one run proves nothing.

## Setup

- Rust 1.97.1 or newer (Bevy 0.20 needs it): `rustup toolchain install 1.97.1 --profile minimal -c clippy -c rustfmt && rustup override set 1.97.1`.
- Bevy needs `libudev-dev` and `pkg-config` to build on Linux.
- perf: `apt-get install -y linux-tools-generic`, then run `/usr/lib/linux-tools/*/perf` directly, as the `perf` wrapper refuses a kernel it doesn't know. Use `-e cpu-clock`, since containers often lack hardware counters.
- A cold release build compiles all of Bevy (10-20 minutes). A second target directory starts cold too, so copy `target/release` into it first.

## Time each stage

```
cargo run --release --example bench              # every stage
cargo run --release --example bench -- 7 lod     # seed 7, LOD meshing only
```

`examples/bench` builds terrain round spawn on one thread and prints regions, chunk generation, chunk meshing (best of 3, with the vertex count) and LOD meshing per level (best of 3 after a warming pass). Vertex counts must match exactly before and after, the cheap check that output is unchanged.

## Find the hotspot

Add line tables to the terrain and world crates alone, so nothing else recompiles.

```
cargo build --release --example bench --config 'profile.release.package.ochre-terrain.debug="line-tables-only"' --config 'profile.release.package.ochre-world.debug="line-tables-only"'
P=$(ls /usr/lib/linux-tools/*/perf | head -1)
$P record -e cpu-clock -F 3000 -o /tmp/perf.data ./target/release/examples/bench 1 lod
$P report -i /tmp/perf.data --no-children --sort symbol | head -40
```

- Hottest lines in one function: `$P annotate -i /tmp/perf.data --stdio -l <mangled symbol> | grep -E '^\s+[0-9.]+ +[a-z_]+\.rs:' | sort -rn | head`. Only Ochre's own functions have line tables, so a dependency's (`noise`'s Perlin) prints nothing.
- To see who calls the hot code, record with `--call-graph dwarf,16384`, dump with `$P script -i /tmp/perf.data --no-demangle > /tmp/perf.txt`, then `python3 .claude/skills/profile/attribute.py /tmp/perf.txt <frames>` naming the frames to follow, such as `tree_in_cell crown_boxes SkyLight mesh_opaque_greedy perlin`. Paths come out muddled where inlining merges functions.
- When neither is clear, add temporary `Instant` timers round the phases (a static `[AtomicU64; N]` the bench prints), and remove them before committing.

## A/B a change

Build the bench and copy `target/release/examples/bench` to `/tmp/ab/old`. Apply the change, build and copy to `/tmp/ab/new`, then alternate them.

```
for i in 1 2 3 4; do for b in old new; do echo -n "$b "; /tmp/ab/$b | grep -E 'mesh chunks|lod total'; done; done
```

Keep a change only if it wins in every pair. Revert one within noise, as it's code that buys nothing. To compare against an older commit, add a worktree there, copy `examples/bench.rs` in if it predates it, and build into its own target directory.

## Main thread and loading

`examples/lod_sim` runs the loading systems headless and logs frames over 100ms, plus holes and LOD node counts every 2 seconds. Fix the seed to compare two builds, and write to a file, as a killed pipe loses its output.

```
OCHRE_SEED=7 timeout -s INT 30 ./target/release/examples/lod_sim 80 10 > /tmp/sim.txt 2>&1
```

The arguments are flying speed in blocks a second and seconds spent flying. A full run lasts about 90 seconds (20 to load, the flight, then 60 stopped), so the 30-second timeout keeps just the load and the start of the flight. Node counts at the same timestamps show loading speed. Holes on a fixed seed must match between builds. Some persistent full-detail holes are pre-existing.

## Where time went last time

- Reading blocks through `ChunkData::get_block_at` scans the chunk's runs, so hot loops must read a dense grid (`ChunkNeighbourhood`, `LodBlocks`, `GridView`).
- Distant trees cost one full noise sample per candidate (`NoiseGenerator::sample` runs every fractal). Rule candidates out cheaply first, as `TreeGround::rules_out` does.
- Region building (65,536 noise samples each) and Perlin noise are the largest costs left. Changing the noise changes the terrain.
