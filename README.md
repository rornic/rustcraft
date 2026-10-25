# ochre

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Bevy](https://img.shields.io/badge/bevy-0.20-232326)](https://bevy.org/)
[![Rust](https://img.shields.io/badge/rust-1.97.1%2B-orange)](https://rustup.rs/)

A 64 km voxel world with erosion-carved rivers, lakes and seas, streamed with octree LODs out to 32km.

https://github.com/user-attachments/assets/b5bac704-e812-4367-990b-0ce3b0a5579f

## Features

- **64 km x 64 km world.** Generated from a seed in a few seconds. Droplet erosion carves valleys, hydrology routes rivers to lakes and the sea, and a climate pass sets temperature and moisture.
- **Biomes.** Desert, forest, plains, glacier and mountain, with snow caps, cliffs, trees and flora placed per biome.
- **Octree level of detail.** Terrain streams in around the player and falls back to coarser meshes with a cross-fade, so the curved horizon reaches 16 km at the default render distance.
- **Water that behaves.** Lakes, rivers and seas sit at consistent levels and don't leak. Water flows into space you open beside it.
- **Sky.** A day and night cycle with sun, moon and stars, atmospheric scattering, volumetric clouds and weather including rain.
- **Atmospheric audio.** Wind that follows altitude, plus ambient loops for forest, desert, crickets, owls, waves, leaves, rain and underwater.
- **Life.** Birds and ground critters.

## Run it

A simple sandbox example is included to showcase atmospheric audio, foliage, water, critters and basic weather.

```
cargo run --release --example sandbox
```

## History

I started ochre, originally named rustcraft, back in 2021 as an experiment to build a Minecraft clone using Rust, [specs](https://github.com/amethyst/specs) and [glium](https://github.com/glium/glium). [Read more here about that early version](https://rornic.dev/posts/a-minecraft-experiment-with-rust-and-opengl/).

In 2023, I migrated the project over to [Bevy](https://bevy.org/) because I wanted to push render distance out further and got bored of fiddling around with a custom rendering pipeline.

Then I abandoned the project for a while, before coming back in 2026 with renewed interest and Claude Code at my disposal to massively speed up delivery of new features. Using Claude, I've been able to complete my unfinished octree level of detail (LOD) implementation, revamp terrain generation and generally beautify the rendering in quite a short period of time.

## License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Issues for bugs and ideas are welcome. Please open an issue to discuss any non-trivial change **before** opening a pull request; large or unsolicited PRs will likely be closed. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
