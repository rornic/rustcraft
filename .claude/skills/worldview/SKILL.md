---
name: worldview
description: Inspect rustcraft's generated world (terrain, water, rivers, lakes, biomes, blocks) with the browser world viewer and its PNG export, instead of hand-built probe scripts. Use when debugging world generation, checking a terrain or water fix, hunting artefacts near a coordinate, or sharing a map image.
---

# Worldview

`examples/worldview.rs` serves a seed's whole world on localhost. It shows exactly what chunks are built from at 2 blocks a pixel or finer (`RegionStore` ground and water, same as `generate_chunk`), so a render here is the evidence for a terrain or water change. Use it before launching the game.

## Start it

```
cargo build --release --example worldview
(nohup cargo run --release --example worldview -- <seed> 8777 > /tmp/worldview.log 2>&1 &)
until curl -s -o /dev/null http://127.0.0.1:8777/; do sleep 3; done
```

- One seed is held in memory, generated on first request (about 4.5 s), tiles after that about 0.2 s.
- It does not reload code. After any change to world generation, stop it and rebuild. Stop it by pid (`pgrep -f '^target/release/examples/worldview'`), never `pkill -f worldview`, which matches your own shell command and kills it.
- The game's seed is in its log (`world seed is ...`). A coordinate quoted by the user only makes sense on the seed it was seen on, so ask for the seed if unsure.

## Look

Export a PNG and read it with the Read tool:

```
curl -s 'http://127.0.0.1:8777/export?seed=S&layers=hillshade,water:0.8,waterlevel:0.5&x=X0&z=Z0&w=800&h=800&scale=1' -o view.png
```

- `x`, `z` are the world position of the image's top-left corner (centre minus half the size times scale), `scale` is blocks a pixel, size up to 4096.
- Layers draw in list order, `name:opacity`. Available: height, hillshade, blocks, contours, biomes, trees, water, waterlevel, flowdir, flow, temperature, moisture, continentalness, roughness, range, features, cells.
- `water` is lake and river in two blues; `waterlevel` bands every block, so a step in a river or lake level shows as a colour change; `cells` draws the 32 block map-cell grid, which is where squared artefacts line up.
- Values for one block: `curl -s 'http://127.0.0.1:8777/info?seed=S&x=X&z=Z'` gives ground height, the map's smooth height ("map bicubic"), top block, water kind, level and depth, and the map cell's lake and river data. Compare ground and bicubic heights to tell detail carved under the smooth ground (hollows, gorges) from the map itself.
- The interactive page at `http://127.0.0.1:8777/?seed=S` has pan, zoom and click-for-info for the user.

## Dig from the command line

`examples/worldprobe.rs` needs no server and prints what chunks are built from, for one block, a line, an ascii grid or a PNG. Prefer it for quick checks; use the viewer for looking round.

```
cargo build --release --example worldprobe
P=target/release/examples/worldprobe
$P <seed> info <x> <z>                  # ground, dry ground, map base, water kind and level, chunk ground_top and water_top, lake level and cover
$P <seed> line <x> <z> <x|z> <len>      # one row a block along an axis: find where a value steps
$P <seed> grid <x> <z> <half>           # ascii: ground (y-400) and water as kind letter (r river, L lake, s sea) plus level
$P <seed> png <x> <z> <half> out.png [pixels_per_block]
```

Real blocks, from the game's `generate_chunk_with`, with the passes laid over the terrain switchable (`--no-cliffs --no-trees --no-flora`), so you can tell which pass made a block:

```
$P <seed> slice <x> <z> <x|z> <len> <y0> <y1> [flags]   # a vertical cross-section, one letter a block (. air, ~ water, # stone, d dirt, g grass, s sand, l leaves, L log)
$P <seed> top <x> <z> <half> out.png [flags]             # top-down surface blocks in colour, 1 pixel a block
$P <seed> passes <x> <z> <half>                          # blocks each pass changes in the area, with examples as (x,y,z) before -> after
```

Render `top` twice, all passes and `--no-cliffs`, to see what the cliff pass does at a shore. Columns and water from `info`, `line`, `grid` are the input to these passes; if the columns are right and `slice` is wrong, a pass is at fault.

It generates the map each run (about 4.5 s), so for many questions about one seed use the viewer's `info` and `export`. `ground` is the finished column, `dry` the ground with no river or lake water on it, `base` the map's smooth height: `ground` far under `base` is detail carved into a hollow, `ground` over `dry` is a bank or valley built up.

## Hunt artefacts

`components.py` (and `diff.py` below) need numpy, scipy and Pillow. `components.py` labels a water-only export's connected parts and lists the small detached ones (specks, slivers), which are the usual leftovers of a shore rule:

```
curl -s 'http://127.0.0.1:8777/export?seed=S&layers=water:1&x=X0&z=Z0&w=2048&h=2048&scale=1' -o w.png
python3 .claude/skills/worldview/components.py w.png X0 Z0
```

Count before and after a change (needs numpy, scipy, Pillow). Then render the worst coordinates zoomed in with `hillshade,water,waterlevel` and `info` them. Wall-like artefacts (water over much lower ground, straight edges) usually follow the map-cell grid: check `cells`, and the ground along a line through the edge.

## Compare a change

Export the view before editing code, rebuild and restart the viewer, export the identical URL again, then diff:

```
python3 .claude/skills/worldview/diff.py before.png after.png diff.png X0 Z0 [scale]
```

For a bare pixel diff without cluster coordinates, ImageMagick does it from the shell (not installed by default, `apt install imagemagick`): `compare -metric AE -fuzz 5% before.png after.png diff.png`.

It prints the changed pixel count and the largest changed clusters with world coordinates, and writes `diff.png` (after dimmed, changes magenta, water that disappeared green). Use it to confirm a fix changed what you meant, and only there. Keep the exports for a few views of the area (the problem spot, a lake, a river mouth) so a regression elsewhere shows. Export both with the same layers, `x`, `z`, `w`, `h` and `scale`.

## Limits

Trees are density only, no individual trees. Cliffs from the chunk cliff pass and snow depth are not drawn in coarse tiles (finer than 2 blocks a pixel they are exact for ground and water, not for the cliff pass's blocks). The viewer builds from the checkout it is run in, so merge or rebase a branch that changes generation before using it to judge that change.
