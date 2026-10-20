# Compares two worldview exports of the same view (same layers, x, z, w, h, scale), before and after a change.
# usage: diff.py before.png after.png out.png x0 z0 [scale]
# out.png is the after image dimmed, with changed pixels in magenta where the after is water and green
# where the before was water and the after is not; prints the changed area and its largest clusters.
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

before = np.array(Image.open(sys.argv[1]).convert("RGB")).astype(int)
after = np.array(Image.open(sys.argv[2]).convert("RGB")).astype(int)
x0, z0 = int(sys.argv[4]), int(sys.argv[5])
scale = float(sys.argv[6]) if len(sys.argv) > 6 else 1.0
if before.shape != after.shape:
    sys.exit(f"sizes differ: {before.shape} vs {after.shape}, export both with the same w, h and scale")
changed = (np.abs(before - after).sum(axis=2) > 24)
out = (after * 0.35).astype(np.uint8)
out[changed] = [255, 0, 255]
background = after[0, 0]
was_water = (before != before[0, 0]).any(axis=2)
is_water = (after != background).any(axis=2)
out[changed & was_water & ~is_water] = [0, 255, 0]
Image.fromarray(out).save(sys.argv[3])
print(f"{int(changed.sum())} changed pixels of {changed.size}")
labels, n = ndimage.label(ndimage.binary_dilation(changed, iterations=3))
sizes = ndimage.sum(changed, labels, range(1, n + 1))
spans = ndimage.find_objects(labels)
for size, i in sorted(((int(s), i) for i, s in enumerate(sizes)), reverse=True)[:15]:
    rows, cols = spans[i]
    cx = x0 + (cols.start + cols.stop) / 2 * scale
    cz = z0 + (rows.start + rows.stop) / 2 * scale
    print(f"  {size:6d} px changed near x={cx:.0f} z={cz:.0f}")
