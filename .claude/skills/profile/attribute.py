"""Shares of a `perf script` dump by call path, through only the frames you name.

    perf script -i perf.data --no-demangle > perf.txt
    python3 attribute.py perf.txt tree_in_cell crown_boxes SkyLight perlin

Inlining folds callees into callers, so `perf report`'s call graph is hard to read. This keeps,
for each sample, the named frames on its stack, outermost first, and counts the paths. A key
matches any frame containing it, so mangled names (`9in_leaves`, `SkyLight3new`) work.
"""
import collections
import sys

dump, keys = sys.argv[1], sys.argv[2:]
paths = collections.Counter()
total = 0
for sample in open(dump).read().split("\n\n"):
    frames = [line.strip() for line in sample.strip().split("\n")[1:]]
    if not frames:
        continue
    total += 1
    path = []
    for frame in reversed(frames):
        for key in keys:
            if key in frame and key not in path:
                path.append(key)
    paths[" > ".join(path) or "(none)"] += 1
for path, count in paths.most_common(30):
    print(f"{100 * count / total:5.1f}% {path}")
