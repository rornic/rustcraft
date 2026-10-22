#!/usr/bin/env python3
"""Generate 16px seamless block tiles via a running ComfyUI. Usage: generate.py [tile ...] [--seed N] [--ckpt NAME] [--candidates N] [--pick TILE=K]"""
import argparse, functools, io, json, shutil, time
from pathlib import Path

import numpy as np
import requests
from PIL import Image, ImageDraw

from flora import sprite

HERE = Path(__file__).parent
OUT = HERE / "out"
API = "http://127.0.0.1:8188"
COLORS = 4  # per tile; 3 posterises, 6+ brings the noise back
DEFAULT_STD = 14  # contrast for tiles with no reference
MIN_STD = 8  # floor on the reference contrast, so near-flat old tiles (sand, snow) still get some texture
LUM = np.array([0.299, 0.587, 0.114])
STYLE = "seamless tileable texture, extreme close-up of a flat surface filling the whole frame, pixel art, 8-bit, "

# Order matches `Tile` in crates/ochre-base/src/block.rs. Generated tiles take a prompt, `avoid` (negative prompt), and optionally
# `mean` (target sRGB colour, when reference.png has no tile), `std`, `smooth` and `colors` to tune their look.
# grey: target mean sRGB value, the shader tints the tile. derive: built from picked tiles by `derive()`, not generated.
TILES = [
    dict(name="Stone", prompt="rough grey stone, dark specks and cracks, high contrast"),
    dict(name="GrassTop", prompt="grass, short blades, no soil", grey=187),
    dict(name="Sand", prompt="fine pale beach sand"),
    dict(name="Water", prompt="calm blue water, faint ripples"),
    dict(name="Snow", prompt="white snow, faint blue shadows"),
    dict(name="LogBark", prompt="dark brown oak tree bark, deep vertical furrows and ridges, grain running straight up and down", avoid="horizontal stripes, horizontal lines, planks, red, bricks, rings"),
    dict(name="Leaves", prompt="dense leaf clusters", grey=135),
    dict(name="GrassSide", derive="overlay", top="GrassTop", depth=(2, 4)),  # grey grass fringe on transparency, world.wesl layers it over Dirt
    dict(name="Dirt", prompt="dark brown soil, packed earth with a few small pebbles", avoid="orange, red, clay, terracotta, bricks, pink", mean=(140, 98, 68)),
    dict(name="LogEnd", prompt="cut tree trunk end grain, concentric growth rings, light tan wood", avoid="bark, stripes, planks, straight lines, red", mean=(172, 136, 86)),
    dict(name="Gravel", prompt="loose gravel of many small mixed pebbles, grey, brown, tan, white and black specks", avoid="uniform, smooth, flat, single color", colors=7, smooth=0.15, std=22, mean=(138, 132, 126)),
    dict(name="Ice", prompt="pale cyan ice, faint cracks", mean=(150, 205, 235)),
    dict(name="SandstoneSide", derive="strata", body="SandstoneTop"),
    dict(name="SandstoneTop", prompt="pale tan sandstone, smooth fine grain", avoid="orange, red, brown, bricks, terracotta", mean=(214, 192, 148)),
    dict(name="SnowSide", derive="fringe", body="Dirt", top="Snow", depth=(3, 5)),
    dict(name="CompactedStone", derive="shade", body="Stone", factor=0.62, tint=(0.96, 0.98, 1.06)),  # stone packed hard, darker and cooler; breaks only to explosives
    # ground flora, drawn by flora.py; grass and fern are greyscale for the climate's tint
    dict(name="TallGrass", derive="sprite", grey=187),  # as bright as GrassTop, so it tints alike
    dict(name="Fern", derive="sprite", grey=150),
    dict(name="Poppy", derive="sprite"),
    dict(name="Dandelion", derive="sprite"),
    dict(name="ShortGrass", derive="sprite", grey=187),
    dict(name="SeedGrass", derive="sprite", grey=187),
]


def run(wf):
    pid = requests.post(f"{API}/prompt", json={"prompt": wf}).json()["prompt_id"]
    while not (h := requests.get(f"{API}/history/{pid}").json()):
        time.sleep(1)
    img = next(iter(h[pid]["outputs"].values()))["images"][0]
    return requests.get(f"{API}/view", params=dict(filename=img["filename"], subfolder=img["subfolder"], type=img["type"])).content


def build(t, seed, ckpt):
    wf = json.loads((HERE / "block_texture.json").read_text())
    wf["1"]["inputs"]["ckpt_name"] = ckpt
    wf["3"]["inputs"]["text"] = STYLE + t["prompt"]
    wf["6"]["inputs"]["seed"] = seed
    if "avoid" in t:
        wf["4"]["inputs"]["text"] += ", " + t["avoid"]
    return wf


def smooth_wrap(small, smooth):
    """Blend a 16px tile with its 4-neighbour blur, wrapping at the edges so it stays seamless."""
    a = np.asarray(small, float)
    blur = (a + np.roll(a, 1, 0) + np.roll(a, -1, 0) + np.roll(a, 1, 1) + np.roll(a, -1, 1)) / 5
    return a * (1 - smooth) + blur * smooth


def normalise(a, mean, std, mask):
    """Move the tile's mean colour to `mean` and its luminance std to `std`, measured over `mask` pixels."""
    k = std / max((a @ LUM)[mask].std(), 1e-3)
    return mean + (a - a[mask].mean(0)) * k


@functools.cache
def reference_atlas():
    return np.asarray(Image.open(HERE / "reference.png").convert("RGBA"), float)


def reference(name):
    """Mean colour, luminance std and alpha of the tile in reference.png (the look to match), or None if it has no tile there."""
    i = [x["name"] for x in TILES].index(name)
    ref = reference_atlas()
    if ref.shape[1] < (i + 1) * 16:
        return None
    ref = ref[:, i * 16:(i + 1) * 16]
    mask = ref[..., 3] > 127
    return ref[..., :3][mask].mean(0), max((ref[..., :3] @ LUM)[mask].std(), MIN_STD), ref[..., 3]


def finish(t, data, suffix=""):
    (OUT / f"raw_{t['name']}{suffix}.png").write_bytes(data)
    small = Image.open(io.BytesIO(data)).convert("RGB").resize((16, 16), Image.LANCZOS)
    a = smooth_wrap(small, t.get("smooth", 0.5))
    ref = reference(t["name"])
    mean, std, alpha = ref if ref else (None, None, np.full((16, 16), 255.0))
    if "grey" in t:
        a = np.repeat((a @ LUM)[..., None], 3, 2)
        mean = np.full(3, t["grey"])  # shader tints these, so the mean is fixed
    mask = alpha > 127
    if mean is None:  # no reference tile: use the tile's own `mean` (or keep the generated colour), restore contrast
        mean, std = np.array(t["mean"], float) if "mean" in t else a.mean((0, 1)), t.get("std", DEFAULT_STD)
    a = normalise(a, mean, std, mask)
    img = Image.fromarray(np.clip(a.round(), 0, 255).astype("uint8")).quantize(t.get("colors", COLORS), dither=Image.NONE).convert("RGBA")
    img.putalpha(Image.fromarray(alpha.astype("uint8")))
    img.save(OUT / f"{t['name']}{suffix}.png")


def derive(t, seed):
    """Tiles built from already-picked ones: `overlay` (just a ragged fringe, on transparency), `fringe` (over a body tile) or `strata` (darker rows).
    `shade` darkens and tints a body tile. `sprite` tiles are drawn from scratch by flora.py."""
    if t["derive"] == "sprite":
        Image.fromarray(sprite(t["name"], seed, t.get("grey")), "RGBA").save(OUT / f"{t['name']}.png")
        return
    rng = np.random.default_rng(seed)
    load = lambda name: np.asarray(Image.open(OUT / f"{name}.png").convert("RGB"), float)
    if t["derive"] == "shade":
        out = load(t["body"]) * t["factor"] * np.array(t["tint"])
        Image.fromarray(np.clip(out.round(), 0, 255).astype("uint8")).convert("RGBA").save(OUT / f"{t['name']}.png")
        return
    if t["derive"] != "strata":
        top = load(t["top"])
        depth = rng.integers(t["depth"][0], t["depth"][1] + 1, 16)
    if t["derive"] == "overlay":
        top = np.roll(top, (5, 9), (0, 1))  # shifted, so it isn't the top's own pattern
        alpha = (np.arange(16)[:, None] < depth[None, :]) * 255
        rgb = np.where(alpha[..., None] > 0, top, top.mean((0, 1)))  # clear texels keep a grass grey, so mips don't bleed dark into the edge
        Image.fromarray(np.dstack([np.clip(rgb.round(), 0, 255), alpha]).astype("uint8"), "RGBA").save(OUT / f"{t['name']}.png")
        return
    out = load(t["body"])
    if t["derive"] == "strata":
        out = out * np.where(rng.random(16) > 0.6, 0.88, 1.0)[:, None, None]
    else:
        for x in range(16):
            out[:depth[x], x] = top[:depth[x], x]
            out[depth[x], x] = out[depth[x], x] * 0.8  # shaded lip under the fringe
    Image.fromarray(np.clip(out.round(), 0, 255).astype("uint8")).convert("RGBA").save(OUT / f"{t['name']}.png")


def tiled(tile, n, scale):
    """The tile repeated n x n, enlarged with hard pixels, for judging seams by eye."""
    t = Image.fromarray(np.tile(np.asarray(tile.convert("RGBA")), (n, n, 1)))
    return t.resize((t.width * scale, t.height * scale), Image.NEAREST)


def sheet(name, n):
    """Numbered 2x2-tiled strip of out/<name>_c1..n.png, for picking the best candidate by eye."""
    cell = 16 * 2 * 5
    s = Image.new("RGBA", (n * (cell + 8), cell + 16), (30, 30, 30, 255))
    d = ImageDraw.Draw(s)
    for k in range(1, n + 1):
        s.paste(tiled(Image.open(OUT / f"{name}_c{k}.png"), 2, 5), ((k - 1) * (cell + 8), 16))
        d.text(((k - 1) * (cell + 8) + 2, 2), f"{name} #{k}", fill="white")
    s.save(OUT / f"{name}_candidates.png")
    return OUT / f"{name}_candidates.png"


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("tiles", nargs="*", help="tile names, default all (derived tiles are built last)")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--ckpt", default="v1-5-pruned-emaonly.safetensors")
    ap.add_argument("--candidates", type=int, default=1, help="N seeds per tile, saved as out/<Tile>_c<k>.png plus a numbered sheet")
    ap.add_argument("--pick", action="append", default=[], metavar="TILE=K", help="make candidate K the tile's output, e.g. --pick LogBark=3")
    a = ap.parse_args()
    OUT.mkdir(exist_ok=True)
    for p in a.pick:
        name, k = p.split("=")
        shutil.copy(OUT / f"{name}_c{k}.png", OUT / f"{name}.png")
        shutil.copy(OUT / f"raw_{name}_c{k}.png", OUT / f"raw_{name}.png")
        print("picked", p)
    if a.pick:
        raise SystemExit
    want = a.tiles or [t["name"] for t in TILES]
    todo = sorted((t for t in TILES if t["name"] in want), key=lambda t: "derive" in t)  # derived tiles need their sources first
    for t in todo:
        if "derive" in t:
            derive(t, a.seed)
        else:
            for k in range(1, a.candidates + 1):
                finish(t, run(build(t, a.seed + k - 1, a.ckpt)), suffix="" if a.candidates == 1 else f"_c{k}")
            if a.candidates > 1:
                print("candidates:", sheet(t["name"], a.candidates))
        print("done", t["name"])
