#!/usr/bin/env bash
# Wrap the built binary in a minimal .app bundle so 3D Review can be run from Finder / the Dock
# like a real macOS app, and optionally launch it on a model.
#
# This is the *development* bundle, not the shipping one. `scripts/build-mac.sh` produces the real
# thing: the .icns, the `.fbx` document type that puts 3D Review in Finder's "Open With", `codesign
# --options runtime`, notarization and the .dmg. This one exists because it is far faster — no
# icon, no signing, and it can take a debug build — and because it is the only way to exercise the
# two shell leaves at all: a bare executable gets no menu bar, no Dock presence, no proper
# activation, and is not what `open` hands file arguments to (mac-port-plan.md D14/D15).
#
#   scripts/dev-app.sh                          build release, bundle, print the path
#   scripts/dev-app.sh assets/.../SM_x.fbx      ... and launch it on that model
#   scripts/dev-app.sh --debug SM_x.fbx         bundle the debug build instead (faster iteration)
#   scripts/dev-app.sh --no-build               re-bundle whatever is already built
#
# Known gaps at this stage, so they are not mistaken for bugs: a generic icon, and no
# `CFBundleDocumentTypes`, so Finder will not route a double-clicked .fbx here — `open -a` and a
# drop on the Dock icon do work, because those name the app explicitly.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile=release
build=1
args=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)    profile=debug; shift ;;
        --release)  profile=release; shift ;;
        --no-build) build=0; shift ;;
        -h|--help)  sed -n '2,19p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)          args+=("$1"); shift ;;
    esac
done

# Product metadata comes from product.json, the same single source crates/app/build.rs reads, so
# the bundle cannot drift from the binary it wraps. plutil reads JSON, so this needs nothing
# installed.
name="$(plutil -extract productName raw -o - "$repo/product.json")"
version="$(plutil -extract version raw -o - "$repo/product.json")"
exe="$(plutil -extract exeName raw -o - "$repo/product.json")"

if [[ $build -eq 1 ]]; then
    if [[ $profile == release ]]; then
        cargo build --manifest-path "$repo/Cargo.toml" -p review-app --release
    else
        cargo build --manifest-path "$repo/Cargo.toml" -p review-app
    fi
fi

bin="$repo/target/$profile/$exe"
[[ -x "$bin" ]] || { echo "no $profile binary at $bin — drop --no-build?" >&2; exit 1; }

app="$repo/target/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$bin" "$app/Contents/MacOS/$name"

# NSHighResolutionCapable is not optional: without it macOS runs the window through 1x scaling and
# every pixel is blurry on a Retina display, which would make the scene look like the port had
# broken the viewport when it had not.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>         <string>$name</string>
    <key>CFBundleIdentifier</key>         <string>com.psmyles.3d-review.dev</string>
    <key>CFBundleName</key>               <string>$name</string>
    <key>CFBundleDisplayName</key>        <string>$name ($profile)</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>$version</string>
    <key>CFBundleVersion</key>            <string>$version</string>
    <key>LSMinimumSystemVersion</key>     <string>11.0</string>
    <key>NSHighResolutionCapable</key>    <true/>
</dict>
</plist>
PLIST

echo "bundled $profile build -> $app"

[[ ${#args[@]} -eq 0 ]] && exit 0

# Launch Services keys an `open` on the bundle, not on the binary inside it, so an older instance
# of this same dev bundle would take the file (D14's whole point) and you would be testing the
# build you just replaced. Retire it first.
if pkill -f "$app/Contents/MacOS/$name" 2>/dev/null; then
    echo "(stopped the previous $name dev instance so this build is the one that opens)"
    sleep 0.3
fi

# Absolute paths: `open` does not resolve arguments against the caller's working directory.
abs=()
for a in "${args[@]}"; do
    [[ "$a" == /* ]] && abs+=("$a") || abs+=("$PWD/$a")
done
# `-a` is load-bearing: it means "open these files *with* this app", which goes through Launch
# Services and arrives as `application:openURLs:` — the path D14 exists for, and the one worth
# testing. Without it `open` treats the app and the file as two things to open independently, and
# the .fbx goes to whatever the system default handler is (Preview, which cannot read one).
# `--args` would be wrong too: that delivers the path as argv, exercising the Windows path on a Mac.
open -a "$app" "${abs[@]}"
