#!/usr/bin/env bash
# Build and package 3D Review for Linux as a tarball — the twin of scripts/build-mac.sh
# and packaging/build-windows-installer.ps1 (docs/ARCHITECTURE.md, Platform decisions
# D26).
#
#   scripts/build-linux.sh               build in release, write dist/<exe>-<version>-linux-x86_64.tar.gz
#   scripts/build-linux.sh --no-build    re-wrap the binaries already in target/release
#   scripts/build-linux.sh --out <dir>   where the tarball goes (default: dist/)
#
# The tarball unpacks to one folder, `<exe>-<version>-linux-x86_64/`, with the viewer
# (`3d-review`) and the console tool (`review3d-cli`, docs/cli.md) at its top, the
# manual's pictures in `docs/` beside them (where the viewer's Help looks), and the
# licence notices. There is no installer: unpack it anywhere, put the folder on PATH or
# point a tool at the binary. A tool that keeps its helpers in a folder of its own
# (a git-ignored `tools/`) unpacks it there.
#
# It edits no tracked files: everything it writes goes to target/ and dist/. The
# version comes from product.json, the only place it is set; the crates' build.rs files
# read the same file.
#
# The archive is reproducible: entries in name order, owned by 0:0, stamped with the
# Unix epoch, so the same binaries always give the same tarball.
#
# What the binaries need from the machine: glibc at least as new as the build
# machine's floor (printed at the end, read from the binaries' own symbol versions),
# the C++ runtime (libstdc++), which meshoptimizer and psd_sdk need, and for the viewer
# the Vulkan loader and a Vulkan 1.3 driver (D28). All come with a desktop
# distribution; a build meant for older ones has to be made on an older one.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
do_build=1
out_dir="$repo/dist"

usage() { sed -n '2,26p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-build) do_build=0; shift ;;
        --out)      out_dir="$2"; shift 2 ;;
        -h|--help)  usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

say() { printf '\n== %s ==\n' "$1"; }
die() { echo "error: $*" >&2; exit 1; }

[[ "$(uname -s)" == Linux ]] || die "this packages a Linux build; run it on Linux"
[[ "$(uname -m)" == x86_64 ]] || die "only x86_64 is packaged"

# Top-level string fields of product.json. Each name appears once, at the top level,
# so a line match is enough and no JSON tool has to be installed.
meta() {
    sed -n "s/^  \"$1\": *\"\\([^\"]*\\)\".*/\\1/p" "$repo/product.json" | head -n 1
}
version="$(meta version)"
exe="$(meta exeName)"
[[ -n "$version" && -n "$exe" ]] || die "could not read version / exeName from product.json"

if [[ $do_build -eq 1 ]]; then
    say "build $exe and review3d-cli $version"
    cargo build --manifest-path "$repo/Cargo.toml" -p review-app -p review-cli --release
fi

viewer="$repo/target/release/$exe"
cli="$repo/target/release/review3d-cli"
for bin in "$viewer" "$cli"; do
    [[ -x "$bin" ]] || die "no release build at $bin — drop --no-build?"
    file -b "$bin" | grep -q 'ELF 64-bit.*x86-64' || die "$bin is not an x86_64 ELF"
done
built="$("$cli" --version)"
[[ "$built" == "review3d-cli $version" ]] || die "the binary says '$built', product.json says $version"

say "stage"
name="$exe-$version-linux-x86_64"
stage="$repo/target/package-linux"
rm -rf "$stage"
mkdir -p "$stage/$name/licenses"
cp "$viewer" "$stage/$name/$exe"
cp "$cli" "$stage/$name/review3d-cli"
# The manual's pictures, when it has any (its text is compiled into the viewer); the
# Windows installer skips them the same way when there are none.
if [[ -d "$repo/docs/book/src/en/images" ]]; then
    mkdir -p "$stage/$name/docs/en"
    cp -R "$repo/docs/book/src/en/images" "$stage/$name/docs/en/"
fi

# The licence notices. The binary carries code from every project in
# docs/THIRD-PARTY-NOTICES.md, and their licences require the notices to travel with it.
for notice in LICENSE docs/THIRD-PARTY-NOTICES.md docs/CREDITS.md; do
    [[ -f "$repo/$notice" ]] || die "missing licence file the package must ship: $notice"
done
cp "$repo/LICENSE" "$stage/$name/LICENSE.txt"
cp "$repo/docs/THIRD-PARTY-NOTICES.md" "$repo/docs/CREDITS.md" "$stage/$name/"
cp "$repo"/licenses/*.txt "$stage/$name/licenses/"

say "tarball"
mkdir -p "$out_dir"
tarball="$out_dir/$name.tar.gz"
tar --create --directory "$stage" --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime=@0 --mode='u+rwX,go+rX,go-w' "$name" | gzip -9 -n > "$tarball"

floor="$(objdump -T "$viewer" "$cli" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)"
say "done — $tarball"
echo "needs glibc $floor or newer, libstdc++, and for the viewer a Vulkan 1.3 driver"
