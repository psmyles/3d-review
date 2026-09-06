#!/usr/bin/env bash
# Fail a release build whose committed shader bytecode does not match the shaders in
# the tree (mac-port-plan.md D5) — the twin of `check-shader-bytecode.ps1`, with the
# two hosts' roles swapped.
#
# The viewer compiles no shader at run time: it `include_bytes!`s blobs committed
# beside the generated sources, and `crates/render/build.rs` rebuilds one only when it
# does not match the source it was compiled from. That gate is enough on a dev box
# with the toolchain. It is not enough for a *release*, for two reasons:
#
#   * a Mac without the 839 MB Metal toolchain builds happily from whatever blobs are
#     committed, and says so only in a `cargo:warning` that scrolls past;
#   * there are two hosts. A shader edited and rebuilt on Windows arrives here as a
#     new generated source next to a `.metallib` that is a shader revision behind, and
#     `git checkout` gives both the same mtime — so "is the blob stale?" cannot be
#     answered from timestamps at all on the one occasion it matters.
#
# So this checks content, against the manifest `build.rs` writes: every generated
# `*_metal_macos_*.metal` must have a row, and both its source and its `.metallib`
# must still hash to what that row records. Anything else fails the build.
#
# The Windows rows are checked too, but only ever **warn**: a .dmg ships no `.dxbc`,
# and it is `check-shader-bytecode.ps1` that hard-fails on those.
set -euo pipefail

repo="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
generated="$repo/crates/render/src/shaders/generated"
manifest="$generated/bytecode.manifest"

[[ -d "$generated" ]] || { echo "not found: $generated (run scripts/gen-shaders.sh)" >&2; exit 1; }
if [[ ! -f "$manifest" ]]; then
    cat >&2 <<EOF
$manifest is missing.
It is written by crates/render/build.rs and committed beside the blobs; build the
render crate once on a machine with a shader toolchain, then commit what it writes.
EOF
    exit 1
fi

digest() { shasum -a 256 "$1" | cut -d' ' -f1; }

# Check one host's generated sources against the manifest, printing one line per
# problem. Returns 1 if there was any.
check_set() {
    local slang="$1" source_ext="$2" blob_ext="$3" problems=0 source name row blob
    for source in "$generated"/review_*_"$slang"_*."$source_ext"; do
        [[ -e "$source" ]] || continue
        name="$(basename "$source")"
        row="$(awk -v n="$name" '$1 == n { print; exit }' "$manifest")"
        if [[ -z "$row" ]]; then
            echo "  $name: no manifest row (never compiled here)"; problems=1; continue
        fi
        read -r _ source_hash _ blob_hash _ <<<"$row"
        if [[ "$(digest "$source")" != "$source_hash" ]]; then
            echo "  $name: changed since its blob was built"; problems=1; continue
        fi
        blob="${source%.$source_ext}.$blob_ext"
        if [[ ! -f "$blob" ]]; then
            echo "  $(basename "$blob"): missing"; problems=1; continue
        fi
        if [[ "$(digest "$blob")" != "$blob_hash" ]]; then
            echo "  $(basename "$blob"): does not match the manifest"; problems=1
        fi
    done
    # A row whose source is gone is stale bookkeeping rather than a shipping hazard,
    # but it means the manifest was hand-edited or a regeneration was half-committed.
    while read -r name _; do
        [[ "$name" == review_*_"$slang"_*."$source_ext" ]] || continue
        [[ -f "$generated/$name" ]] || { echo "  $name: manifest row with no source"; problems=1; }
    done < <(grep -v '^[[:space:]]*\(#\|$\)' "$manifest")
    return $problems
}

echo "==> Checking shader bytecode against the committed sources..."

if ! metal_problems="$(check_set metal_macos metal metallib)"; then
    echo "Shader bytecode is out of date:" >&2
    echo "$metal_problems" >&2
    cat >&2 <<'EOF'
The committed .metallib does not match the shaders in the tree, so this build would
ship bytecode that is not the shader source beside it. Build the render crate on a Mac
with the Metal toolchain (any `cargo build -p review-render`, after
`xcodebuild -downloadComponent MetalToolchain`), then commit the regenerated
.metallib files and bytecode.manifest.
EOF
    exit 1
fi

# The Windows half: warn only.
if compgen -G "$generated/*.dxbc" > /dev/null; then
    if ! hlsl_problems="$(check_set hlsl5 hlsl dxbc)"; then
        echo "warning: the Windows shader bytecode is out of date (this .dmg does not ship it):" >&2
        echo "$hlsl_problems" >&2
        echo "warning: rebuild on Windows before shipping a Windows build." >&2
    fi
fi

count="$(ls "$generated"/review_*_metal_macos_*.metal 2>/dev/null | wc -l | tr -d ' ')"
echo "==> Shader bytecode is current ($count programs)."
