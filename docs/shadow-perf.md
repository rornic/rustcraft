# Shadow pass performance

Findings from reading Bevy 0.20's shadow path and counting what the sun's cascades are handed. No GPU was available, so every number below is a CPU-side submission count (triangles in meshes that pass each cascade's frustum), not a GPU time. The A/B list at the end settles which costs dominate on a real card.

Tool: `cargo run --release --example shadow_cull -- [shadow distance in blocks] [cascades] [camera pitch]`. It loads the start area headless, builds the cascades with Bevy's own `build_directional_light_cascades` for 15 sun positions (5 to 85 degrees up, ahead, beside and behind the camera), and applies `check_dir_light_mesh_visibility`'s test to every terrain mesh. Seed 1, spawn in jungle, 90 degree vertical fov, 16:9. Build with `--ignore-rust-version` if the toolchain is 1.97.0.

## Implemented options 1 and 3

`ochre-world::shadow::prune_shadow_casters` runs after Bevy's light visibility check. Each cascade
keeps terrain intersecting its camera slice swept indefinitely toward the fitted shadow sun.
The unbounded sweep also covers very low sunlight and casters above the configured world height.
Camera-relative plane offsets avoid precision loss far from the origin. The hull is expanded by
four shadow texels, the light's receiver biases, and 0.3 blocks for foliage sway. Unknown casters
and explicit culling opt-outs keep Bevy's result. When `WORLD_CURVE.is_flat_at` becomes false
(100 blocks above sea), the system keeps Bevy's whole list.

Terrain meshes are split before GPU packing, after the existing generation and group merge.
Leaf holes and flora go to a cutout child mesh; ground and already-solid coarse leaves use a
`non_cutout` material. This classification survives camera fade material swaps. Its orthographic
shadow pipeline omits the fragment stage on GPUs with native depth-clip control. GPUs requiring
`UNCLIPPED_DEPTH_ORTHO_EMULATION` retain the fragment depth write. Camera passes remain masked,
including with `solid_leaves`, so their LOD fade still discards the same pixels. The material
binding remains necessary for vertex curvature and sway. This optimization removes fragment
work, not the material binding. Splitting adds a draw for mixed ground/foliage meshes; splitting
and packing both run on the terrain worker tasks, including LOD tasks.

Headless check on 2026-10-26, `OCHRE_SEED=1 cargo run --release --example shadow_cull -- 512 3 0`,
with the current settings (16 full-detail chunks, LOD base 16) and the cached seed-1 map:

| Mean over 15 sun positions | Triangles |
|---|---:|
| Camera view | 1.799M |
| Bevy shadow visibility | 2.365M |
| After swept hull | 1.083M (54% fewer) |
| Retained geometry eligible for no fragment stage | 1.054M (97.3%) |

These are submission counts, not GPU timings. They use the runtime hull implementation and include
cutout children and visible inner leaves. They are not directly comparable with the earlier jungle
counts below, which used different settings and world data. GPU frame-time p99 and visual checks
of low sun, LOD swaps, and the curvature transition remain required on a rendered run.

`bench` generation vertex counts match exactly before and after: 620,695 chunk vertices and 311,077
LOD vertices, with every level unchanged. Tests cover the split's attributes and triangle winding,
solid coarse leaves, fade classification, native/emulated shadow specialization, swept bounds,
and the runtime curved-world fallback. The prepass compiles through Bevy's WESL processor for
both material classes in camera, native-shadow and emulated-shadow configurations.

The settings test now matches the checked-in full-detail distance of 16 chunks and covers the
new cloud resolution presets. All 353 workspace unit tests, doc tests and release Clippy pass.
`lod_sim` at seed 1 flags one persistent surface chunk after stopping on
both the original branch code and the changed code; it is an existing coverage issue, not
evidence of a new gap from this split.

## Original baseline

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

These are the original proposals. Options 1 and 3 are implemented as described above.

1. **Swept-hull caster culling (CPU only, no visual change).** A caster matters only if it lies in the hull of the cascade's view slice and that slice swept toward the sun up to `WORLD_HEIGHT`. Pruning `CascadesVisibleEntities` after `SimulationLightSystems::CheckLightVisibility` with it cut shadow triangles 5.6M to 2.8M (1.07x the view). It cut 43 to 51% at pitch -35, 0 and 20. Cascade 1 loses most (1.58M to 0.61M). Needs a fallback to Bevy's list when the curve bends the land (camera over about 100 blocks above sea, see `curvature.rs`), as flat AABBs stop matching drawn positions there. Low sun sweeps far, so the gain shrinks at 5 degrees (about 20%).
2. **Settings, visual trade.** Shadow distance 64 to 32 chunks: 4.2M (1.6x), hull 2.2M. 24 chunks: 3.7M. Shadows already fade from 80% of reach, so the loss is far hill shadows. Fewer cascades hurt near shadows (a 2 cascade split puts 2 block texels at 20 blocks). Combine with 1 for 512 blocks and 3 cascades at 0.84x the view.
3. **Drop the fragment stage for non-cutout geometry.** `ChunkMaterial` is `AlphaMode::Mask` with custom prepass shaders, so every shadow draw runs the fragment shader (discard, late depth write) and binds the material, even for ground. `prepass_reads_material()` rules out Bevy's depth-only fast path, but `specialize` can set `descriptor.fragment = None` for orthographic views on a material flagged non-cutout. That needs the mesher to split leaves and flora into their own mesh (`inner_leaves` already shows the pattern) and fading to stay `Mask` in the camera prepass. Bench vertex totals must not change.
4. **Cascade-aware shadow proxies or caching.** Cascade 2 could draw coarse LOD geometry instead of the full-detail disc, using render layers (light sees layers 0 and 1, camera only 0) and a per-cascade filter on the visible lists. Re-rendering cascade 2 only every few frames needs a patched `bevy_pbr` shadow node. Both are large. Try 1 to 3 first.
5. **Smaller things.** Fading-out meshes could be `NotShadowCaster` once shadows stop dithering (see below), saving the doubled geometry during the 1.5 s swap. `DirectionalLightShadowMap::size` is not exposed. Dropping it to 1024 cuts shadow fill 4x if fill matters.

## Fade guard correction

The fade dither and depth push now use `!VIEW_PROJECTION_ORTHOGRAPHIC`. Their previous guard,
`!UNCLIPPED_DEPTH_ORTHO_EMULATION`, disabled them only on GPUs requiring depth-clamp emulation,
so native desktop shadow views also dithered and pushed their depth. This correction keeps
camera fading while allowing non-cutout shadows to omit their fragment stage safely.

## A/B on a GPU

Read the per pass GPU ms in the debug overlay (`OCHRE_STATS=1`). Change one thing at a time in `assets/settings.toml`:

- `shadow_cascades = 2`, `shadow_distance = 32`: how far geometry scales the cost.
- `[debug] solid_leaves = true`: removes leaf holes, to isolate alpha-cutout cost (flora still has holes).
- `still_leaves = true`: vertex sway cost, expected small.

If `solid_leaves` moves the shadow pass little, the pass is geometry bound and options 1 and 2 are the work. If it moves it a lot, option 3 comes first.
