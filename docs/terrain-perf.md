# Terrain performance log

Every terrain phase is compared against the row before it. A phase that regresses p99 frame time or
chunk throughput needs a recorded reason here.

Tools:
- `cargo bench --bench terrain`: criterion micro-benchmarks, then p50/p99 over 128 distinct chunks per kind.
- `cargo run --release --example flight -- --seed 1 --secs 30 --out bench/flight-<phase>.txt`: scripted
  camera flight (settle, walk 20 m/s, sprint 80 m/s, high fly-over at y=900), headless. Reports are
  `key: value` lines, so `diff bench/flight-a.txt bench/flight-b.txt` works.
- `cargo run --release --example bench`: single-thread stage timings, vertex counts must not change in a refactor.

Flight throughput is wall-clock and varies run to run (the walk leg is limited by what the camera needs, not
by generation speed), so judge it by p99 frame time and by the sprint and high legs.

## Baseline: layered noise (before phase 0), seed 1

| bench | result |
|---|---|
| chunk_gen cold, plains / mountains / coast (criterion) | 32.4 / 41.0 / 25.3 ms |
| chunk_gen p50/p99 warm regions, plains | 0.129 / 30.4 ms |
| chunk_gen p50/p99 cold, plains / mountains / coast | 26.2/60.2, 27.1/60.3, 19.8/84.1 ms |
| terrain_sample height / biome / climate / cell 64 | 1.119 / 1.109 / 0.173 / 0.968 us |
| lod_node level 1 / 3 / 5 / 7 | 0.171 / 0.238 / 2.62 / 0.174 ms |
| example bench | 664454 chunk vertices, 352454 lod vertices |

Flight (`bench/flight-baseline.txt`): frame p99 3.0 / 5.2 / 5.7 / 4.7 ms for load / walk / sprint / high,
worst 84.9 ms in the high leg. Sprint and high generate about 13.4k chunks/s.

## Phase 0: `TerrainSource` trait

Chunk generation and LOD read terrain through `Arc<dyn TerrainSource>` only (`RegionStore::terrain()`).
Vertex counts unchanged (664454 / 352454). Criterion and percentiles within noise of the baseline
(height sample 1.124 us, cold plains chunk p50 26.3 ms, level 5 node 2.65 ms). Flight
(`bench/flight-phase0.txt`): frame p99 2.4 / 5.4 / 5.9 / 5.1 ms, high-leg throughput 13.4k chunks/s.
No regression. The one-off 77-85 ms worst frame in the high leg is in the baseline too.

## Phase 1: world map (not yet used by the game)

`worldmap::generate` makes the default 65,536 m world at 32 m/cell (2048 x 2048) in 92 ms on 16 cores.
Samples: bicubic height 47.5 ns, bilinear 16.9 ns, `WorldTerrain` height 71.9 ns, biome 72.2 ns,
water 82.7 ns, against 1.12 us for a layered-noise sample. The game still runs on the noise generator
until phase 5, so the flight numbers are unchanged.

## Phase 3: hydrology (not yet used by the game)

Priority-flood fill, breaching of small basins, D8 flow, accumulation and river carving.
Generating the default world rose from 92 ms to 2.4 s, almost all of it hydrology (a priority flood over
4.2M cells, up to six times while rims are cut). `WorldTerrain` height 112.9 ns, water 115.1 ns, against
71.9 ns before, because each lookup now searches 25 cells for river segments. Still well under the
1.12 us of a layered-noise sample. A coarse per-cell "river nearby" mask would remove the search; not
needed yet.

## Phase 2: erosion (not yet used by the game)

Generation pipeline: base terrain, hydrology (breaching only), stream-power valleys (6 passes),
droplet erosion in tiles, thermal erosion, hydrology again with rivers carved. The default world takes
3.3 s on 16 cores. The priority flood now queues only the sea's shore (open sea is never reached from
land), which took hydrology from 2.3 s to under 1 s. Droplets run in 128-cell tiles, four rounds apart,
so tiles run in parallel and a seed still gives the same map (tested).
Plain droplets alone left cell-sized speckle on slopes and no valleys at range scale, hence the
stream-power pass for valleys and fewer, longer, higher-inertia droplets for flank gullies.

## Phase 4: climate and biomes (not yet used by the game)

Warmth from latitude, lapse rate of 1/800 per block (the rate `snow_line` already uses), moisture from a
westerly sweep over each row (sea evaporates, rising ground rains out, 60% of rain is recycled inland),
blended with its rank among the land so every seed has dry and wet ground. A Whittaker table picks one
of ten biomes per cell. The default world takes 3.5 s. Seed 1 comes out at 18% desert, 14% steppe, 21%
savanna, 16% jungle, 15% forest, 7% tundra/taiga/glacier, with jungle on windward coasts and desert in
the lee of ranges.

## Phase 5a: the game runs on the world map

`World::new` loads or generates the map (cached under `worlds/`, or `RC_WORLD_DIR`), `WorldTerrain` is the
`TerrainSource`, and the layered-noise generator, its biome shaping and the `Fractal` helper are gone. The
terrain differs (new continents), so LOD vertex counts and spawn-area timings are no longer comparable
with earlier rows, only the per-sample and per-chunk numbers are.

| bench | before | now |
|---|---|---|
| terrain sample, height | 1.12 us | 0.256 us |
| chunk_gen cold p50, plains / mountains / coast | 26.2 / 27.1 / 19.8 ms | 13.5 / 13.4 / 13.4 ms |
| chunk_gen cold p99, plains / mountains / coast | 60.2 / 60.3 / 84.1 ms | 52.4 / 40.1 / 39.1 ms |
| lod_node level 1 / 3 / 5 / 7 | 0.171 / 0.238 / 2.62 / 0.174 ms | 0.226 / 0.827 / 1.27 / 0.182 ms |

Level 1 and 3 nodes are slower because the spawn area is now jungle and forest with far more trees to place
(tree placement dominates those levels), not because sampling got slower.
Flight (`bench/flight-phase5a.txt`): frame p99 2.5 / 5.4 / 6.1 / 5.0 ms against 2.4 / 5.4 / 5.9 / 5.1,
throughput unchanged (it is capped by the loader's per-frame budget, not by generation). Worst frame in the
high leg was 110 ms, against 77-85 ms in earlier runs; it is a single frame and varies run to run.

## Open question: map cell size, 16 m or 32 m

Seed 1, 65,536 m world, 16 cores (`world_map --cell N`):

| cell | cells | generation | peak memory |
|---|---|---|---|
| 32 m | 2048^2 (4.2M) | 2.7 s | 270 MB |
| 16 m | 4096^2 (16.8M) | 12.9 s | 856 MB |
| 8 m (extrapolated) | 8192^2 (67M) | about a minute | 3+ GB |

32 m stays the default: rivers are drawn as segments with their own width, so cell size does not limit them,
and chunk detail noise supplies everything finer than 64 m. 16 m is supported (`[world] map_cell = 16`) and
costs about 5x the time. The erosion constants are in cell units, so a finer map erodes differently, not
just more finely; it would need retuning, not only a config change.
