#!/usr/bin/env bash
# Capture identical whole-region views from a running examples/worldview binary.
set -euo pipefail
viewer_url=${1:?usage: capture-worldview.sh URL OUTPUT_DIR before|after [seeds...]}
output_dir=${2:?missing output directory}
phase=${3:?missing before|after}
shift 3
if [[ "$phase" != before && "$phase" != after ]]; then
    echo "phase must be before or after" >&2
    exit 2
fi
if [[ $# == 0 ]]; then set -- 1 42 1337; fi
mkdir -p "$output_dir"
for seed in "$@"; do
    names=(landscape biomes moisture temperature)
    layers=('height,hillshade,water:0.75' 'biomes,hillshade,water:0.75' 'moisture,water:0.75' 'temperature,water:0.75')
    if [[ "$phase" == after ]]; then
        names+=(plates uplift precipitation surface-temperature rivers)
        layers+=('plates,boundaries:0.55,motion' 'uplift,hillshade,water:0.75' 'precipitation,water:0.75' 'surface_temperature,water:0.75' 'height,hillshade,water:0.4,rivers')
    fi
    for i in "${!names[@]}"; do
        destination="$output_dir/seed-$seed-${names[$i]}.png"
        curl --fail --silent --show-error --get "$viewer_url/export" \
            --data-urlencode "seed=$seed" --data-urlencode "layers=${layers[$i]}" \
            --data-urlencode 'x=-32768' --data-urlencode 'z=-32768' \
            --data-urlencode 'w=1024' --data-urlencode 'h=1024' \
            --data-urlencode 'scale=64' -o "$destination"
        echo "$destination"
    done
    curl --fail --silent --show-error "$viewer_url/summary?seed=$seed" -o "$output_dir/seed-$seed-summary.txt"
done
