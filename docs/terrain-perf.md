# Terrain performance log

Every terrain phase is compared against the row before it. A phase that regresses p99 frame time or
chunk throughput needs a recorded reason here.

Tools:
- `cargo bench --bench terrain`: criterion micro-benchmarks, then p50/p99 over 128 distinct chunks per kind.
- `cargo run --release --example flight -- --seed 1 --secs 30 --out target/flight-<phase>.txt`: scripted
  camera flight (settle, walk 20 m/s, sprint 80 m/s, high fly-over at y=900), headless. Reports are
  `key: value` lines, so `diff target/flight-a.txt target/flight-b.txt` works.
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

Flight: frame p99 3.0 / 5.2 / 5.7 / 4.7 ms for load / walk / sprint / high,
worst 84.9 ms in the high leg. Sprint and high generate about 13.4k chunks/s.

## Phase 0: `TerrainSource` trait

Chunk generation and LOD read terrain through `Arc<dyn TerrainSource>` only (`RegionStore::terrain()`).
Vertex counts unchanged (664454 / 352454). Criterion and percentiles within noise of the baseline
(height sample 1.124 us, cold plains chunk p50 26.3 ms, level 5 node 2.65 ms). Flight: frame p99 2.4 / 5.4 / 5.9 / 5.1 ms, high-leg throughput 13.4k chunks/s.
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

`ochre_world::load_world` loads or generates the map (cached under `worlds/`, or `OCHRE_WORLD_DIR`), `WorldTerrain` is the
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
Flight: frame p99 2.5 / 5.4 / 6.1 / 5.0 ms against 2.4 / 5.4 / 5.9 / 5.1,
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

## Phase 5: chunk detail, water, strata, cliffs (done)

Full chunk pipeline: bicubic ground plus roughness-scaled detail, river channels and lakes, water filled to
each column's own surface, soil/stone/compacted-stone strata, 3D-noise cliffs cut before the water fills.
A cheap "water nearby" mask skips river and lake lookups for the columns that have none.

| bench | baseline | now |
|---|---|---|
| terrain sample, height | 1.12 us | 0.242 us |
| chunk_gen cold p50, plains / mountains / coast | 26.2 / 27.1 / 19.8 ms | 13.2 / 11.1 / 12.0 ms |
| chunk_gen cold p99, plains / mountains / coast | 60.2 / 60.3 / 84.1 ms | 61.3 / 33.1 / 37.0 ms |

Cold plains p99 is level with the baseline (61.3 against 60.2 ms): it is the first region in an area plus
columns, and it varies by about 10 ms between runs of the same build. The other five rows improved.
Flight (taken before the water mask, so the mask only helps): frame p99
2.6 / 5.3 / 6.2 / 5.2 ms against baseline 3.0 / 5.2 / 5.7 / 4.7. Sprint is 0.5 ms over the baseline's p99, a
run-to-run difference on a headless harness (the same build moved by 0.4 ms between two runs), and
throughput is identical, as the loader's per-frame budget caps it. The high leg's worst frame was 73 ms
against 77-85 ms in the baseline runs.
Determinism: `generator::determinism` pins a hash of chunks built from a hand-made map, portable across
machines.

## Phase 6: gameplay pass, plus fixes from flying the game

Passes (graded valleys joining walkable lowlands across ranges), river crossings at 2-3 km, spawn regions,
resource sites, caves and hotspots are generated and validated on world creation; the default world takes
3.0 s (`worldmap_generate`). Smoothing, minimum biome size, riparian belts (big rivers and lakes only) and
drying of narrow streams in dry ground were added after flying the game.

Flight: frame p99 3.2 / 5.1 / 7.1 / 6.3 ms against baseline 3.0 / 5.2 / 5.7 / 4.7.
Sprint and high-altitude p99 are about 1.5 ms over the baseline. Not proven, but the likely reason is that the
flight now crosses forest and jungle where the baseline crossed open land, so the main thread integrates more
mesh per frame; the same build moved 0.4-0.5 ms between runs, which does not cover it. Treat it as a regression
to chase with a render-side profile, not as settled. Chunk generation is faster than the baseline (cold p50
11.1 / 11.0 ms against 26.2 / 27.1, plains p99 43.9 against 60.2).

## After flying it, round three

Cliffs and terrain: mountain is now a biome (high or steep ground, grey on the map) with terraces in patches
(55-120 block steps, strata tilted differently in each patch), cliff carving starts at gradient 1.0 (half of
mountain columns are over it; it started at 2.6, which almost none were), and some stretches of coast rise
60+ m within 190 m of the shore (13-16% of coast points) so there are sea cliffs. Dunes are gentler and
desert below 90 m has no cliffs. Streams join rivers (two rivers sampled at a confluence), rivers are smooth
curves, and the bed is cut down into the land rather than levelled. The climate is wetter (desert 10-12%
of land, from 23-25%), lakes and narrow streams dry up in dry ground, only big water greens its banks.
Foothills is gone: it needed the old noise generator's ruggedness.
`GENERATOR_VERSION` is 5: a map cached by an earlier version is regenerated, not reused.

## Round four

Terraces stack (they follow height alone, with a slowly drifting step size and phase, so rising ground climbs
through them and never falls back; an earlier version added a tilt that sawtoothed on gentle ground). The same
function runs at every LOD cell size. Rivers are never raised by their bed (a test pins it) and the tallest
ground is stretched so peaks reach 986-988 of the world's 1024. `GENERATOR_VERSION` is 7.
Known gap: the LOD seam test only samples ground under 640 blocks; in high ranges rays leak at seams between
levels 1 and 2. Probably the harness not loading every node below the surface (to confirm and fix later).

## Round five: peaks

The height easing curve squashed every summit to the same level, so tops were flat. It is replaced by sharpening
(each high cell pushed 1.1x its height over the ground within ~600 blocks, which raises peaks and deepens the
valleys between them) and a straight rescale so the tallest ground stands at 990. Seed 1: tallest 990, 99.9th
percentile about 900, 4% of summit area nearly flat. Ground over 360 blocks above the sea is snow-capped in any
climate. `GENERATOR_VERSION` is 8.

## Round six: snow and alpine forest

Snow is full snow blocks on every face (the snow-over-dirt tile is no longer used) lying 14 blocks deep, with
packed stone only beneath it at 16. Warmth now falls with height wherever trees and grass read the climate,
so pines grow on the slopes of any range (cooler than 0.46 at their height), and a mountain's lower slopes are
a conifer belt (density 0.3) that thins to nothing between 170 and 290 blocks above the sea. `GENERATOR_VERSION` is 9.

## Round seven: meadows on mountain faces

Mountain slopes were all bare rock, so no tree could stand on them. In patches about 50 blocks across (a third
of the ground), mountain ground is treated as no steeper than 0.9 when choosing its blocks, so grass, and the
pines and flora on it, cling to the faces. The alpine rock line moved from 175 to 260 blocks above the sea, and
trees over 120 blocks above the sea are pines in any climate (tropical ranges never get cold enough). The
generator version is 10.

## Round eight: rough foothills

A `range` layer (the mountain biome's share of a wide window, spread twice) marks the foothills. Ground near a
range gets roughness up to 0.8, scarps (12-34 block steps in patches), and gullies and sharp ridges (ridged noise,
22 blocks), as mountains do; cliff carving follows from the steeper faces. Median gradient on foothill ground is
1.49 against 0.29 far from a range (a test needs 3x). Mountain terraces and scarps now use a fixed flight size and
phase per square of ground, blended at square edges, because a flight size that drifted slid ledges across level
ground as phantom cliffs. `GENERATOR_VERSION` is 12.

Cost (game closed): terrain sample 274 ns (was 242); cold chunk p50 plains / mountains / coast 13.7 / 17.9 / 13.2 ms
(was 13.2 / 11.0 / 12.0), p99 52.7 / 52.1 / 47.6. Mountain chunks cost more with gullies, terraces and cliffs on
them; still under the original baseline (26.2 / 27.1 / 19.8). GPU memory at spawn rose to 3.7 GB (from 2.7) with the
rougher ground, so watch it in big ranges on a 6 GB card.

## Round nine: flowing water

`examples/flow_sim` loads round a spot headless, times frames with the water still, then breaks up to 8 banks of
still water nearest the camera and times frames while it flows. Seed 2790361079 at `OCHRE_SPAWN=6730,16960,3`
(a steep river in a gully) spills down the gully to 8,661 flowing blocks over about 45 seconds, then settles.

The flow step itself is cheap: at most 0.63 ms for about 100 changed blocks, four times a second, and queueing
their groups to mesh again 0.05 ms. The cost was swapping the new meshes in: `apply_remeshes` packed every mesh on
the main thread, 0.36 ms a group, and a step finishes up to 20 groups at once, 7-8 ms. Packing now happens in the
meshing task, for groups loading in as well (`pack_group`). Flowing frames, two runs: p99 4.38 to 1.57 / 1.61 ms,
slowest 7.94 to 2.20 / 2.85 ms; still frames p99 0.77 either way. `lod_sim` (seed 7, 80 blocks/s) has no slow
frames or new holes, and flies further in the same time, as its frames are faster.

## Shadow caster pruning and solid shadow pipelines (2026-10-26)

Options 1 and 3 from `shadow-perf.md` are implemented. The runtime prunes terrain casters against
each cascade's camera slice swept toward the actual shadow direction, with filter/bias/sway
padding and a fallback to Bevy's visibility when world curvature is active. Meshes are split
into solid and cutout parts before packing on worker tasks. Solid directional shadows omit
their fragment stage where native depth clipping is available. Camera fade materials retain
their solid/cutout classification and masked rendering; emulated depth-clamp GPUs keep a fragment
depth write. LOD splitting and packing also run on workers to avoid adding work to mesh upload frames.

Seed 1, current settings, 512-block shadows, three cascades, level camera, 15 sun directions:
mean submissions 2.365M to 1.083M triangles (54% fewer); 1.054M retained triangles (97.3%) are
eligible for the pipeline without a fragment stage. `shadow_cull` uses the runtime hull and counts
the split meshes, including child foliage. These are CPU submission counts; GPU time and frame-time
p99 were not measured, so no GPU speedup or p99 improvement is claimed. Splitting can add a draw
for a mesh containing both ground and foliage. See `shadow-perf.md` for correctness guards and
the required rendered A/B check.

`bench` before/after: chunk vertices 620,695 both times; LOD vertices 311,077 both times, with
all eight levels matching exactly. Chunk meshing best of three 208.2 / 216.7 ms; LOD total
78.9 / 79.6 ms. Generation is unchanged; splitting happens later, outside this benchmark.
The seed-1 `lod_sim` route flags one persistent surface chunk after stopping with both the
original branch code and this change. Temporary diagnostic edits were removed after the
baseline comparison. This check does not establish that the existing loader has no gaps.

## Cloud march resolution cap

`renderer.cloud_resolution` selects a target width: low 320, medium 640, high 960, ultra 1280.
Medium is the default. Height follows the window's aspect ratio, and smaller windows retain
half display resolution. Both the march target and its temporal history use this calculation
on startup and resize. At 16:9, medium reduces the 1440p main march from 921,600 to 230,400 rays
per frame; at 4K, from 2,073,600 to 230,400. The fixed reflection target and march step counts
are unchanged. Pixel counts depend on aspect ratio; this caps width rather than total area.

The user reported that both 960×540 and 640×360 retained the desired cloud quality. The second
cap uses 56% fewer main-march rays than the first at 16:9. GPU timings and frame-time p99 were
not measured, so no proportional timing improvement is claimed. See `docs/clouds.md` for presets.

## Tectonic MVP checkpoint (2026-10-27)

Release Rust 1.97.1, managed CPU-only environment (2 vCPUs), seed 1, render distance
1024. `flight --seed 1 --secs 3` uses 179 frames per leg, separate world caches and
unchanged renderer settings. Before is 96679cb; after is the version-39 tectonic MVP.
Geometry and spawn surroundings differ, so vertex equality does not apply. These short
runs are diagnostic, not a stable frame-time benchmark or GPU measurement.

| Headless frame p99 (ms) | Before | After |
| --- | ---: | ---: |
| Loading | 8.101 | 6.298 |
| Walking | 5.730 | 4.816 |
| Sprinting | 7.791 | 5.779 |
| High-altitude flight | 11.997 | 41.383 |

Walk-stage mesh storage was 158 MB before and 159 MB after. High-altitude LOD throughput
fell from 257.0 to 110.5 meshes/s. The larger vertical bounds and increased relief change
the scene and loading workload; they are hypotheses to profile, not an established cause
of the p99 regression. This checkpoint is a draft and should not merge until that
regression is understood or reduced. Normal ground legs do not establish high-altitude
performance.

Final viewer generation times were 13.0 s (42), 11.4 s (1) and 19.1 s (1337), without
concurrent builds/tests. Earlier baseline captures were not all taken under equivalent
load, so no generation speedup or regression ratio is claimed. See
[tectonic-mvp.md](tectonic-mvp.md) for same-scale visual comparisons and open issues.

## Tectonic MVP draft follow-up from 4883cfa

The fixed 900 m high-flight path stays at least 824 m above seed-1 ground, so flying
through newly tall terrain does not explain its slow frames. Raising the camera to
2200 m made high-flight p99 worse in a two-core diagnostic run (14.8 to 22.1 ms),
consistent with more horizon and LOD work; the run was not an A/B of identical scenes.

Temporary timers found 9–12 ms calls in `retire_lods` during the high leg. It checked
membership in the retired vector while retaining the retirement queue, making a large
retirement batch quadratic. A set now handles that membership check. On this 16-core host
restricted to cores 0 and 1 with `taskset`, three alternating old/new pairs of
`flight --seed 1 --secs 3` gave:

| Pair | Old high p50/p99 (ms) | Draft high p50/p99 (ms) |
| --- | ---: | ---: |
| 1 | 6.302 / 16.551 | 4.407 / 11.229 |
| 2 | 5.147 / 16.122 | 3.625 / 13.360 |
| 3 | 6.397 / 18.657 | 4.559 / 11.434 |

The draft wins both frame percentiles in each pair. LOD mesh throughput varies across
the pairs and does not establish a throughput gain. `bench` matched exactly before and
after: 712,019 chunk vertices and 330,471 LOD vertices, including every LOD level.
These timings are diagnostic headless CPU runs and cannot be compared numerically with
the original managed-environment checkpoint. GPU frame time remains unmeasured.

Version 40's steep-bend bank correction adds one upstream profile lookup near river banks.
`bench` still reports 712,019 chunk and 330,471 LOD vertices, with 224.5 ms chunk meshing
and 97.7 ms LOD meshing in one run. A single two-core, three-second seed-1 flight had an
11.566 ms high-leg p99, within the three version-39 draft runs above. That short run does
not establish GPU cost or a precise frame-time change.

## Startup device loss report (2026-10-27)

A desktop launch warned that random seed 1522288356 was unplayable, then lost the GPU device
about 39 seconds later. The cache shows that seed's map saved at 09:01:35 and a replacement
seed 2413427271 saved at 09:01:39. The warning is a rejected map during the normal bounded
retry loop; it did not stop world generation. The later `DeviceLost` and invalid mapped-buffer
errors show a render-device failure, but do not identify its cause. No graphics device is
accessible in this environment, so the GPU failure has not been reproduced here.

For seed 2413427271, a six-second-per-leg headless flight completed at both view distances.
At 512 chunks it held 405 MB of CPU mesh data after load and 459 MB after walking; at the
configured 1024 chunks, 478 MB and 532 MB. Peak process RSS was 1.24 GB and 1.32 GB,
respectively. These are CPU measures, not GPU allocation or frame time. The default game
still needed a GPU check at the time of this measurement.

A second launch with the original 1024-chunk settings and a different random seed also
froze the desktop. Its log says seed 1737182360's map was ready and the player spawned
before the final `bevy_time` warning; the earlier rejected seed was not the blocker.
The user confirmed a temporary 256-chunk safe preset started, then confirmed a fixed
seed started with the original 1024-chunk view, 16-chunk full detail and LOD base,
64-chunk shadows and medium clouds. The safe preset and runtime overrides were removed;
the original renderer settings remain in `assets/settings.toml`. The fixed-seed success
shows the full render settings can start on this GPU, though it does not isolate the
cause of the random-seed freezes. A two-second-per-leg headless flight on seed
1737182360 at the temporary safe preset completed with 102 MB of mesh data after load,
105 MB after walking and 624 MB peak process RSS.

## One map per launch

Both reported hangs followed the random-seed retry path: a geography issue caused the
first map to be discarded, then another map was generated before terrain streaming.
`load_world` now chooses one seed and loads or generates exactly one map. It does not run
the geography validator at startup. The gameplay pass still carves passes and places
spawn regions; the validator remains available to diagnostic map tools. This removes
the second large map generation and its allocation churn from the startup path. It is
plausible that the repeated work contributed to the desktop stalls, but the logs do
not establish that. A fixed-seed launch with the full render settings succeeded on
the user's GPU; random-seed startup with the one-map build has not been checked.
At seed 2154739607, which the former validator rejected, a one-second-per-leg headless
flight now loads that seed directly without a playability warning and completes all legs.

## Initial rock data prototype

The geology fields add five bytes per map cell (20 MiB at the default 2048-square map),
plus temporary generation and serialization storage. This initial prototype did not
feed geology into erosion. Its release `bench` at seed 1 matched the previous build at
every level: 712,019 chunk and 330,471 LOD vertices.

Loading an old cached seed-1 map and initializing a zero-length headless flight on two
cores took 0.33–0.39 s before and 0.44–0.45 s with geology reconstruction over three
alternating pairs. Peak process RSS was about 433 MB before and 465 MB after. This was
measured on the prototype that reconstructed geology from older caches; the subsequent
erosion change uses version-41 caches instead.

Three alternating old/new headless flights, restricted to two cores, used seed 1 and
three seconds per leg. High-flight p99 was 11.234/13.277, 10.393/10.838 and
12.543/12.557 ms (old/new). The new build is higher in these short pairs, by 0.014–2.043
ms. The extra retained map data may change cache or memory pressure, but these runs do
not establish a cause; frame time and GPU cost need longer checks. The sample counts
and timing variance make this a diagnostic rather than a stable performance baseline.

## Rock-dependent stream erosion

The version-41 erosion and water changes alter terrain, spawn surroundings and mesh
counts. At seed 1, `bench` reports 693,310 chunk and 323,777 LOD vertices, against
712,019 and 330,471 before; exact vertex equality does not apply to this change.
Three alternating two-core headless flights, three seconds per leg, gave high-flight p99
of 10.468/11.105, 13.321/10.597 and 13.598/10.605 ms (before/after). Sprint p99 was
3.586/3.529, 3.717/4.983 and 4.717/4.631 ms. The second sprint pair regressed by
1.266 ms; changed terrain and spawn can change streaming work, but these short runs do
not isolate the cause. No GPU timing is available.

## Mountain river containment

The first general neighbour check sampled all four neighbours throughout the indexed
river area. On seed 1, one `bench` run spent 650.1 ms building regions, against 179.8 ms
before the change. A conservative bound using meander movement and possible river-width
variation skips neighbour sampling where water cannot reach an adjacent block; the same
benchmark then took 208.2 ms. Mesh output intentionally changed with the bank geometry:
693,310/693,664 chunk and 323,777/323,873 LOD vertices before/after.

Three alternating two-core headless flights, three seconds per leg, used render distance
1024 and full-detail distance 16. Load p99 was 4.064/3.223, 1.539/3.980 and 3.636/3.104 ms;
high-flight p99 was 11.259/11.637, 11.137/10.281 and 9.407/11.062 ms (before/after).
The second loading and first and third high-flight pairs regressed. Neighbour sampling
adds work and bank geometry changes the scene; these short, variable runs do not isolate
their contribution. No GPU timing was collected. These measurements precede the final
short-circuit that skips width-bound calculations when the nearest river already qualifies.
The final short-circuit retained the same vertex counts and five water-audit summaries;
one `bench` run measured 206.6 ms for regions and 98.5 ms for LOD meshing.
After the clipped-mouth search was included, `bench` still matched those vertex counts,
with 208.2 ms for regions and 104.2 ms for LOD meshing in one run.

Three more alternating two-core flights used the current full-detail distance of 12.
Load p99 was 3.185/3.405, 3.338/3.214 and 3.556/2.280 ms; high-flight p99 was
11.622/11.995, 10.436/11.517 and 11.768/11.257 ms (before/after). High-flight p99 rose
in the first two pairs by 0.373 and 1.081 ms. The new neighbour checks add work near
water and the terrain differs; the short runs do not establish how much each contributed.

## Landform variety draft

With seed 1, one release `bench` run measured 693,664 / 1,896,431 chunk vertices and
323,873 / 458,404 LOD vertices before / after the landform changes. The generated scene
and spawn terrain differ, so this is a workload change rather than a like-for-like mesh
optimization. Chunk meshing took 225.0 / 419.7 ms and total LOD meshing 103.7 / 250.0 ms.
In the second flight's load snapshot, all mesh assets held 96 / 340 MB (2.66 / 9.31
million vertices). A separate instrumented after run measured 335 MB: 5 MB was water,
70 MB was opaque chunk and LOD meshes, and approximately 260 MB was other meshes using
the terrain material, including cutout and inner-leaf faces. All measured mesh assets
were attached to entities. The different spawn scenery is a likely contributor, but the
specific source of the extra terrain-material geometry needs profiling before drawing
conclusions about GPU memory or render performance.

Three alternating two-core headless flights used full-detail distance 12 and three seconds
per leg. High-flight p99 was 12.774 / 8.460, 12.917 / 16.633, and 12.579 / 8.426 ms
before / after. The second pair regressed by 3.716 ms. Changed terrain and mesh workload
are plausible contributors, but these short runs do not isolate the cause; a viewer export
also overlapped part of the measurement. GPU performance has not been measured.

## Rock-dependent slumping draft

One seed-1 release `bench` run before / after reported 1,896,431 / 1,896,431 chunk
vertices and 458,404 / 458,590 LOD vertices. Region generation took 124.8 / 123.4 ms,
chunk meshing 417.9 / 416.3 ms and total LOD meshing 249.9 / 253.7 ms. These single
runs do not establish a performance change. The map and mesh workload change with
erosion; map generation and frame-time A/B comparisons remain to be measured.

## Lake shore transition draft

Three alternating seed-1 release `bench` runs before / after the wider lake search
measured region generation at 123.7 / 144.2, 124.5 / 143.8 and 123.7 / 143.1 ms.
Total LOD meshing was 253.0 / 267.6, 252.7 / 268.3 and 252.8 / 268.6 ms. The wider
per-sample search is a likely contributor. The sampled scene produced 1,891,787 chunk
and 458,029 LOD vertices in both builds, but the shore change alters terrain elsewhere.
Frame-time p99 and GPU performance were not measured.
