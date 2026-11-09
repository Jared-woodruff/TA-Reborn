#!/usr/bin/env bash
# The benchmark's quick runs for its reference (on the reference computer, after a change to what
# a frame draws): one run for each size, window mode and antialiasing given, each with a settings
# folder of its own made from TEMPLATE (a config.toml; by default the player's), muted, the frame
# rate unlimited. Reports go to scratch/bench/cal/<tag>_<W>x<H>_<msaa>.json; then
# `python tools/benchmark/fit.py <tag>` works the reference out of them.
#
#   tools/benchmark/calibrate.sh <tag> 1920x1080:Windowed:4 2560x1440:Windowed:4 3840x2160:Fullscreen:4 \
#       1920x1080:Windowed:1 1920x1080:Windowed:2 1920x1080:Windowed:8
#
# (Nothing else should run meanwhile: no build, no capture. The release build is used.)
set -u
cd "$(dirname "$0")/../.."
tag=$1; shift
template=${TEMPLATE:-$APPDATA/TA Reborn/config.toml}
mkdir -p scratch/bench/cal
for spec in "$@"; do
    IFS=: read -r size mode msaa <<< "$spec"
    w="${size%x*}"; h="${size#*x}"
    name="${tag}_${size}_${msaa}"
    dir="scratch/bench/cal/app_$name"
    mkdir -p "$dir/TA Reborn"
    sed -e "s/^msaa = .*/msaa = $msaa/" -e "s/^width = .*/width = $w/" -e "s/^height = .*/height = $h/" \
        -e "s/^window_mode = .*/window_mode = \"$mode\"/" -e "s/^vsync = .*/vsync = false/" -e "s/^master = .*/master = 0.0/" \
        "$template" > "$dir/TA Reborn/config.toml"
    out="scratch/bench/cal/$name.json"
    rm -rf "$out" "${out%.json}"
    start=$(date +%s)
    APPDATA="$(cygpath -w "$PWD/$dir")" timeout 900 target/release/ta-reborn.exe --benchmark "$out" --benchmark-mode quick --no-mods > "scratch/bench/cal/$name.log" 2>&1
    echo "$name: exit $? after $(( $(date +%s) - start )) s"
done
