#!/usr/bin/env python3
"""Pack out/<Tile>.png into assets/textures/blocks.png (missing tiles keep their current art), rewrite block_mean in
block_texture.wesl to match, and write out/contact.png with each tile tiled 3x3."""
import re

import numpy as np
from PIL import Image

from generate import HERE, OUT, TILES, tiled

ATLAS = HERE / "../../../assets/textures/blocks.png"
SHADER = HERE / "../../../assets/shaders/block_texture.wesl"

atlas = Image.open(ATLAS).convert("RGBA")
assert atlas.size == (16 * len(TILES), 16), f"atlas is {atlas.size}, TILES in generate.py out of sync with Tile enum"
OUT.mkdir(exist_ok=True)
sheet = Image.new("RGBA", (3 * 16 * 3 * len(TILES), 3 * 16 * 3))
for i, t in enumerate(TILES):
    p = OUT / f"{t['name']}.png"
    if p.exists():
        atlas.paste(Image.open(p).convert("RGBA"), (i * 16, 0))
    sheet.paste(tiled(atlas.crop((i * 16, 0, i * 16 + 16, 16)), 3, 3), (i * 144, 0))
atlas.save(ATLAS)
sheet.save(OUT / "contact.png")


def linear(v):
    v = v / 255
    return np.where(v <= 0.04045, v / 12.92, ((v + 0.055) / 1.055) ** 2.4)


# mean linear colour of each cell's drawn texels (alpha >= 128), as material.rs's test_block_means_match_the_atlas
cells = [np.asarray(atlas, float)[:, i * 16:(i + 1) * 16].reshape(-1, 4) for i in range(len(TILES))]
means = iter([linear(c[c[:, 3] >= 128, :3]).mean(0) for c in cells])
rewritten = 0


def mean_row(m):
    global rewritten
    rewritten += 1
    return "vec3<f32>(%.4f, %.4f, %.4f)" % tuple(next(means))


lines = [re.sub(r"vec3<f32>\([^)]*\)", mean_row, l, count=1) if l.strip().startswith("vec3<f32>(") else l for l in SHADER.read_text().splitlines()]
assert rewritten == len(TILES), f"block_texture.wesl has {rewritten} vec3 rows, expected one per tile"
text = "\n".join(lines) + "\n"
if text != SHADER.read_text():  # an untouched shader keeps its mtime, so it isn't hot-reloaded for nothing
    SHADER.write_text(text)
print("packed; seam check:", OUT / "contact.png")
