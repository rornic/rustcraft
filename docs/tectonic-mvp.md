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
  flag is near `x=-21752, z=15520`. These are unresolved diagnostic findings, not
  a clean water validation. Seed 1337 also has three coast wall flags below 6 m.
  River/lake surface-step flags need inspection to distinguish falls and transitions
  from artifacts. [Raw audit and flight reports](data/tectonic-mvp/) are retained.
- The seed-42 seam diagnostic completed; its reported ground steps are retained in
  the raw report. Completion is not evidence that every seam is artifact-free.
- GPU rendering, in-game mountain silhouettes and cloud/shadow intersections have not
  been validated on a graphics device. The coast-parallel river remains unresolved.

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
