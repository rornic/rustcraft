# rustcraft

A voxel survival game in Rust and [Bevy](https://bevy.org), where nature is the only enemy and you can see for miles.

It began as a Minecraft clone and is turning into something harsher: the player against the elements, not a creative sandbox. There are no monsters and no strip mining. The threats are cold, wet, steep ground, storms and fire.

```
cargo run --release
```

On Linux, Bevy's audio and input need ALSA and udev headers (`libasound2-dev` and `libudev-dev` on Debian and Ubuntu).

![Image of rustcraft](images/readme.jpg)

## What's there

- Terrain drawn out to a curved horizon, with LODs, continents, biomes and trees.
- A day and night cycle with a moon and stars, clouds lit by the sun, and water that follows the wind.
- Storms with rain, lightning, thunder and rainbows, running on the game clock.
- Ambient sound, ground flora swaying in the wind, insects and birds.

## Where it's going

- **Systems that interact.** Materials burn, rot and collapse. Fire spreads by material, wind and dryness, and rain puts it out.
- **Seasons and climate.** Gather in summer, prepare in autumn, survive winter. Body temperature, wetness and injuries per body part.
- **Distance as a cost.** You can't carry much. Moving goods in bulk takes carts, pack animals and boats, so every long trip is an expedition.
- **A persistent world.** Unkept buildings decay into ruins, iron is scarce and shared, and raiding is a slow, planned siege rather than a quick fight.

## License

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Issues for bugs and ideas are welcome. Please open an issue to discuss any non-trivial change **before** opening a pull request; large or unsolicited PRs will likely be closed. See [CONTRIBUTING.md](CONTRIBUTING.md) for details.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
