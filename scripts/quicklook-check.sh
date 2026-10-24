#!/usr/bin/env bash
# Check the Quick Look extensions' dylib without Finder: draw a model's thumbnail,
# then drive a preview session through a load, an orbit, a zoom and a reframe, and
# write every step as a PNG — the twin of scripts/shell-thumbnail.ps1 and
# shell-preview.ps1 on Windows.
#
# It calls crates/shell-quicklook exactly as the extensions do, through the same C
# header and the same Swift image conversion, but in a plain process: no sandbox,
# no Quick Look agent, nothing on screen. Whether Finder picks the extensions is a
# separate question — see `qlmanage -t` / `qlmanage -p` after scripts/dev-app.sh.
#
#   scripts/quicklook-check.sh <model.fbx>... [--out <dir>] [--debug] [--no-build]
#
#   --out <dir>   where the PNGs go (default target/quicklook-check), one folder
#                 per model
#   --debug       check the unoptimized dylib
#   --no-build    reuse the dylib already built
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
crate="$repo/crates/shell-quicklook"
profile=release-shell
build=1
out="$repo/target/quicklook-check"
models=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --out)      out="$2"; shift 2 ;;
        --debug)    profile=debug; shift ;;
        --no-build) build=0; shift ;;
        -h|--help)  sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)          models+=("$1"); shift ;;
    esac
done
[[ ${#models[@]} -gt 0 ]] || { echo "name at least one model" >&2; exit 2; }

if [[ $build -eq 1 ]]; then
    if [[ $profile == debug ]]; then
        cargo build --manifest-path "$repo/Cargo.toml" -p review-shell-quicklook
    else
        cargo build --manifest-path "$repo/Cargo.toml" -p review-shell-quicklook --profile "$profile"
    fi
fi
lib="$repo/target/$profile"
[[ -f "$lib/libreview_quicklook.dylib" ]] || { echo "no dylib in $lib — drop --no-build?" >&2; exit 1; }

harness="$lib/quicklook-check"
xcrun swiftc -O -swift-version 5 \
    -import-objc-header "$crate/include/review_quicklook.h" \
    "$crate/check/main.swift" "$crate/extension/Image.swift" \
    -L "$lib" -lreview_quicklook -Xlinker -rpath -Xlinker "$lib" \
    -o "$harness"

status=0
for model in "${models[@]}"; do
    name="$(basename "$model" .fbx)"
    echo "== $model"
    "$harness" "$model" "$out/$name" || status=1
done
exit $status
