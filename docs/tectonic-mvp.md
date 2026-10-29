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
- Erosion and water routing run over the new ground. Stream erosion varies with rock type.
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
- Generator version 42 invalidates older maps. Cache identity includes bounds, geology
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

Comparisons used the same seed, coordinates, scale and elevation palette; generator
geometry intentionally changes.

Seed 42 rises from a previous maximum of 542 m above sea level to 1451 m. The new
maximums for seeds 1 and 1337 are 1828 m and 1984 m. These are map heights before
block-scale detail. The warped plate boundaries improve the straight initial belts;
geological shape and erosion still need further visual work.

### Review overlays (seed 42)

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
- Headless streaming timings and follow-up profiling are recorded in
  [terrain-perf.md](terrain-perf.md). GPU timings remain unmeasured.
- Water audits on seeds 1, 42 and 1337 report zero map-level uncontained cells.
  River-edge wall flags are 26/21/75, including 1/1/2 over 6 m. Seed 42's tall
  flag is near `x=-21752, z=15520`. These are unresolved diagnostic findings.
- The seed-42 seam diagnostic completed. Completion is not evidence that every seam is
  artifact-free.
- In-game mountain silhouettes and cloud/shadow intersections have not been systematically
  reviewed on a graphics device. The coast-parallel river remains unresolved.

### Draft follow-up from checkpoint 4883cfa

The seed-42 river visible near the southern coast was traced with `worldprobe route` from
`(3216,1392)` to the sea at `(6352,3632)`. Its map water surface falls from 59.9 m at
`(3472,1488)` to sea level. At the `x=4000` transect, sampled dry ground north and south
of the channel reaches 70.8 m at `z=1400` and 75.7 m at `z=2100`. This supports a local
valley barrier, but the checkpoint did not identify the exact reported segment; a longer
route or a better outlet elsewhere has not been excluded. Keep the visual issue open.
The detail view covered `x=0..8192, z=0..8192` at 8 m/pixel.

The audit previously counted every water kind inside each 192 m window. Its three seed-1337
"coast walls" were river water near the sampled coast cells, including a flagged river at
`(-10212,26188)`. Counting only the sampled kind in each category gives zero lake and coast
walls or surface steps for all three seeds. River walls remain 26/21/75 and steps remain
529/981/2183 for seeds 1/42/1337. Seed 42's 6.8 m river wall lies on a steep reach whose
water falls roughly 12 m over 24 blocks, consistent with the waterfall exception in
[water.md](water.md). The other river flags still need block-level review. The seed-42 wall
view covered `x=-21952..-21552, z=15320..15720` at 1 m/pixel.

### Draft steep-bend correction

At `(-21752,15523)` the seed-42 river surface was 775.51 m, while the dry column one block
south stood at 774.12 m. The closest point on the steep curve moves downstream as the sample
moves across its edge, so the dry bank used a lower river level. The version-40 fix read the
surface about one block upstream from nearby stretches of the same river. That dry column
stands at 776.42 m; the water's downhill profile and audit wet-column counts are unchanged.
The later block-neighbour containment check replaces that upstream estimate.
Generator version 40 invalidates older saved terrain because the banks change.
The matching viewer comparison changed 102 of 160,000 pixels, concentrated along banks;
the river footprint remained in place.

| Seed | River walls before / after | Over 6 m before / after | Lake and coast walls after |
| --- | ---: | ---: | ---: |
| 1 | 26 / 16 | 1 / 0 | 0 |
| 42 | 21 / 16 | 1 / 0 | 0 |
| 1337 | 75 / 48 | 2 / 0 | 0 |

The bend audits use the same 4 m sampling windows as the review reports. Seed 2790361079,
a known steep gully, has 11 river wall flags and none over 6 m.
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

### Later rock pass

After the initial landform, climate and visual stages, expand the rock palette and give
each addition a role in terrain and appearance:

- **Granite:** intrusive mountain cores, domes and fractured cliffs.
- **Shale:** layered sedimentary rock alongside limestone and sandstone; retain clay as
  a softer sediment or deposit.
- **Gneiss:** banded metamorphic rock in mountain belts, alongside marble.
- **Andesite:** volcanic rock for island arcs and volcanoes, alongside basalt.

Start with granite and shale, then add gneiss and andesite as their landforms develop.
Tune erosion and make the strata visible with the block texture pass; names and textures
alone will not create distinctive landforms.

## Rock data: first landform-variety step

The world map now records broad volcanic, metamorphic and sedimentary provinces, based
on the seeded tectonic fields. Volcanic provinces sample as basalt, metamorphic ones as
marble. Sedimentary provinces alternate limestone, clay and sandstone beds every 160 m,
with a smoothly varying vertical offset. `WorldMap::rock_at` samples those beds at an
arbitrary elevation; the viewer's `geology` layer colors the rock at the surface. This is
a simplified starting model, not a geological history or a final visual treatment.
At this stage provinces are sampled at 32 m map-cell resolution, and their bedding is
smoothly offset rather than folded or faulted.

The geology layer uses dark for basalt, pale for marble, tan for limestone, red for clay
and orange for sandstone.

Stream erosion now reads the rock at the current surface on each pass. Basalt and marble
have lower stream erodibility than limestone; clay and sandstone have higher values.
Hydraulic droplets and thermal slumping still use the same rules for all rocks. Terrain
blocks and textures do not yet use the rock data.

The first erosion trial exposed a river profile discontinuity on seed 1: adjacent samples
at `(11508,-2708)` and `(11508,-2707)` differed by nearly 8 m because two consecutive
stretches disagreed at their shared endpoint. Profiles now meet there; the difference is
about 1.7 m. Tests retain this join, a block-level bank edge, a spring rim and the
earlier steep-bend fixture.

In the four seed audits, lake and coast walls were
zero. The fine check found no open block edges among the sampled river columns; the
four-block river check still reported 2, 0, 5 and 1 walls over 6 m for seeds 1, 42,
1337 and 2790361079. Some are steep reaches with intervening banks. These are sampled
checks, and the remaining waterfall and bend flags still need visual review.

The new erosion changes terrain, so generator version 41 uses new map caches. Version-40
terrain and its saved edits do not match the new world. Different block materials and
textures are deferred to the visual composition pass. Rifts, island arcs, geological age
and additional landform work remain in this iteration.

Matching views of seed 1 at 64 m/pixel and a seed-42 mountain basin at 4 m/pixel compare
the geology-data-only build with rock-dependent erosion. The basin view shows how a terrain
change can also change a lake and its drainage.

### Mountain river flats

On seed 3942325500, a river's bank hold raised the surrounding terrain to a constant
height far beyond its channel. At `(20749,-1920)` it raised ground by about 16 m. The
channel now eases into the terrain over its bank width; at full block detail, dry ground
beside actual water is held to the water's block height. This replaces the separate
upstream bank-floor estimate, so bends and spring edges use the same check. The new
terrain uses generator version 42. See [water.md](water.md) for the containment rule and
the sampling limits of the water audit.

Both views cover `x=20600..21624, z=-2320..-1296` at one block per pixel. In the
five seed audits, no sampled river column had an open neighbouring block. The
four-block scan still flags steep banks and falls; it is
not a direct test of whether the intervening blocks contain the water.

## Landform variety draft

Plate divergence now makes continental rift valleys with raised shoulders and submarine
spreading ridges. Lateral motion makes narrower transform valleys. Converging oceanic
plates produce a trench and a broken volcanic island arc. These shapes enter the map
before erosion and water routing. Stream erosion still uses the exposed rock type;
hydraulic and thermal erosion do not yet vary by rock.

The map also stores a relative crust-age indicator: seeded plate ages blend at plate
edges, with younger values at active spreading zones and volcanic arcs. This is a
procedural snapshot, not dated geology or a history of plate movement. The viewer's
`geological_age` layer shows young crust in blue and old crust in gold. Age does not
yet affect erosion or sedimentary bedding.

Generator version 43 invalidates earlier map caches because the terrain changes.
Matching overview captures for seeds 1, 42 and 1337 show new island chains and rift
lakes; these still need ground-level review. Fine water audits on five seeds found no
sampled open river block edges and no lake or coast wall flags. The coarser river scan
found 31 tall-wall flags on seed 1337, up from 20 before this draft; steep falls and
their appearance remain open for review. A seed-42 seam scan reported 28 candidate
steps across 374 sampled windows; the scan alone does not establish which are artifacts.
A short seed-1 headless loading simulation reported no holes in its sampled window.
The headless timing and vertex-count comparison is recorded in
[terrain-perf.md](terrain-perf.md).

## Rock-dependent slumping draft

Thermal erosion now varies its angle of rest with the exposed rock in each map cell.
The threshold uses the inverse square root of the existing stream-erodibility tuning:
harder basalt and marble retain steeper slopes, while softer sedimentary beds slump
further. The donor's threshold is used for both removal and distribution. These are
artistic values, and deposited material is not tracked as a separate sediment layer.
At this point hydraulic droplets still used a uniform erosion rate; geological age
remains diagnostic.
Generator version 44 invalidates earlier terrain caches and saved edits.

For seed 42, matching 64 m/pixel overview exports changed 656 of 1,048,576 pixels,
mostly in steep terrain. Drainage changed near the old seed-42 river-edge fixture,
which was moved to a new river reach. Fine water audits for seeds 1, 42 and 1337 found
no sampled open river block edges and no lake or coast wall flags. The coarse river
scan reported 5, 0 and 17 tall-wall flags respectively. Those flags, drainage changes,
and ground-level appearance need visual review; the sampled audits do not prove every
bank is contained.

## Rock-dependent hydraulic erosion draft

Droplets now scale their erosion demand at each brush cell by the exposed rock, sampled
again as its height changes. Basalt resists cutting more than clay. The coefficients are
artistic values at most one, so the brush cannot take more than the original downhill
limit. Sediment carried by a droplet still equals the material removed; deposition and
the existing handling of droplets leaving the grid are unchanged. Version 44 includes
both rock-dependent passes.

On seed 42, matching 64 m/pixel overview exports before and after this hydraulic pass
changed 4,231 of 1,048,576 pixels, including changes to lakes and drainage. Fine water
audits for seeds 1, 42 and 1337 found no sampled open river block edges and no lake or
coast wall flags. The coarse river scan reported 0, 1 and 20 tall-wall flags respectively.
These sampled checks do not establish containment everywhere. Ground-level scenery,
waterfalls, seam candidates and changed drainage still need visual review.

## Foothill relief adjustment

Foothill scarps, gullies and the range-driven roughness floor now use 60% of their
previous strength. Mountain relief keeps its own full-strength treatment. A matching
block-scale view around seed 300059728 at `x=-12594, z=-340` changed 22,837 of
1,048,576 pixels; the foothills still need ground-level review. A fine water audit on
that seed found no sampled open river edges or lake and coast wall flags. Generator
version 45 prevents terrain from version 44 being reused with the adjusted foothills.

The lake near `x=-8240, z=3068` has a conspicuous shore band. At `x=-8304, z=3108`,
the surrounding terrain detail would place ground about 20 blocks below the lake, so the
shore hold raises it to contain the water. Previously the hold and calming faded within
one map cell, leaving a steep bank. A wider lake-only falloff spreads the transition.
Along the sampled transect its largest four-block rise near the old transition falls
from about 9 blocks to under 2. A ground-level appearance review remains needed.
Fill-spill routes water on the map before block-scale detail is applied; it cannot see
these later gullies. The wider fade reduces the abrupt correction but does not make
lake routing account for detailed terrain. Generator version 46 invalidates version-45
terrain and saved edits.
