# Shadow pass performance

Findings from reading Bevy 0.20's shadow path and counting what the sun's cascades are handed. No GPU was available, so every number below is a CPU-side submission count (triangles in meshes that pass each cascade's frustum), not a GPU time. The A/B list at the end settles which costs dominate on a real card.

Tool: `cargo run --release --example shadow_cull -- [shadow distance in blocks] [cascades] [camera pitch]`. It loads the start area headless, builds the cascades with Bevy's own `build_directional_light_cascades` for 15 sun positions (5 to 85 degrees up, ahead, beside and behind the camera), and applies `check_dir_light_mesh_visibility`'s test to every terrain mesh. Seed 1, spawn in jungle, 90 degree vertical fov, 16:9. Build with `--ignore-rust-version` if the toolchain is 1.97.0.

## What the pass is handed

Defaults (1024 blocks, 3 cascades, 2048 maps): bounds 20 / 143 / 1024, texels 0.04 / 0.29 / 2.04 blocks.

| | triangles (mean over 15 suns) |
|---|---|
| camera view (drawn twice, depth prepass and main) | 2.6M |
| cascade 0 | 0.10M (61% leaves or flora) |
| cascade 1 | 1.58M (50%) |
| cascade 2 | 3.96M (24%) |
| shadows in all | 5.6M, 2.2x the view |

- Cascade 2's light-space square is 4177 blocks wide (the far plane's diagonal), over 9x the area of the slice it serves. It always contains the whole full-detail disc (1.96M triangles, every triangle of it) plus LOD levels 1 to 4, much of it behind or beside the camera.
- At 2.04 blocks a texel, everything drawn at full detail or level 1 in cascade 2 is under a texel across. About 2.4M of its 4.4M triangles (side sun) add almost nothing.
- Near cascades are mostly leaves and flora, which are alpha tested.
- Cascade count costs a lot: 2 cascades 4.1M, 3 cascades 5.6M, 4 cascades 7.1M.
- Bevy culls casters against each cascade's box with near-plane culling off, so nothing limits them to what can shadow the view.

## Options, best first

1. **Swept-hull caster culling (CPU only, no visual change).** A caster matters only if it lies in the hull of the cascade's view slice and that slice swept toward the sun up to `WORLD_HEIGHT`. Pruning `CascadesVisibleEntities` after `SimulationLightSystems::CheckLightVisibility` with it cut shadow triangles 5.6M to 2.8M (1.07x the view). It cut 43 to 51% at pitch -35, 0 and 20. Cascade 1 loses most (1.58M to 0.61M). Needs a fallback to Bevy's list when the curve bends the land (camera over about 100 blocks above sea, see `curvature.rs`), as flat AABBs stop matching drawn positions there. Low sun sweeps far, so the gain shrinks at 5 degrees (about 20%).
2. **Settings, visual trade.** Shadow distance 64 to 32 chunks: 4.2M (1.6x), hull 2.2M. 24 chunks: 3.7M. Shadows already fade from 80% of reach, so the loss is far hill shadows. Fewer cascades hurt near shadows (a 2 cascade split puts 2 block texels at 20 blocks). Combine with 1 for 512 blocks and 3 cascades at 0.84x the view.
3. **Drop the fragment stage for non-cutout geometry.** `ChunkMaterial` is `AlphaMode::Mask` with custom prepass shaders, so every shadow draw runs the fragment shader (discard, late depth write) and binds the material, even for ground. `prepass_reads_material()` rules out Bevy's depth-only fast path, but `specialize` can set `descriptor.fragment = None` for orthographic views on a material flagged non-cutout. That needs the mesher to split leaves and flora into their own mesh (`inner_leaves` already shows the pattern) and fading to stay `Mask` in the camera prepass. Bench vertex totals must not change.
4. **Cascade-aware shadow proxies or caching.** Cascade 2 could draw coarse LOD geometry instead of the full-detail disc, using render layers (light sees layers 0 and 1, camera only 0) and a per-cascade filter on the visible lists. Re-rendering cascade 2 only every few frames needs a patched `bevy_pbr` shadow node. Both are large. Try 1 to 3 first.
5. **Smaller things.** Fading-out meshes could be `NotShadowCaster` once shadows stop dithering (see below), saving the doubled geometry during the 1.5 s swap. `DirectionalLightShadowMap::size` is not exposed. Dropping it to 1024 cuts shadow fill 4x if fill matters.

## Likely bug

`world_prepass.wesl` guards the fade dither and depth push with `!UNCLIPPED_DEPTH_ORTHO_EMULATION`, and the comment says shadows are not dithered. That define is set only on GPUs lacking depth clip control, so on desktop GPUs shadow views dither and push too, and shadows of a fading mesh flicker. `VIEW_PROJECTION_ORTHOGRAPHIC`, already used in `cut_leaf_holes`, is the condition that matches the intent. Check by flying through a LOD swap with the sun low.

## A/B on a GPU

Read the per pass GPU ms in the debug overlay (`OCHRE_STATS=1`). Change one thing at a time in `assets/settings.toml`:

- `shadow_cascades = 2`, `shadow_distance = 32`: how far geometry scales the cost.
- `[debug] solid_leaves = true`: no fragment stage in shadow draws, the upper bound for option 3.
- `still_leaves = true`: vertex sway cost, expected small.

If `solid_leaves` moves the shadow pass little, the pass is geometry bound and options 1 and 2 are the work. If it moves it a lot, option 3 comes first.
