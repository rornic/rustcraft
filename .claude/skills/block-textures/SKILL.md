---
name: block-textures
description: Generate or regenerate Ochre's 16px seamless block textures (stone, grass, logs, side variants) with ComfyUI, and pack them into assets/textures/blocks.png. Use when adding, restyling or fixing a block texture.
---

# Block textures

The atlas `assets/textures/blocks.png` is one 16px-tall strip. Tile order is `enum Tile` in `crates/ochre-base/src/block.rs` and must match `TILES` in `generate.py`. `pack.py` asserts the length.

## One-time setup

Target GPU is a 6GB RTX 2060, so SD1.5.

1. `git clone https://github.com/comfyanonymous/ComfyUI ~/ComfyUI`, then `python3 -m venv --system-site-packages ~/ComfyUI/venv` and `~/ComfyUI/venv/bin/pip install -r ~/ComfyUI/requirements.txt`. Start with `~/ComfyUI/venv/bin/python ~/ComfyUI/main.py` (serves :8188).
2. Download `v1-5-pruned-emaonly.safetensors` from the `stable-diffusion-v1-5/stable-diffusion-v1-5` HF repo into `~/ComfyUI/models/checkpoints/` (or pass `--ckpt`).
3. `git clone https://github.com/spinagon/ComfyUI-seamless-tiling ~/ComfyUI/custom_nodes/ComfyUI-seamless-tiling` (`SeamlessTile`, `CircularVAEDecode`).
4. `pip install pillow requests numpy` (system python is fine).

On Windows it runs natively (no WSL needed): the venv's python is `venv\Scripts\python.exe`, and torch wants the CUDA 13 wheels, `pip install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu130`, for 20-series cards and newer.

## Generate

```
cd .claude/skills/block-textures
python generate.py                 # all tiles, seed 1
python generate.py Stone Dirt --seed 7
python pack.py                     # writes the atlas, the block_mean table and out/contact.png
```

Pick the best of several seeds. `python generate.py LogBark --candidates 4` writes `out/LogBark_c1..4.png` and a numbered `out/LogBark_candidates.png`. Look at the sheet, then `python generate.py --pick LogBark=1` makes that one the tile. Claude can do the looking itself by reading the sheet image.

Per-tile `avoid` in `TILES` adds negative prompt terms (the bark one bans horizontal stripes, which SD otherwise loves).

Open `out/contact.png` (each tile repeated 3x3, 9x zoom) and check seams. Re-roll bad tiles with another `--seed`. `out/` is scratch, not committed.

## How it works

- `block_texture.json` is the txt2img graph. SD1.5 renders 512px with circular padding in both axes so the result tiles. `generate.py` saves the render as `out/raw_<Tile>.png` and `finish()` turns it into 16x16: Lanczos down, wrap-around blur (`smooth`, keeps seams), normalise mean colour and luminance contrast, then a 4-colour quantise (`COLORS`). Plain `area` averaging gives flat grey, so don't use it.
- Normalisation targets come from `reference.png`, the hand-drawn 7-tile atlas from before the procedural generator (commit 1d2587c), not the live atlas, because `pack.py` overwrites that. Its alpha also supplies the leaf holes, since SD can't draw cut-outs. A tile past the reference's 7 can set `mean=(r, g, b)` in `TILES` (sandstone, log end, ice, dirt and gravel do) and `std`, or it keeps the colour SD produced with `DEFAULT_STD` contrast.
- `GrassTop` and `Leaves` are greyscale and tinted per climate by the shader, which assumes mean brightness 187 and 135 (sRGB), so `grey=` fixes their mean.
- Flora tiles (`derive="sprite"`, the atlas's last six) aren't AI-generated either: SD can't draw a lone plant on transparency at 16px. `flora.py` draws them pixel by pixel, greyscale grasses and fern levelled by `grey=` for the climate tint, flowers in colour. The shaders treat every tile from `Tile::FIRST_FLORA` on as a sprite, so new flora goes on the end.
- Side tiles are not AI-generated, since SD can't draw a fringe. `derive()` builds them from the picked tiles, so after picking new sources rerun `generate.py GrassSide SnowSide SandstoneSide`. `GrassSide` is `overlay`: only a greyscale grass fringe on transparency (shifted GrassTop pixels, 2-4 rows deep), which `world.wesl` layers over the Dirt tile by its alpha, tinting just the grass, so changing Dirt changes grass sides too. `SnowSide` is `fringe`: a snow fringe baked over Dirt. `SandstoneSide` is `strata`: darker rows over SandstoneTop.
- `pack.py` rewrites the `block_mean` table in `block_texture.wesl`, which `material.rs` tests against the atlas.
- Don't add the Plixel Minecraft LoRA. It was trained on sprites, so it draws a sprite on a white or checkerboard background and ignores tiling. At 512px it only produces dither noise.
- Prompts must not say "minecraft", "top-down" or "orthographic". SD reads them as an isometric scene. `STYLE` asks for a flat close-up instead, and the negative prompt bans landscapes and trees.
- Known weak spot: Leaves candidates all look the same, because the holes come from the reference alpha.

## Add a block texture

1. Add the variant to `enum Tile` and bump `TILE_COUNT` in `crates/ochre-base/src/block.rs`, and use it in `BlockType::tile`.
2. Append a row to `TILES` in `generate.py` at the same index.
3. In `block_texture.wesl`, bump `ATLAS_CELLS` and add a `vec3` row to `block_mean` (`pack.py` asserts one row per tile and fills the numbers in).
4. Widen the atlas first (`pack.py` asserts 16px per row entry), e.g. paste a blank 16px onto the right edge, then generate and pack.
