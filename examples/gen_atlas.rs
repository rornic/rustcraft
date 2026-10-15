// Generates assets/textures/blocks.png: `cargo run --example gen_atlas [-- contact_sheet.png]`.
// Prints each tile's mean linear colour as `block_mean` lines for block_texture.wesl.
use image::{Rgba, RgbaImage};

const N: usize = 16;
const SHEET_SCALE: u32 = 8;

type Px = [u8; 4];
type Tile = Vec<Px>;

// Atlas order, matching `Tile` in src/block.rs.
const NAMES: [&str; 15] = [
    "stone",
    "grass top",
    "sand",
    "water",
    "snow",
    "log bark",
    "leaves",
    "grass side",
    "dirt",
    "log end",
    "gravel",
    "ice",
    "sandstone side",
    "sandstone top",
    "snow side",
];

fn hash(seed: u32, x: i32, y: i32) -> f32 {
    let mut h = seed.wrapping_mul(0x9E37_79B1)
        ^ (x as u32).wrapping_mul(0x85EB_CA6B)
        ^ (y as u32).wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

// Value noise with `px` x `py` lattice cells across the tile, wrapping so the tile repeats seamlessly.
fn vnoise(seed: u32, x: f32, y: f32, px: i32, py: i32) -> f32 {
    let (u, v) = (x / N as f32 * px as f32, y / N as f32 * py as f32);
    let (iu, iv) = (u.floor(), v.floor());
    let (fu, fv) = (smooth(u - iu), smooth(v - iv));
    let at = |dx: i32, dy: i32| {
        hash(
            seed,
            (iu as i32 + dx).rem_euclid(px),
            (iv as i32 + dy).rem_euclid(py),
        )
    };
    let top = at(0, 0) + (at(1, 0) - at(0, 0)) * fu;
    let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * fu;
    top + (bottom - top) * fv
}

fn fbm(seed: u32, x: f32, y: f32, base: i32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut total) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        let period = base << o;
        sum += amp * vnoise(seed.wrapping_add(o * 101), x, y, period, period);
        total += amp;
        amp *= 0.5;
    }
    sum / total
}

// Distances to the nearest and second nearest feature point, in cells, and the nearest one's id; wraps like `vnoise`.
fn worley(seed: u32, x: f32, y: f32, n: i32) -> (f32, f32, f32) {
    let (u, v) = (x / N as f32 * n as f32, y / N as f32 * n as f32);
    let (ci, cj) = (u.floor() as i32, v.floor() as i32);
    let (mut f1, mut f2, mut id) = (f32::MAX, f32::MAX, 0.0);
    for dj in -1..=1 {
        for di in -1..=1 {
            let (cx, cy) = (ci + di, cj + dj);
            let (wx, wy) = (cx.rem_euclid(n), cy.rem_euclid(n));
            let p = (
                cx as f32 + hash(seed, wx, wy),
                cy as f32 + hash(seed ^ 0x55, wx, wy),
            );
            let d = ((p.0 - u).powi(2) + (p.1 - v).powi(2)).sqrt();
            if d < f1 {
                (f2, f1, id) = (f1, d, hash(seed ^ 0xA7, wx, wy));
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2, id)
}

// Hard steps through a palette, which is what keeps the tiles pixel-arty.
fn ramp(t: f32, colours: &[[u8; 3]]) -> [u8; 3] {
    colours[((t.clamp(0.0, 0.999)) * colours.len() as f32) as usize]
}

fn lerp3(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    std::array::from_fn(|i| {
        (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t.clamp(0.0, 1.0)).round() as u8
    })
}

fn opaque(c: [u8; 3]) -> Px {
    [c[0], c[1], c[2], 255]
}

fn build(f: impl Fn(f32, f32, i32, i32) -> Px) -> Tile {
    (0..N * N)
        .map(|i| {
            f(
                (i % N) as f32 + 0.5,
                (i / N) as f32 + 0.5,
                (i % N) as i32,
                (i / N) as i32,
            )
        })
        .collect()
}

fn stone(seed: u32) -> Tile {
    const GREYS: [[u8; 3]; 6] = [
        [68, 70, 75],
        [80, 82, 87],
        [92, 94, 99],
        [103, 105, 110],
        [114, 116, 121],
        [126, 128, 133],
    ];
    build(|x, y, ix, iy| {
        let (warp_x, warp_y) = (
            x + (fbm(seed + 2, x, y, 2, 2) - 0.5) * 5.0,
            y + (fbm(seed + 3, x, y, 2, 2) - 0.5) * 5.0,
        );
        let (f1, f2, _) = worley(seed + 4, warp_x, warp_y, 3);
        let broken = fbm(seed + 5, x, y, 4, 2) > 0.45;
        let speck = hash(seed + 1, ix, iy);
        if f2 - f1 < 0.06 && broken {
            return opaque([52, 54, 59]);
        }
        let mut t = 0.2 + 0.55 * fbm(seed, x, y, 2, 3) + (speck - 0.5) * 0.3;
        if speck > 0.95 {
            t += 0.3;
        } else if speck < 0.04 {
            t -= 0.3;
        }
        opaque(ramp(t, &GREYS))
    })
}

// Greyscale, tinted by the vertex colour. `gain` brightens it so the mean can be fitted.
fn grass_top(seed: u32, gain: f32) -> Tile {
    const GREYS: [f32; 7] = [142.0, 160.0, 174.0, 186.0, 198.0, 210.0, 224.0];
    build(|x, y, ix, iy| {
        // blades are short vertical runs, offset a little per column
        let run = ((y + hash(seed, ix, 0) * 4.0) / 2.5).floor() as i32;
        let blade = hash(seed + 1, ix, run.rem_euclid(7));
        let patch = fbm(seed + 2, x, y, 2, 3);
        let t = 0.45 * patch + 0.45 * blade + 0.05 + (hash(seed + 3, ix, iy) - 0.5) * 0.1;
        let v = GREYS[(t.clamp(0.0, 0.999) * GREYS.len() as f32) as usize] * gain;
        let g = v.clamp(0.0, 255.0).round() as u8;
        [g, g, g, 255]
    })
}

fn dirt_at(seed: u32, x: f32, y: f32, ix: i32, iy: i32) -> [u8; 3] {
    const BROWNS: [[u8; 3]; 6] = [
        [84, 58, 38],
        [98, 69, 45],
        [112, 80, 52],
        [126, 91, 60],
        [138, 102, 70],
        [150, 113, 79],
    ];
    let (f1, _, id) = worley(seed + 9, x, y, 6);
    let speck = hash(seed + 1, ix, iy);
    if f1 < 0.2 && id > 0.55 {
        return if id > 0.8 {
            [150, 142, 130]
        } else {
            [120, 112, 104]
        };
    }
    let mut t = 0.15 + 0.6 * fbm(seed, x, y, 2, 3) + (speck - 0.5) * 0.3;
    if speck < 0.05 {
        t -= 0.3;
    }
    ramp(t, &BROWNS)
}

fn dirt(seed: u32) -> Tile {
    build(|x, y, ix, iy| opaque(dirt_at(seed, x, y, ix, iy)))
}

fn grass_side(seed: u32) -> Tile {
    const GREENS: [[u8; 3]; 4] = [[52, 110, 30], [64, 128, 36], [78, 144, 44], [92, 158, 52]];
    build(|x, y, ix, iy| {
        let fringe = 3.0
            + (hash(seed, ix, 0) * 3.0).floor()
            + if hash(seed + 1, ix, 1) > 0.7 {
                1.0
            } else {
                0.0
            };
        if y < fringe {
            let t = 0.4 * (y / fringe) + 0.6 * hash(seed + 2, ix, iy);
            let c = ramp(1.0 - t, &GREENS);
            // the fringe's last row picks up a little dirt
            return opaque(if y > fringe - 1.0 {
                lerp3(c, dirt_at(seed, x, y, ix, iy), 0.4)
            } else {
                c
            });
        }
        let shade = if y < fringe + 1.5 { 0.82 } else { 1.0 };
        opaque(dirt_at(seed, x, y, ix, iy).map(|c| (c as f32 * shade) as u8))
    })
}

fn sand(seed: u32) -> Tile {
    const SANDS: [[u8; 3]; 5] = [
        [168, 146, 106],
        [176, 154, 112],
        [184, 161, 118],
        [192, 169, 126],
        [200, 177, 134],
    ];
    build(|x, y, ix, iy| {
        // faint wind ripples, long in x
        let ripple = vnoise(seed, x, y, 2, 6);
        let t = 0.15
            + 0.4 * ripple
            + 0.3 * fbm(seed + 1, x, y, 4, 2)
            + (hash(seed + 2, ix, iy) - 0.5) * 0.35;
        opaque(ramp(t, &SANDS))
    })
}

fn water(seed: u32) -> Tile {
    const BLUES: [[u8; 3]; 4] = [[62, 78, 236], [70, 86, 242], [78, 96, 246], [88, 108, 250]];
    build(|x, y, _, _| {
        let t = fbm(seed, x, y, 2, 3) * 0.8 + vnoise(seed + 1, x, y, 8, 3) * 0.3;
        opaque(ramp(t - 0.1, &BLUES))
    })
}

fn snow(seed: u32) -> Tile {
    const WHITES: [[u8; 3]; 4] = [
        [226, 238, 248],
        [238, 247, 253],
        [248, 253, 255],
        [255, 255, 255],
    ];
    build(|x, y, ix, iy| {
        let t = 0.4 + 0.7 * fbm(seed, x, y, 4, 2) + (hash(seed + 1, ix, iy) - 0.5) * 0.3;
        let (f1, _, id) = worley(seed + 2, x, y, 5);
        // faint dimples where the snow packed down
        if f1 < 0.3 && id > 0.7 {
            return opaque([222, 235, 247]);
        }
        opaque(ramp(t, &WHITES))
    })
}

fn snow_side(seed: u32) -> Tile {
    const WHITES: [[u8; 3]; 4] = [
        [222, 234, 246],
        [234, 244, 252],
        [246, 252, 255],
        [255, 255, 255],
    ];
    build(|x, y, ix, iy| {
        // packed layers, a faint band every few rows
        let layer = ((y / 16.0 * 4.0 * std::f32::consts::TAU).sin() * 0.5 + 0.5) * 0.25;
        let t = 0.45 + 0.6 * fbm(seed, x, y, 4, 2) - layer + (hash(seed + 1, ix, iy) - 0.5) * 0.3;
        let speck = hash(seed + 2, ix, iy);
        if speck > 0.97 {
            return opaque([170, 188, 206]);
        }
        opaque(ramp(t, &WHITES))
    })
}

fn log_side(seed: u32) -> Tile {
    const BARKS: [[u8; 3]; 6] = [
        [40, 29, 20],
        [52, 38, 26],
        [64, 47, 32],
        [78, 58, 38],
        [92, 69, 45],
        [106, 80, 52],
    ];
    build(|x, y, ix, iy| {
        // ridges run up the trunk, so noise stretches in y
        let ridge = vnoise(seed, x, y, 8, 2);
        let t =
            0.7 * ridge + 0.25 * fbm(seed + 1, x, y, 4, 2) + (hash(seed + 2, ix, iy) - 0.5) * 0.2;
        let gap = hash(seed + 3, ix, 0) > 0.82 && vnoise(seed + 4, x, y, 2, 4) > 0.35;
        opaque(ramp(if gap { 0.0 } else { t }, &BARKS))
    })
}

fn log_end(seed: u32) -> Tile {
    const WOODS: [[u8; 3]; 5] = [
        [150, 114, 70],
        [162, 126, 80],
        [174, 138, 90],
        [186, 150, 102],
        [196, 162, 112],
    ];
    const BARK: [[u8; 3]; 3] = [[64, 47, 32], [76, 56, 38], [88, 66, 44]];
    build(|x, y, ix, iy| {
        let edge = ix.min(iy).min(N as i32 - 1 - ix).min(N as i32 - 1 - iy);
        if edge == 0 {
            return opaque(ramp(hash(seed, ix, iy), &BARK));
        }
        let (dx, dy) = (
            x - 8.0 + (fbm(seed + 1, x, y, 2, 2) - 0.5) * 1.8,
            y - 8.0 + (fbm(seed + 2, x, y, 2, 2) - 0.5) * 1.8,
        );
        let ring = ((dx * dx + dy * dy).sqrt() * 1.9).sin() * 0.5 + 0.5;
        let t = 0.15 + 0.7 * ring + (hash(seed + 3, ix, iy) - 0.5) * 0.2;
        opaque(ramp(if edge == 1 { t * 0.55 } else { t }, &WOODS))
    })
}

// Grey, alpha cut where there are holes. `gain` brightens it so the mean can be fitted.
fn leaves(seed: u32, gain: f32) -> Tile {
    const GREYS: [f32; 6] = [96.0, 116.0, 134.0, 152.0, 170.0, 188.0];
    build(|x, y, ix, iy| {
        let clump = fbm(seed, x, y, 4, 3);
        let hole = clump * 0.75 + hash(seed + 1, ix, iy) * 0.25 < 0.4;
        // texels under a hole's lower edge shade darker
        let below = fbm(seed, x, y - 1.0, 4, 3) * 0.75
            + hash(seed + 1, ix, (iy - 1).rem_euclid(N as i32)) * 0.25
            < 0.4;
        let t = 0.6 * fbm(seed + 2, x, y, 2, 2) + 0.4 * hash(seed + 3, ix, iy)
            - if below { 0.3 } else { 0.0 };
        let g = (GREYS[(t.clamp(0.0, 0.999) * GREYS.len() as f32) as usize] * gain)
            .clamp(0.0, 255.0)
            .round() as u8;
        // holes keep the grey so filtering doesn't darken their edges
        let g = if hole {
            (134.0 * gain).round() as u8
        } else {
            g
        };
        [g, g, g, if hole { 0 } else { 255 }]
    })
}

fn gravel(seed: u32) -> Tile {
    const STONES: [[u8; 3]; 6] = [
        [96, 92, 88],
        [112, 108, 102],
        [126, 121, 115],
        [140, 134, 126],
        [118, 106, 94],
        [152, 146, 138],
    ];
    build(|x, y, ix, iy| {
        let (wx, wy) = (
            x + (fbm(seed, x, y, 4, 2) - 0.5) * 2.0,
            y + (fbm(seed + 1, x, y, 4, 2) - 0.5) * 2.0,
        );
        let (f1, f2, id) = worley(seed + 2, wx, wy, 5);
        if f2 - f1 < 0.12 {
            return opaque([58, 55, 52]);
        }
        let base = ramp(id, &STONES);
        // each pebble lit from its upper left
        let lit = if f1 < 0.3 { 1.0 } else { 0.0 } * (hash(seed + 3, ix, iy) - 0.3);
        let shade = 1.0 + lit * 0.25 + (hash(seed + 4, ix, iy) - 0.5) * 0.14;
        opaque(base.map(|c| (c as f32 * shade).clamp(0.0, 255.0) as u8))
    })
}

fn ice(seed: u32) -> Tile {
    const ICES: [[u8; 3]; 5] = [
        [126, 178, 222],
        [138, 190, 230],
        [150, 201, 238],
        [164, 212, 244],
        [180, 224, 250],
    ];
    build(|x, y, ix, iy| {
        let (wx, wy) = (
            x + (fbm(seed, x, y, 2, 2) - 0.5) * 6.0,
            y + (fbm(seed + 1, x, y, 2, 2) - 0.5) * 6.0,
        );
        let (f1, f2, _) = worley(seed + 2, wx, wy, 3);
        let gap = f2 - f1;
        if gap < 0.06 {
            return opaque([232, 245, 252]);
        }
        if gap < 0.13 && hash(seed + 3, ix, iy) > 0.45 {
            return opaque([104, 158, 208]);
        }
        let t = 0.1 + 0.9 * fbm(seed + 4, x, y, 2, 3) + (hash(seed + 5, ix, iy) - 0.5) * 0.12;
        // a few bubbles
        if hash(seed + 6, ix, iy) > 0.985 {
            return opaque([205, 232, 250]);
        }
        opaque(ramp(t, &ICES))
    })
}

fn sandstone_side(seed: u32) -> Tile {
    const STONES: [[u8; 3]; 6] = [
        [178, 148, 100],
        [188, 158, 108],
        [198, 168, 116],
        [208, 178, 126],
        [216, 188, 138],
        [224, 198, 150],
    ];
    build(|x, y, ix, iy| {
        let warp = (fbm(seed, x, y, 2, 2) - 0.5) * 2.0;
        let fine = ((y / 16.0 * 8.0 + warp) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let coarse = ((y / 16.0 * 2.0 + warp * 0.3) * std::f32::consts::TAU).sin() * 0.5 + 0.5;
        let t = 0.05
            + 0.4 * fine
            + 0.4 * coarse
            + 0.1 * fbm(seed + 1, x, y, 4, 2)
            + (hash(seed + 2, ix, iy) - 0.5) * 0.14;
        opaque(ramp(t, &STONES))
    })
}

fn sandstone_top(seed: u32) -> Tile {
    const STONES: [[u8; 3]; 5] = [
        [200, 172, 120],
        [208, 180, 128],
        [216, 188, 136],
        [224, 197, 146],
        [230, 205, 156],
    ];
    build(|x, y, ix, iy| {
        let t = 0.2 + 0.4 * fbm(seed, x, y, 4, 2) + (hash(seed + 1, ix, iy) - 0.5) * 0.4;
        let (f1, _, id) = worley(seed + 2, x, y, 4);
        if f1 < 0.12 && id > 0.7 {
            return opaque([188, 160, 110]);
        }
        opaque(ramp(t, &STONES))
    })
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// The mean linear colour of the texels the shaders draw, those with alpha of at least half.
fn mean_linear(tile: &Tile) -> [f32; 3] {
    let drawn: Vec<&Px> = tile.iter().filter(|p| p[3] >= 128).collect();
    std::array::from_fn(|i| {
        drawn.iter().map(|p| srgb_to_linear(p[i])).sum::<f32>() / drawn.len() as f32
    })
}

// Bisects `gain` until the tile's mean linear grey is `target`; biome/mod.rs's tint maths relies on it.
fn fitted(target: f32, make: impl Fn(f32) -> Tile) -> Tile {
    let (mut lo, mut hi) = (0.3, 2.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if mean_linear(&make(mid))[0] < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    make((lo + hi) / 2.0)
}

fn digit_rows(d: u32) -> [u8; 5] {
    match d {
        0 => [7, 5, 5, 5, 7],
        1 => [2, 6, 2, 2, 7],
        2 => [7, 1, 7, 4, 7],
        3 => [7, 1, 7, 1, 7],
        4 => [5, 5, 7, 1, 1],
        5 => [7, 4, 7, 1, 7],
        6 => [7, 4, 7, 5, 7],
        7 => [7, 1, 1, 2, 2],
        8 => [7, 5, 7, 5, 7],
        _ => [7, 5, 7, 1, 7],
    }
}

fn contact_sheet(tiles: &[Tile]) -> RgbaImage {
    let cell = N as u32 * SHEET_SCALE;
    let (label, gap) = (14, 4);
    let width = tiles.len() as u32 * (cell + gap) + gap;
    let mut sheet =
        RgbaImage::from_pixel(width, label + cell * 2 + gap * 3, Rgba([24, 24, 28, 255]));
    for (i, tile) in tiles.iter().enumerate() {
        let left = gap + i as u32 * (cell + gap);
        let draw = |sheet: &mut RgbaImage, x: u32, y: u32, scale: u32, repeats: u32| {
            for py in 0..N as u32 * repeats * scale {
                for px in 0..N as u32 * repeats * scale {
                    let p = tile[((py / scale) as usize % N) * N + (px / scale) as usize % N];
                    // cut-out texels show a checker so holes read as holes
                    let back = if (px / 4 + py / 4) % 2 == 0 {
                        200.0
                    } else {
                        150.0
                    };
                    let a = p[3] as f32 / 255.0;
                    let out: [u8; 3] =
                        std::array::from_fn(|c| (p[c] as f32 * a + back * (1.0 - a)) as u8);
                    sheet.put_pixel(x + px, y + py, Rgba([out[0], out[1], out[2], 255]));
                }
            }
        };
        draw(&mut sheet, left, label + gap, SHEET_SCALE, 1);
        draw(&mut sheet, left, label + cell + gap * 2, SHEET_SCALE / 2, 2);
        for (k, ch) in i.to_string().chars().enumerate() {
            for (row, bits) in digit_rows(ch.to_digit(10).unwrap()).iter().enumerate() {
                for col in 0..3 {
                    if bits >> (2 - col) & 1 == 1 {
                        for s in 0..2 {
                            for t in 0..2 {
                                sheet.put_pixel(
                                    left + (k as u32 * 4 + col) * 2 + s,
                                    2 + row as u32 * 2 + t,
                                    Rgba([240, 240, 240, 255]),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    sheet
}

fn main() {
    let tiles: Vec<Tile> = vec![
        stone(11),
        fitted(0.5, |g| grass_top(23, g)),
        sand(31),
        water(47),
        snow(53),
        log_side(61),
        fitted(0.25, |g| leaves(71, g)),
        grass_side(83),
        dirt(97),
        log_end(103),
        gravel(113),
        ice(127),
        sandstone_side(131),
        sandstone_top(149),
        snow_side(157),
    ];
    assert_eq!(tiles.len(), NAMES.len());

    let mut atlas = RgbaImage::new((N * tiles.len()) as u32, N as u32);
    for (i, tile) in tiles.iter().enumerate() {
        for (j, p) in tile.iter().enumerate() {
            atlas.put_pixel((i * N + j % N) as u32, (j / N) as u32, Rgba(*p));
        }
    }
    atlas.save("assets/textures/blocks.png").unwrap();

    for (name, tile) in NAMES.iter().zip(&tiles) {
        let m = mean_linear(tile);
        println!(
            "        vec3<f32>({:.4}, {:.4}, {:.4}), // {name}",
            m[0], m[1], m[2]
        );
    }

    let sheet = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "target/atlas_sheet.png".into());
    contact_sheet(&tiles).save(&sheet).unwrap();
    println!("contact sheet: {sheet}");
}
