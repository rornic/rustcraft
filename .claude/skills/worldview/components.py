# Lists the water bits in a worldview `water:1` export, smallest-first counts and the biggest small ones.
# usage: components.py export.png x0 z0 [max_pixels]
import sys

import numpy as np
from PIL import Image
from scipy import ndimage

path, x0, z0 = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
limit = int(sys.argv[4]) if len(sys.argv) > 4 else 2000
pic = np.array(Image.open(path).convert("RGB")).astype(int)
water = (pic[..., 0] != pic[0, 0, 0]) | (pic[..., 1] != pic[0, 0, 1]) | (pic[..., 2] != pic[0, 0, 2])
labels, n = ndimage.label(water)
sizes = ndimage.sum(water, labels, range(1, n + 1))
spans = ndimage.find_objects(labels)
small = sorted(((int(s), i) for i, s in enumerate(sizes) if s < limit), reverse=True)
print(f"{n} water parts, {int(water.sum())} water pixels, {len(small)} under {limit} pixels")
for size, i in small[:25]:
    rows, cols = spans[i]
    print(f"  {size:6d} px at x={x0 + (cols.start + cols.stop) // 2} z={z0 + (rows.start + rows.stop) // 2}")
