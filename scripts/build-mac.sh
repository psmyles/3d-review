#!/usr/bin/env bash
# Build, sign, notarize and package 3D Review for macOS — the twin of
# packaging/build-windows-installer.ps1 (mac-port-plan.md D12).
#
# This runs **on the dev Mac, by hand**. Nothing else signs or packages, so the
# Developer ID certificate and the App Store Connect credential never have to exist as
# secrets anywhere but this keychain.
#
#   scripts/build-mac.sh                   a release: build, sign, notarize, staple, .dmg
#   scripts/build-mac.sh --no-notarize     signed but not notarized (faster; not shippable)
#   scripts/build-mac.sh --no-sign         unsigned bundle, for local testing only
#   scripts/build-mac.sh --no-dmg --no-build   re-wrap the binary that is already built
#
# The no-argument form is the shipping one and takes both credentials from the
# keychain: the "Developer ID Application" identity, and the `notarytool` profile named
# below. Neither is ever passed on a command line, and neither lives in the repo.
#
# It edits one tracked file: like the Windows installer script, it writes
# product.json's version into crates/app/Cargo.toml before building, so the manifest
# cannot drift from the version the app and the bundle report. Everything else it
# writes goes to target/ and dist/.
#
# Options
#   --sign-id <identity>       codesign identity; default is the "Developer ID
#                              Application" one in the keychain, which is the only kind
#                              Gatekeeper accepts outside the App Store.
#   --no-sign                  skip codesign entirely. The bundle runs here and nowhere
#                              else. Implies --no-notarize. Never ship this.
#   --notarize-profile <name>  use a different `xcrun notarytool store-credentials`
#                              profile.
#   --no-notarize              skip notarization and stapling. The .dmg is still signed,
#                              but macOS 15+ has no Control-click bypass any more —
#                              opening it on another Mac means System Settings →
#                              Privacy & Security → Open Anyway. Iterate with it; do not
#                              ship it.
#   --no-dmg                   stop after the .app.
#   --no-build                 reuse target/release/3d-review as it stands.
#   --out <dir>                where the .dmg goes (default: dist/).
#
# What is deliberately not here:
#
#   * A universal binary. D11 is Apple Silicon only, so the script checks the binary's
#     architecture rather than pretending.
#   * An IBL re-bake. The Windows installer script runs `generate-ibl-bake.ps1`,
#     freshness-gated, and there is no twin on purpose: the Metal bake is numerically
#     equivalent to the committed maps but not byte-identical (GPU float precision —
#     measured at ≤0.021 on a 0..1 BRDF LUT), so running it from a release build would
#     silently swap the committed assets for a second set of bytes. It warns when an
#     input is newer instead, and re-baking stays a deliberate act.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
sign_id=""
do_sign=1
do_notarize=1
do_dmg=1
do_build=1
out_dir="$repo/dist"

usage() { sed -n '2,46p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --sign-id)          sign_id="$2"; shift 2 ;;
        --no-sign)          do_sign=0; shift ;;
        --notarize-profile) notary_profile="$2"; shift 2 ;;
        --no-notarize)      do_notarize=0; shift ;;
        --no-dmg)           do_dmg=0; shift ;;
        --no-build)         do_build=0; shift ;;
        --out)              out_dir="$2"; shift 2 ;;
        -h|--help)          usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

say() { printf '\n== %s ==\n' "$1"; }
die() { echo "error: $*" >&2; exit 1; }

# Product metadata comes from product.json, the same single source crates/app/build.rs
# reads, so the bundle cannot drift from the binary it wraps. plutil reads JSON, so
# this needs nothing installed.
meta() { plutil -extract "$1" raw -o - "$repo/product.json"; }
name="$(meta productName)"
version="$(meta version)"
copyright="$(meta copyright)"
exe="$(meta exeName)"
file_ext="$(meta fileAssociation.extension)"; file_ext="${file_ext#.}"
file_type_name="$(meta fileAssociation.typeName)"
bundle_id="com.psmyles.3d-review"
# The master artwork is a cube whose faces run almost corner to corner, and macOS
# masks a legacy icon like this one into the rounded rect it draws everywhere — with
# no inset the cube's own corners are what gets clipped. The alpha is *kept*: a shaped
# icon is normal on this OS and is what the Windows .ico shows too, so there is no
# background to invent here.
icon_inset="8%"
# The `xcrun notarytool store-credentials` profile to use when none is named: the
# product name lower-cased with spaces removed, which is what the setup line printed
# on failure tells you to create. The credential lives in the data-protection
# keychain, which `security(1)` cannot even enumerate — so there is no point
# pre-flighting it here; notarytool's own error is immediate and says what is missing.
notary_profile="${notary_profile:-$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | tr -d ' ')}"

# `--no-sign` leaves nothing notarizable: notarization is a check on a Developer ID
# signature.
[[ $do_sign -eq 1 ]] || do_notarize=0

# Submit `path` and wait. On failure print the one command that says *why* —
# notarytool's exit status alone does not, and `set -e` would otherwise kill the
# script before the submission id is visible.
notarize() {
    local path="$1" log status id
    log="$(mktemp)"
    xcrun notarytool submit "$path" --keychain-profile "$notary_profile" --wait 2>&1 | tee "$log"
    status=${PIPESTATUS[0]}
    if [[ $status -ne 0 ]]; then
        id="$(sed -n 's/^ *id: *\([0-9a-f-]*\)$/\1/p' "$log" | head -1)"
        echo >&2
        if [[ -n $id ]]; then
            echo "error: notarization failed. Apple's reasons:" >&2
            echo "  xcrun notarytool log $id --keychain-profile $notary_profile" >&2
        else
            echo "error: notarization failed before submitting — is the '$notary_profile' profile set up?" >&2
            echo "  xcrun notarytool store-credentials $notary_profile --apple-id <id> --team-id <team> --password <app-specific-password>" >&2
        fi
        rm -f "$log"
        exit 1
    fi
    rm -f "$log"
}

# ---------------------------------------------------------------------------------------------
# 1. Preflight: the assets the binary embeds, and the shader bytecode it ships
# ---------------------------------------------------------------------------------------------
# All three are `include_bytes!`-ed, so a missing one is a build error rather than a
# broken release — but the *stale* cases are silent, and this is the last place to
# catch them.

say "preflight"
"$repo/packaging/check-shader-bytecode.sh" "$repo"

baked="$repo/assets/ibl_baked"
for map in "$baked"/T_IBL_*.bin; do
    [[ -e "$map" ]] || die "no baked IBL maps in $baked — the render crate will not build"
done
# Freshness is a warning, not a failure: see the header for why no re-bake runs here.
# Two `find`s because they take different predicates — paths must all precede the
# expression, and folding the source files in beside `-name 'T_HDR_*.hdr'` made the
# whole call a usage error that `set -e` then turned into a silent exit.
newest_input="$( {
    find "$repo/assets/textures" -name 'T_HDR_*.hdr' -newer "$baked/T_IBL_BRDF.bin" -print
    find "$repo/crates/render/src/ibl.rs" \
         "$repo/crates/render/src/shaders/review.glsl" \
         "$repo/crates/render/src/bin/bake_ibl.rs" \
         "$repo/crates/render/src/rhi/bake.rs" \
         -newer "$baked/T_IBL_BRDF.bin" -print
} 2>/dev/null | head -5 || true)"
if [[ -n "$newest_input" ]]; then
    echo "  warning: newer than the baked IBL maps —"
    echo "$newest_input" | sed "s|^$repo/|    |"
    echo "    re-bake deliberately if that matters:"
    echo "    cargo run --release -p review-render --features bake --bin bake_ibl"
fi

thumbnails="$(ls "$repo/assets/thumbnails"/T_HDR_*.png 2>/dev/null | wc -l | tr -d ' ')"
(( thumbnails > 0 )) || die "no HDR thumbnails in assets/thumbnails — crates/ui embeds them"
echo "  $(ls "$baked"/T_IBL_*.bin | wc -l | tr -d ' ') baked IBL maps, $thumbnails HDR thumbnails"

# ---------------------------------------------------------------------------------------------
# 2. Sync the crate version to product.json
# ---------------------------------------------------------------------------------------------
# `product.json` is the single source of the version, but nothing in a `cargo build`
# reads it into the manifest — crates/app/build.rs only *warns* when the two disagree,
# in a `cargo:warning` that scrolls past. So the release syncs it, exactly as the
# Windows installer script does.

say "syncing crates/app/Cargo.toml version to $version"
app_toml="$repo/crates/app/Cargo.toml"
current="$(sed -n -E 's/^version[[:space:]]*=[[:space:]]*"([^"]*)".*/\1/p' "$app_toml" | head -1)"
[[ -n $current ]] || die "no version found in $app_toml"
if [[ "$current" == "$version" ]]; then
    echo "  already in sync."
else
    sed -i '' -E "s/^version[[:space:]]*=[[:space:]]*\"[^\"]*\"/version = \"$version\"/" "$app_toml"
    echo "  updated: $current -> $version."
    [[ $do_build -eq 1 ]] || echo "  note: --no-build, so target/ still holds a $current binary."
fi

# ---------------------------------------------------------------------------------------------
# 3. Build
# ---------------------------------------------------------------------------------------------

if [[ $do_build -eq 1 ]]; then
    say "cargo build --release"
    cargo build --manifest-path "$repo/Cargo.toml" -p review-app --release
fi

bin="$repo/target/release/$exe"
[[ -x "$bin" ]] || die "no release binary at $bin — drop --no-build?"

archs="$(lipo -archs "$bin")"
[[ "$archs" == *arm64* ]] || die "the binary is '$archs', not arm64 (D11 is Apple Silicon only)"

# ---------------------------------------------------------------------------------------------
# 4. The icon
# ---------------------------------------------------------------------------------------------
# iconutil wants an .iconset directory of exact sizes, each the 1024² master
# downsampled and inset. Rebuilt every run: it is well under a second, and it means a
# change to the master cannot be silently missing from a release.
#
# `sips -z` would do the downsampling but cannot inset, so the resize and the inset
# happen together in AppKit, driven by osascript's JavaScript-for-Automation ObjC
# bridge — which is in the base OS, so this still needs nothing installed. The bitmap
# is retagged sRGB before anything is drawn into it; left as NSCalibratedRGB the
# artwork comes out colour-converted, and the cube's primaries are the whole icon.

say "icon"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
iconset="$tmp/$name.iconset"
mkdir -p "$iconset"

cat > "$tmp/inset-icon.js" <<'JS'
ObjC.import('AppKit');

// argv: <src.png> <inset-percent> <size>:<dst.png>...
function run(argv) {
    var src = argv[0], inset = parseFloat(argv[1]) / 100;
    var img = $.NSImage.alloc.initWithContentsOfFile(src);
    if (img.isNil()) throw new Error('cannot read ' + src);

    argv.slice(2).forEach(function (spec) {
        var colon = spec.indexOf(':');
        var size = parseInt(spec.slice(0, colon), 10), dst = spec.slice(colon + 1);
        // Fractional at the small sizes (16² insets by just over a pixel), which is
        // the point: the artwork is drawn into a sub-pixel rect and antialiased,
        // rather than snapped to the edge.
        var pad = size * inset;

        var rep = $.NSBitmapImageRep.alloc
            .initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
                $(), size, size, 8, 4, true, false, $.NSCalibratedRGBColorSpace, 0, 0);
        rep = rep.bitmapImageRepByRetaggingWithColorSpace($.NSColorSpace.sRGBColorSpace);

        $.NSGraphicsContext.saveGraphicsState;
        $.NSGraphicsContext.setCurrentContext(
            $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
        $.NSGraphicsContext.currentContext.setImageInterpolation(3); // .high
        img.drawInRectFromRectOperationFraction(
            $.NSMakeRect(pad, pad, size - 2 * pad, size - 2 * pad),
            $.NSZeroRect, $.NSCompositingOperationSourceOver, 1.0);
        $.NSGraphicsContext.restoreGraphicsState;

        var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $({}));
        if (!png.writeToFileAtomically(dst, true)) throw new Error('cannot write ' + dst);
    });
}
JS

renders=()
for size in 16 32 128 256 512; do
    renders+=("$size:$iconset/icon_${size}x${size}.png")
    renders+=("$((size * 2)):$iconset/icon_${size}x${size}@2x.png")
done
osascript -l JavaScript "$tmp/inset-icon.js" \
    "$repo/assets/icons/application-logo.png" "${icon_inset%\%}" "${renders[@]}" \
    || die "could not render the iconset from assets/icons/application-logo.png"
(( $(ls "$iconset" | wc -l) == ${#renders[@]} )) || die "the iconset is short of ${#renders[@]} images"

icns="$tmp/$name.icns"
iconutil -c icns "$iconset" -o "$icns"
echo "  $(basename "$icns") — $(stat -f%z "$icns") bytes, inset $icon_inset"

# ---------------------------------------------------------------------------------------------
# 5. The bundle
# ---------------------------------------------------------------------------------------------

app="$repo/target/$name.app"
say "bundle -> $app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/$name"
cp "$icns" "$app/Contents/Resources/$name.icns"
# The four-byte type/creator file. Vestigial, but its absence still confuses some tools.
printf 'APPL????' > "$app/Contents/PkgInfo"

# `LSHandlerRank = Alternate` rather than Owner is deliberate (D12): 3D Review
# volunteers for .fbx and appears in "Open With", but does not take the extension away
# from whatever already claims it. The user chooses, in Get Info → Change All.
#
# The document type is what makes a *double-clicked* .fbx reach the app at all — and
# it arrives as an Apple event, not as argv, which is what `review-shell-macos`'s
# openURLs: hook exists for (D14). Declaring one without the other would leave a
# double-click opening an empty window.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>         <string>$name</string>
    <key>CFBundleIdentifier</key>         <string>$bundle_id</string>
    <key>CFBundleName</key>               <string>$name</string>
    <key>CFBundleDisplayName</key>        <string>$name</string>
    <key>CFBundleIconFile</key>           <string>$name</string>
    <key>CFBundlePackageType</key>        <string>APPL</string>
    <key>CFBundleShortVersionString</key> <string>$version</string>
    <key>CFBundleVersion</key>            <string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>NSHumanReadableCopyright</key>   <string>$copyright</string>
    <key>LSApplicationCategoryType</key>  <string>public.app-category.graphics-design</string>
    <!-- arm64 only (D11), and arm64 starts at Big Sur. -->
    <key>LSMinimumSystemVersion</key>     <string>11.0</string>
    <!-- Not optional: without it macOS runs the window through 1x scaling and the
         whole viewport is blurry on a Retina display. -->
    <key>NSHighResolutionCapable</key>    <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>       <string>$file_type_name</string>
            <key>CFBundleTypeRole</key>       <string>Viewer</string>
            <key>LSHandlerRank</key>          <string>Alternate</string>
            <key>CFBundleTypeIconFile</key>   <string>$name</string>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>$file_ext</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST
plutil -lint "$app/Contents/Info.plist" >/dev/null || die "generated Info.plist is malformed"
echo "  $app (.$file_ext as \"$file_type_name\")"

# ---------------------------------------------------------------------------------------------
# 6. Sign
# ---------------------------------------------------------------------------------------------

if [[ $do_sign -eq 1 ]]; then
    if [[ -z $sign_id ]]; then
        # Only a "Developer ID Application" certificate produces something another Mac
        # will run; an "Apple Development" one signs fine here and is rejected
        # everywhere else, so picking one automatically would just move the failure to
        # the person you sent it to.
        sign_id="$(security find-identity -v -p codesigning \
            | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)"
        [[ -n $sign_id ]] || die "no 'Developer ID Application' identity in the keychain.
  Install one from developer.apple.com, or pass --sign-id, or --no-sign for a local-only build."
    fi
    say "codesign — $sign_id"
    # `--options runtime` is the hardened runtime, which notarization requires;
    # `--timestamp` gets a secure timestamp from Apple, which it also requires and
    # which is what keeps the signature valid after the certificate expires. No
    # entitlements: the viewer needs none, and every entitlement is a hole to justify.
    codesign --force --options runtime --timestamp --sign "$sign_id" "$app"
    codesign --verify --deep --strict --verbose=2 "$app"
else
    say "codesign — skipped (--no-sign)"
    echo "  This bundle will run on this Mac and be refused on every other one."
fi

# ---------------------------------------------------------------------------------------------
# 7. Notarize the app
# ---------------------------------------------------------------------------------------------
# Stapling the *app* as well as the .dmg matters: a stapled ticket is what lets it
# launch on a Mac that is offline or behind a filter. Without it Gatekeeper has to
# reach Apple on first launch, and the failure reads as "damaged and can't be opened"
# rather than "no network".

if [[ $do_notarize -eq 1 ]]; then
    say "notarize $name.app — profile '$notary_profile'"
    zip_path="$tmp/$name.zip"
    # ditto, not zip(1): only ditto preserves the bundle's symlinks and extended
    # attributes, and notarytool rejects an archive that has lost them.
    ditto -c -k --keepParent "$app" "$zip_path"
    notarize "$zip_path"
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
fi

# ---------------------------------------------------------------------------------------------
# 8. The disk image
# ---------------------------------------------------------------------------------------------

if [[ $do_dmg -eq 0 ]]; then
    say "done — $app"
    exit 0
fi

say "dmg"
mkdir -p "$out_dir"
dmg="$out_dir/$name-$version.dmg"
staging="$tmp/staging"
mkdir -p "$staging"
# ditto again, for the same reason: `cp -R` on a signed bundle can drop extended
# attributes and invalidate the signature.
ditto "$app" "$staging/$name.app"
# The drag-to-install target. A .dmg with only the app in it teaches people to run it
# from the disk image, where it stays read-only and every update re-downloads.
ln -s /Applications "$staging/Applications"
rm -f "$dmg"
# `hdiutil` prints a deprecation notice on macOS 26+, pointing at `diskutil image
# create`. Left as it is on purpose: the replacement does not exist on any earlier
# macOS, and a release script that only runs on the newest one is worse than a
# warning. UDZO is the compressed, read-only format every Mac can mount.
hdiutil create -volname "$name $version" -srcfolder "$staging" -ov -format UDZO "$dmg" >/dev/null
echo "  $dmg — $(stat -f%z "$dmg") bytes"

if [[ $do_sign -eq 1 ]]; then
    codesign --force --timestamp --sign "$sign_id" "$dmg"
fi

if [[ $do_notarize -eq 1 ]]; then
    say "notarize $(basename "$dmg")"
    notarize "$dmg"
    xcrun stapler staple "$dmg"
fi

# ---------------------------------------------------------------------------------------------
# 9. Verify what actually ships
# ---------------------------------------------------------------------------------------------

if [[ $do_sign -eq 1 ]]; then
    say "verify"
    codesign --verify --verbose=2 "$dmg"
fi

if [[ $do_notarize -eq 1 ]]; then
    xcrun stapler validate "$dmg"
    # The question the user's Mac will actually ask. `spctl` here is the same check
    # Gatekeeper runs on first launch, so a pass means the download opens with a
    # double-click.
    say "gatekeeper"
    spctl -a -t open --context context:primary-signature -vv "$dmg"
fi

say "done"
echo "  app: $app"
echo "  dmg: $dmg"
if [[ $do_notarize -eq 0 ]]; then
    echo
    echo "  NOT NOTARIZED — do not ship this. On another Mac it opens only via System Settings →"
    if [[ $do_sign -eq 0 ]]; then
        echo "  Privacy & Security → Open Anyway, and unsigned it may not open at all."
    else
        echo "  Privacy & Security → Open Anyway (macOS 15 removed the Control-click bypass)."
    fi
    echo "  Re-run without --no-notarize / --no-sign for a shippable build."
fi
