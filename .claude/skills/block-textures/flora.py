"""Ground flora sprites, drawn pixel by pixel: SD can't draw a lone plant on transparency at 16px
(it frames it, adds a background or draws a mandala). Row 0 is the top; the plant stands on
row 15. Grass and ferns are greyscale, tinted by the climate like `GrassTop`; flowers keep
their colours. Clear texels carry the sprite's mean colour so mips don't bleed dark."""
import numpy as np

SIZE = 16
STEM = (66, 112, 40)
LEAF = (84, 134, 46)


def blank():
    return np.zeros((SIZE, SIZE, 4), float)


def put(img, x, y, rgb):
    x, y = int(round(x)), int(round(y))
    if 0 <= x < SIZE and 0 <= y < SIZE:
        img[y, x] = (*rgb, 255)


def grey(v):
    v = float(np.clip(v, 0, 255))
    return (v, v, v)


def tall_grass(rng):
    """A tuft of blades fanning up from the ground, darker at their roots."""
    img = blank()
    for _ in range(9):
        base = rng.uniform(2.5, 12.5)
        height = rng.integers(7, 16)
        lean = rng.uniform(-3.5, 3.5) + (base - 7.5) * 0.35
        tone = rng.uniform(-18, 18)
        for step in range(height):
            t = step / max(height - 1, 1)
            put(img, base + lean * t ** 1.6, 15 - step, grey(135 + 95 * t + tone))
    return img


def short_grass(rng):
    """A low, dense tuft, only a third to a half of a block tall."""
    img = blank()
    for _ in range(11):
        base = rng.uniform(1.5, 13.5)
        height = rng.integers(3, 8)
        lean = rng.uniform(-2.0, 2.0) + (base - 7.5) * 0.2
        tone = rng.uniform(-20, 20)
        for step in range(height):
            t = step / max(height - 1, 1)
            put(img, base + lean * t ** 1.4, 15 - step, grey(140 + 85 * t + tone))
    return img


def seed_grass(rng):
    """A few tall, thin stalks with pale seed heads, over a short tuft."""
    img = short_grass(rng)
    for base in rng.choice(np.arange(3, 13), size=3, replace=False):
        height = rng.integers(11, 16)
        lean = rng.uniform(-2.5, 2.5)
        for step in range(height):
            t = step / (height - 1)
            x, y = base + lean * t ** 1.8, 15 - step
            # the head: the top few texels, paler and a texel wider
            if step >= height - 4:
                put(img, x, y, grey(235))
                if step % 2:
                    put(img, x + (1 if lean >= 0 else -1), y, grey(215))
            else:
                put(img, x, y, grey(130 + 60 * t))
    return img


def fern(rng):
    """Fronds rising from one root and arching outward, leaflets either side, shorter toward the tips."""
    img = blank()
    for lean in (-30, -9, 8, 29):
        heading = np.radians(lean + rng.uniform(-6, 6))
        arch = np.sign(lean) * rng.uniform(0.035, 0.06)
        length = rng.integers(12, 15)
        x, y = 7.5 + np.sign(lean) * 0.5, 15.0
        for step in range(length):
            t = step / length
            x += np.sin(heading)
            y -= np.cos(heading)
            heading += arch
            put(img, x, y, grey(110 + 70 * t))
            if step >= 2 and step % 2 == 0 and step < length - 1:
                across = np.array([np.cos(heading), np.sin(heading)])
                for sign in (-1, 1):
                    for r in range(1, 3 if t < 0.55 else 2):
                        put(img, x + across[0] * r * sign, y + across[1] * r * sign + 0.5, grey(140 + 60 * t))
    return img


def stem(img, x, top, rng, leaf_at=None):
    for y in range(top, 16):
        wiggle = 0.6 * np.sin(y * 0.9 + rng.uniform(0, 1))
        put(img, x + wiggle, y, STEM)
    if leaf_at is not None:
        side = rng.choice((-1, 1))
        put(img, x + side, leaf_at, LEAF)
        put(img, x + 2 * side, leaf_at - 1, LEAF)


def poppy(rng):
    """A tall poppy and a shorter one beside it: four-petalled red heads with dark hearts."""
    img = blank()
    for x, top in ((6, 3), (11, 7)):
        stem(img, x, top + 2, rng, leaf_at=12)
        red, deep = (214, 38, 30), (168, 22, 22)
        for dx, dy, c in ((-1, 0, red), (0, 0, deep), (1, 0, red), (-1, 1, red), (0, 1, (40, 18, 16)), (1, 1, red),
                          (0, -1, red), (-2, 1, deep), (2, 1, deep), (0, 2, deep)):
            put(img, x + dx, top + dy, c)
    return img


def dandelion(rng):
    """Two dandelions over a jagged rosette of leaves."""
    img = blank()
    for x in range(3, 13):
        if rng.random() < 0.75:
            h = 1 + (rng.random() < 0.5) + (abs(x - 7.5) < 3)
            for y in range(16 - h, 16):
                put(img, x, y, LEAF if (x + y) % 3 else STEM)
    for x, top in ((7, 4), (10, 8)):
        stem(img, x, top + 2, rng)
        gold, light = (236, 186, 26), (252, 218, 64)
        for dx, dy in ((-1, 0), (0, 0), (1, 0), (-1, 1), (0, 1), (1, 1), (0, -1)):
            put(img, x + dx, top + dy, light if (dx + dy) % 2 else gold)
    return img


DRAW = {"TallGrass": tall_grass, "Fern": fern, "Poppy": poppy, "Dandelion": dandelion, "ShortGrass": short_grass, "SeedGrass": seed_grass}


def sprite(name, seed, mean_grey=None):
    """The sprite as RGBA uint8, greyscale ones levelled to `mean_grey` (sRGB) for the tint."""
    img = DRAW[name](np.random.default_rng(seed))
    drawn = img[..., 3] > 0
    if mean_grey is not None:
        img[drawn, :3] *= mean_grey / img[drawn, :3].mean()
    img[~drawn, :3] = img[drawn, :3].mean(0)
    return np.clip(img.round(), 0, 255).astype("uint8")
