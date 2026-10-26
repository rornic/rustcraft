# Cloud resolution

Set `cloud_resolution` in the `[renderer]` section of `assets/settings.toml`:

```toml
[renderer]
cloud_resolution = "medium"
```

| Preset | Maximum target width |
|---|---:|
| `low` | 320 |
| `medium` (default) | 640 |
| `high` | 960 |
| `ultra` | 1280 |

Height follows the window's aspect ratio. At 16:9, medium marches into a 640×360 target at
1080p, 1440p, and 4K. Smaller windows retain half display resolution rather than upscaling
their cloud target. Ultrawide and portrait windows keep their aspect ratio, so their pixel
counts differ at the same preset.

The march target and temporal history use the same calculation on startup and window resize.
Temporal accumulation and the existing linear sampler blend the clouds up to display resolution.
Restart after changing the setting. Reflection-map resolution and march step counts are separate
and stay fixed.
