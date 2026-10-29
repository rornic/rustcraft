# Water

How the sea, lakes and rivers are made, and the rules that keep them from leaking. Read this before
touching `ochre-world/src/worldmap/{hydrology,water,source}.rs`.

## The one rule

Water is held in by ground at least as high as its surface. A wall of water is a wet column beside a
dry one whose ground is under the water's surface. Everything below exists to make walls impossible
by construction, not to patch them where they show up.

Two exceptions are meant: a river's waterfall stands over the ground at its foot, and the terrain
crate draws a curtain there; and a river can run on a levee across a slope, held in by its own bank.

## Map level: `hydrology::run`

`worldmap::generate` runs hydrology twice: once on the raw terrain, so small basins are drained
before erosion wears the ground, and once on the final ground. Nothing that comes after the last run
may lower ground beside still water under its level. `gameplay::crossings` (bridge sites flatten the
banks of wide rivers) is the one later edit, and it keeps every cell a block over any lake or sea
beside it (`held_over`).

The last run, in order:

1. `scatter` adds up to a block of noise to the ground, for good, so rivers on gentle ground wind
   instead of following the flood's straight lines. It used to be added for the flood and taken out
   again, which left lakes worked out on ground a block off the ground they lay in.
2. A priority flood (Barnes et al.) from the open sea and the map's edge gives every cell the height
   it fills to and the cell it drains into. The cells it raised, joined eight ways as the flood moves,
   are basins, each with one spill height (its brim).
3. Basins smaller than `MIN_LAKE_CELLS` or shallower than `MIN_LAKE_DEPTH` are drained by `drain`:
   a channel from the floor out along the flood's path, each cell under the last, until it reaches
   lower ground (complete breaching, after Lindsay 2016). It stops at water downstream rather than
   drain that too, and never cuts under the sea. A small hollow already under the sea is filled to just
   over it instead, so the sea can't come in up a channel. Up to `DRAIN_ROUNDS`, as draining one basin
   can open another.
4. `balance` sets each basin's lake level from its water budget, in the spirit of Fill-Spill-Merge
   (Barnes, Callaghan and Wickert 2021) but for a steady state rather than one storm. Flow runs
   downhill with rain from `rain` (runoff by dryness and rock, the same model rivers use). All that
   drains into a basin is gathered; the lake spreads cell by cell from the floor up, each lake cell
   losing `LAKE_EVAPORATION` by how dry it is, until it loses all that comes in, or reaches the brim
   and spills the rest on. A lake is either full, spilling from its surface into the river that leaves
   it, or held under its brim by what it loses, with no outlet. There's no depth or size cap: capping a
   wet basin's lake left water with nowhere to go but a river starting from nothing at the rim. Then:
   - a basin whose floor is under the sea holds water at least `INLAND_SEA_ABOVE_SEA` over it, or it
     would stand dry under the sea's level, which the terrain crate would fill with sea;
   - fewer than `MIN_POND_CELLS` under the level and the basin is a dry flat.
   Pits inside one basin share its level: two puddles in a dry basin stand at the same height. That
   is a simplification of FSM's hierarchy, and the only place two separate waters are forced level.
5. Basins whose lake doesn't fill them (closed, or held) are rerouted by `reroute`: a flood from the
   lake outward over the basin, so the rivers in it run down to the lake, not out over the brim the
   first flood found. Flow is gathered again over the new routing, with lakes as sinks.
6. Sea cells are the open sea; lake cells are each basin's cells under its level, their beds dug at least
   `SHALLOWEST` under the whole block their water is drawn at, so every cell the map calls lake is lake
   in blocks too (otherwise a lake whose cells only just hold water comes apart into pools); rivers start where
   enough water drains and the ground is steep enough, and are carved into the ground as before, but
   never under the level of a lake or the sea beside the cell (`still_beside`): a carved cell under a
   lake's level would let the lake's water into its whole square.

`water_kind`, `water_surface`, `river_width`, `flow` and `down` are what it leaves on the map. Lakes
and the sea are no longer decided separately: a walled-in basin under the sea is just a lake.

What it means for the world: wet country's lakes fill to their brims as before; dry country's stand
lower in closed basins, or are dry. `LAKE_EVAPORATION` (wet, driest) is the knob: raise the second
number for smaller desert lakes.

## Block level: `WorldTerrain::sample_for_cell`

The terrain crate reads one `TerrainSample` per column (`ground`, `water_surface`, `water_kind`,
`spring`) and nothing else, so all of this lives in `ochre-world`.

### Still water

`still_round` finds the sea or the lake whose cells are among the four round a point, at its level
(a lake's floored to a whole block). That is all a point needs, because of a property of the flood:
every cell beside (eight ways) a basin's cell is either in the basin or at least as high as its brim,
or the water would have spilled there. So among the four cells round any point near a lake, every
non-lake corner is at or over the lake's level, and the water's edge always falls inside the four.
Where the smooth ground (`map.height_at`, bicubic) is under the level, the point is in the lake. Where all
four cells round it are the water's, which no rim can be among, it's in the lake up to `DROWN` over the
level too, fading as fewer of the four are, so low strips between a lake's cells go under.
The shore is the smooth ground's contour at the level: no cell's side ever shows, and no water ever
stands past a rim's crest on ground falling away beyond it. The same holds for the sea.

`contain` then holds the ground to the side of the water its smooth ground is on: under the level
where the smooth ground is, by as much as it is up to `HOLD`, and over it by as much where it's over.
The detail laid on the ground can't poke an island through the water or dig a dry pit under its
level beside it. The hold on the dry side is looked up beyond the water (`still_by`) and fades by
`near`, so it does not stop at a cell edge. Lake shore smoothing extends over one more map cell,
with a gradual falloff. This eases the transition into deep detail gullies outside a lake rather
than forcing the whole drop into one cell. Sea smoothing retains its original reach. Detail is
also calmed to nothing at the water's edge over `CALM` blocks, for smooth shores.

Inland, out of any lake, the ground is never under the sea (`LAND_ABOVE_SEA`).

### Rivers

Rivers are curves through their cells (`Stretch`), each with a level profile that never rises
downstream, ramps to the lake or sea it runs into, and starts at the lake it leaves
(`build_river_profiles`). `RiverProfiles::index` lists each stretch in every cell its water, banks and
valley reach, plus the meander, so a point looks only at its own cell's list, however wide the river.
Each stretch also knows its river (`stem`, the run of widest feeders it's part of), so the second river a
point sees is another river, never another stretch of a wide one: a river wider than its cells reaches
over several of its own stretches, whose different levels read as humps if taken for a second river.
Consecutive profiles share their endpoint level: a lower downstream start brings the upstream
endpoint down to it, carrying the drop in the last profile interval instead of jumping between
different heights at the same point.

`channel` cuts the bed and eases the bank into the surrounding terrain over its width.
At full block detail, a dry column near a river then checks its four actual neighbours. If a
neighbour contains water, the dry column is raised to the block containing that water's
surface. The same containment check handles bends, springs and adjacent still water without
estimating a neighbouring level from an upstream curve point. A conservative geometric
search bound avoids checking neighbours far from water; LOD cells keep the channel's smooth
geometry rather than this block-level correction.

Where a river meets still water:
- **joined**: its level within `JOIN` of the still water's, as at a mouth or outlet. Inside the
  still water the river is left out; the still water's shore is the edge there. Near it the river
  cuts its bed but builds no banks, and its water takes the still water's level, so the two meet at
  one height instead of being parted by a strip of bank.
- **under** the still water: the still water covers it.
- **over** it, passing by: it is cut and banked as ever, and its banks keep the two apart.

The same barrier holds between two rivers: the second river's bank holds too unless the two are
joined or the point is in the nearest's water (where a stream runs into a river, its banks would
otherwise stand up through the river as strips), and a second river's bed isn't cut where it would sink the first's bank under the first's
water. This is the rule Minecraft's aquifers use: wherever two waters at different levels meet,
ground stands between them.

## Flowing water

Generated water is still and never moves. Water the player lets out flows (`ochre-terrain/src/flow.rs`):
it falls as far as it can, spreads up to 7 blocks sideways from where it rests, a level less each block,
and dries up back from wherever it's cut off. Flowing levels live in `ChunkData` beside the blocks.

The one rule now matters at block level, not only to the eye: still water beside or over air or flora
spills the moment an edit nearby stirs it. `test_lakes_and_the_sea_have_no_open_block_beside_or_under_their_water`
holds lakes and the sea to that. Rivers have open blocks at every step down their course, so flow never
spreads over still water, only joins it. A waterfall's lip still spills when stirred.

Neighbouring surfaces less than `WATER_SLOPE_MAX_STEP` (2 blocks) apart are drawn as one slope, so a
rapid climbing more than a block a column is a ramp, not stairs. Bigger drops are curtains.

## Checking it

- `cargo test --release -p ochre-world`: `test_no_lake_or_sea_cell_stands_over_the_dry_cells_beside_it`
  (map) and `test_no_lake_or_sea_stands_as_a_wall_over_the_ground_beside_it` (blocks round every
  40th shore cell) over three seeds. The hydrology tests cover basins, draining, below-sea basins and
  rivers that never rise.
- The terrain crate's water tests (`generator/tests.rs`) are the block-level regression spec: walls,
  crevices, joins, ramps, LOD water.
- `cargo run --release --example water_audit -- <seed>` counts walls, steps and raised banks round
  rivers, lake shores and coasts, one `AUDIT` line each, for a sweep of seeds before and after a
  change. Each line counts only its own sampled water kind, even when a window also contains
  another kind. `--fine` also checks the four immediate neighbours of sampled river columns,
  reporting height gaps and edges where the rounded dry ground lies below the water block.
  This samples the existing windows rather than every river column. Walls left on rivers are
  mostly waterfalls.
- The `worldview` skill to look, and `worldprobe <seed> info x z` for one column's ground, dry
  ground, smooth ground, water and map cell.

Before this model (seeds 368871838, 5, 2198630850, the default 65 km world): lake walls over 1.5
blocks 0 / 23 / 16 (6+ blocks: 0 / 6 / 10), dry map cells under a lake's level 32 / 42 / 61. After:
lake walls 0 / 0 / 2 (none over 6), dry cells under a lake's level 0 / 0 / 0.
Across 16 seeds: lake walls 61 to 3 (over 6 blocks 17 to 0), dry map cells under a neighbouring
lake's level 543 to 0, river walls 105 to 60 (over 6 blocks 4 to 0).

## Research

- Barnes, Lehman, Mulla 2014, *Priority-flood*: the flood every basin and river comes from.
- Barnes, Callaghan, Wickert 2021, *Fill–Spill–Merge*, Earth Surface Dynamics 9: lakes from a water
  budget over the basin hierarchy.
- Lindsay 2016, *Efficient hybrid breaching-filling sink removal*, Hydrological Processes 30: draining
  a basin by one channel all the way out.
- Génevaux et al. 2013, *Terrain generation using procedural models based on hydrology*, and Fischer,
  Boeckers, Zachmann 2022: water first, ground shaped to hold it.
- Minecraft's aquifers (1.18): per-region fluid levels, with stone only where neighbouring levels
  disagree.
