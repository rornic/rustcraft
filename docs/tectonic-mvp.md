# Tectonic world MVP

The first iteration replaces noise-selected mountain belts with a procedural tectonic
snapshot. The art direction is an alpine coastal region: deep offshore water, rugged
coasts, forested slopes, high peaks and a sheltered dry interior. The map remains 64 km
across at 32 m/cell; it represents a compressed region, not an entire planet.

## What changes

- World bounds are signed and configurable, defaulting to -1024..2048 (exclusive top).
  Sea level is zero. Missing water is identified by its kind, never by positive elevation.
- Twelve seeded plates carry motion. Continental crust crosses plate boundaries.
  Relative convergence makes collision belts or asymmetric subduction ranges/trenches.
  Other boundary types are identified but do not yet get specialized landforms.
- Erosion and water routing keep their existing rules and run over the new ground.
  Fixed peak rescaling is removed; the vertical budget is not a mountain shape algorithm.
- Flood queues use signed sortable height keys. Raw positive-float bit ordering cannot be
  used for negative ocean elevations. Monotone height interpolation preserves steep lake rims.
- Rivers remain water in coarse samples when their width resolves at that cell size; the
  old unconditional 16 m cutoff hid even broad rivers from overview maps and distant LODs.
- A regional warmth baseline replaces the miniature equator-to-pole gradient. Wind
  carries ocean moisture and loses precipitation on rising ground. Soil moisture reflects
  precipitation versus evaporative demand, without percentile-based biome quotas.
- Curvature retains its previous radius independently of build height. Clouds move above
  the intended summits. Snow and treeline elevations are tuned for the larger relief.
- Generator version 39 invalidates older maps. Cache identity includes bounds, geology
  and climate configuration; no legacy generator is kept for existing worlds.
- Range connectivity uses inland water surface slopes, keeping riverbed depth separate
  from mountain barriers. River crossing availability is still checked independently.

The world configuration lives under `[world]` in `assets/settings.toml`, with nested
`bounds`, `tectonics` and `climate` tables. Wind components point downwind; sea-level
warmth is normalized to 0..1. Runtime height expansion and variable sea level are outside
this iteration.

## Reviewing the maps

Use the repository's `.claude/skills/worldview/SKILL.md` workflow. Build the viewer in
release mode, run it, and export matching views:

```sh
cargo build --release --example worldview
target/release/examples/worldview 1 8777
# In another terminal:
bash scripts/capture-worldview.sh http://127.0.0.1:8777 target/tectonic-mvp/after after
```

The capture script uses seeds 1, 42 and 1337, the same coordinates, 64 m/pixel and
1024-square exports. Before captures use the original generator with only the viewer's
elevation palette expanded to match the after palette. Elevations are relative to each
generator's sea level. Do not compare independently normalized height images.

New viewer layers:

| Layer | Meaning |
| --- | --- |
| plates | Plate membership; distinct categorical colors |
| boundaries | Red collision, purple subduction, cyan extension, yellow shear |
| motion | White arrows showing plate motion direction |
| uplift | Pre-erosion uplift, dark to orange, 0..1400 m |
| precipitation | Background rainfall, tan to blue, 0..1 calibrated relative units |
| surface_temperature | Temperature after altitude cooling, blue cold to red warm |
| rivers | Drainage network, with display widths clamped for legibility; actual widths are in `/summary` |

Click inspection includes plate, boundary, uplift, precipitation, wind and bounds.
Boundary colors show influence bands rather than infinitesimally thin fault lines.
The existing temperature layer remains the sea-level field, distinct from surface temperature.
`/summary?seed=42` reports elevation range, peak position, maximum physical river width,
its location and its effective runoff area. The network layer is diagnostic; the regular
water layer and close-up exports show the sampled water surface.

## Checkpoint review (2026-10-27)

**Draft checkpoint, not ready to merge.** The coastal river that appears to run parallel
with the sea is an open visual correctness issue reported during review. A coastal
barrier could explain such a route, but that has not been established for this map.
Inspect drainage direction, terrain and water-surface profiles, flood-fill spill heights,
and the actual sampled water before accepting it. Visibility and routing are separate:
the coarse-sampling fix makes rivers visible but does not prove their routes are right.

A later desktop launch lost the graphics device after world generation. The rejected-seed
warning preceded a retry; both reported freezes followed this path.
The render failure has not been reproduced without a GPU; its headless comparison is
recorded in [terrain-perf.md](terrain-perf.md). A temporary safe-render preset started,
and a fixed seed then started with the original full render settings. The preset was
removed and the original renderer settings remain.
The random-seed geography rejection and retry have since been removed: each launch
builds one map, including seeds whose geography still has issues. The map diagnostic
validator remains available without affecting startup.

### Matching landscape comparisons

Same seed, coordinates, scale and elevation palette; generator geometry intentionally changes.

| Seed | Before | After |
| --- | --- | --- |
| 42 | ![Seed 42 before](images/tectonic-mvp/seed-42-before-landscape.png) | ![Seed 42 after](images/tectonic-mvp/seed-42-after-landscape.png) |
| 1 | ![Seed 1 before](images/tectonic-mvp/seed-1-before-landscape.png) | ![Seed 1 after](images/tectonic-mvp/seed-1-after-landscape.png) |
| 1337 | ![Seed 1337 before](images/tectonic-mvp/seed-1337-before-landscape.png) | ![Seed 1337 after](images/tectonic-mvp/seed-1337-after-landscape.png) |

Seed 42 rises from a previous maximum of 542 m above sea level to 1451 m. The new
maximums for seeds 1 and 1337 are 1828 m and 1984 m. These are map heights before
block-scale detail. The warped plate boundaries improve the straight initial belts;
geological shape and erosion still need further visual work.

### Review overlays (seed 42)

| Moisture before | Moisture after |
| --- | --- |
| ![Moisture before](images/tectonic-mvp/seed-42-before-moisture.png) | ![Moisture after](images/tectonic-mvp/seed-42-after-moisture.png) |

| Plates, boundaries and motion | Precipitation |
| --- | --- |
| ![Plate overlay](images/tectonic-mvp/seed-42-plates.png) | ![Precipitation](images/tectonic-mvp/seed-42-precipitation.png) |

| River network (display widths exaggerated) | Temperature after elevation cooling |
| --- | --- |
| ![River network](images/tectonic-mvp/seed-42-rivers.png) | ![Surface temperature](images/tectonic-mvp/seed-42-surface-temperature.png) |

The region is 64 km across. Seed 42's widest physical river is 84.3 m at
`x=-14384, z=848`, with effective runoff area 35.16 km². Seeds 1 and 1337 have maximum
widths of 46.0 m and 71.7 m. A continent-scale river is not an expected consequence of
more vertical relief: catchment area, precipitation and routing determine discharge.
At 64 m/pixel, even the widest river is only about one pixel wide. The cyan network
is therefore a diagnostic overlay, not a literal rendering of those widths.

An earlier prototype's approximately 575 m river was invalid: raw float-bit ordering
broke flooding below zero. It is superseded by these exports and must not be used as
evidence of a successful large river.

### Validation and remaining work

- 364 release workspace library tests passed (base 24, engine 5, gameplay 18,
  terrain 237, world 80).
- Release Clippy across the workspace and all targets passed; formatting and shell
  syntax checks passed. Release viewer, flight, benchmark, water audit and seam tools built.
- Signed flood ordering, water at zero, coarse river sampling, cache configuration,
  deterministic terrain, variable vertical bands and range connectivity have regression coverage.
- Headless streaming timings are recorded in [terrain-perf.md](terrain-perf.md).
  The high-altitude leg regresses substantially and needs profiling before merge.
- Water audits on seeds 1, 42 and 1337 report zero map-level uncontained cells.
  River-edge wall flags are 26/21/75, including 1/1/2 over 6 m. Seed 42's tall
  flag is near `x=-21752, z=15520`. These are unresolved diagnostic findings.
  [Raw audit and flight reports](data/tectonic-mvp/) are retained.
- The seed-42 seam diagnostic completed; its reported ground steps are retained in
  the raw report. Completion is not evidence that every seam is artifact-free.
- In-game mountain silhouettes and cloud/shadow intersections have not been systematically
  reviewed on a graphics device. The coast-parallel river remains unresolved.

### Draft follow-up from checkpoint 4883cfa

The seed-42 river visible near the southern coast was traced with `worldprobe route` from
`(3216,1392)` to the sea at `(6352,3632)`. Its map water surface falls from 59.9 m at
`(3472,1488)` to sea level. At the `x=4000` transect, sampled dry ground north and south
of the channel reaches 70.8 m at `z=1400` and 75.7 m at `z=2100`. This supports a local
valley barrier, but the checkpoint did not identify the exact reported segment; a longer
route or a better outlet elsewhere has not been excluded. Keep the visual issue open.
The [detail view](images/tectonic-mvp/seed-42-coastal-river-detail.png) covers
`x=0..8192, z=0..8192` at 8 m/pixel.

The audit previously counted every water kind inside each 192 m window. Its three seed-1337
"coast walls" were river water near the sampled coast cells, including a flagged river at
`(-10212,26188)`. Counting only the sampled kind in each category gives zero lake and coast
walls or surface steps for all three seeds. River walls remain 26/21/75 and steps remain
529/981/2183 for seeds 1/42/1337. Seed 42's 6.8 m river wall lies on a steep reach whose
water falls roughly 12 m over 24 blocks, consistent with the waterfall exception in
[water.md](water.md). The other river flags still need block-level review. The corrected
[review audit reports](data/tectonic-mvp/) retain example coordinates for each category.
The [seed-42 wall view](images/tectonic-mvp/seed-42-river-wall-detail.png) covers
`x=-21952..-21552, z=15320..15720` at 1 m/pixel.

### Draft steep-bend correction

At `(-21752,15523)` the seed-42 river surface was 775.51 m, while the dry column one block
south stood at 774.12 m. The closest point on the steep curve moves downstream as the sample
moves across its edge, so the dry bank used a lower river level. The bank now also reads the
surface about one block upstream from nearby stretches of the same river. That dry column
stands at 776.42 m; the water's downhill profile and audit wet-column counts are unchanged.
Generator version 40 invalidates older saved terrain because the banks change.
The matching [after view](images/tectonic-mvp/seed-42-river-wall-after.png) changes 102 of
160,000 pixels in the [viewer diff](images/tectonic-mvp/seed-42-river-wall-diff.png), concentrated
along banks; the blue river footprint remains in place.

| Seed | River walls before / after | Over 6 m before / after | Lake and coast walls after |
| --- | ---: | ---: | ---: |
| 1 | 26 / 16 | 1 / 0 | 0 |
| 42 | 21 / 16 | 1 / 0 | 0 |
| 1337 | 75 / 48 | 2 / 0 | 0 |

The [bend audit reports](data/tectonic-mvp/) use the same 4 m sampling windows as the review
reports. Seed 2790361079, a known steep gully, has 11 river wall flags and none over 6 m.
Remaining river flags, waterfall curtains, and GPU appearance still need visual
review; a lower audit count alone does not establish that every steep bend looks right.

## Next iterations

1. **Landform variety:** rifts, spreading ridges, island arcs, transform valleys, geological
   age, rock resistance and erosion that preserves their different structures.
2. **Climate and ecology:** seasonal wind patterns, maritime moderation, inland extremes,
   climate-derived treelines and separate biome/landform properties.
3. **Visual composition:** strata, scree, vegetation transitions, atmospheric tuning,
   scenic routes and consistent silhouettes across LOD transitions.

Every iteration needs matching viewer exports, deterministic fixture tests, water audits,
seam/loading checks and timing comparisons. Geological plausibility supports the scenery;
it does not substitute for looking at it from the ground.
