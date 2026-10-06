#!/usr/bin/env bash
# Build, sign, notarize and package 3D Review for macOS — the twin of
# packaging/build-windows-installer.ps1 (docs/ARCHITECTURE.md, Platform decisions D12).
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
# It edits two tracked files: like the Windows installer script, it writes
# product.json's version into crates/app/Cargo.toml and refreshes Cargo.lock before
# building, so neither can drift from the version the app and the bundle report.
# Everything else it writes goes to target/ and dist/.
#
# Options
#   --sign-id <identity>       codesign identity; default is the "Developer ID
#                              Application" one in the keychain, which is the only kind
#                              Gatekeeper accepts outside the App Store.
#   --no-sign                  skip codesign entirely. The bundle runs here and nowhere
#                              else. Implies --no-notarize. Never ship this.
#   --notarize-profile <name>  use a different `xcrun notarytool store-credentials`
#                              profile. The default is account-level, not per-product
#                              — see the note beside `notary_profile` below.
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
# The `xcrun notarytool store-credentials` profile to use when none is named.
#
# **Not derived from the product name**, which is the obvious thing to do and is
# wrong: what the profile holds is an Apple ID, a team ID and an app-specific
# password, and all three belong to the *account*, not to a product. One profile
# notarizes everything this team signs, and `fire` is what that one profile happens to
# be called here because it was created for the first product to need it. A
# `3dreview`-shaped default would have meant every release of this repo carrying a
# `--notarize-profile` flag to reach a credential it already had.
#
# Change this only if the profile is renamed or replaced; an app-specific password
# cannot be read back once created, so re-creating one under a new name means minting
# a fresh password at appleid.apple.com (which does not invalidate the old one).
#
# The credential lives in the data-protection keychain, which `security(1)` cannot
# even enumerate — so there is no point pre-flighting it here; notarytool's own error
# is immediate and says exactly what is missing.
notary_profile="${notary_profile:-fire}"

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
#
# Cargo.lock records the member's version too, and is committed. `cargo update
# --workspace` re-resolves the workspace members *only*, so the lock picks the new
# version up while every registry dependency stays pinned exactly as it was — which a
# bare `cargo generate-lockfile` would not guarantee. Deliberately not `--offline`: on
# a machine whose registry cache is cold that would fail here, before the build that
# would have populated it.

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

# Run it unconditionally: the manifest can already be in sync while the lock is not
# (someone edited Cargo.toml by hand), and with both in sync it is a no-op.
cargo update --manifest-path "$repo/Cargo.toml" --workspace --quiet \
    || die "could not refresh Cargo.lock for version $version"
lock_version="$(awk '/^name = "review-app"$/ { getline; print }' "$repo/Cargo.lock" \
    | sed -n -E 's/^version = "([^"]*)".*/\1/p' | head -1)"
[[ "$lock_version" == "$version" ]] \
    || die "Cargo.lock still records review-app $lock_version, not $version"
echo "  Cargo.lock: review-app $lock_version."

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
# `assets/mac_icon/icon.icon` is an Icon Composer document (layers, fill, specular, the
# Liquid Glass treatment), and `actool` is what compiles one. It writes two things: the
# `Assets.car` macOS 26 renders the layered icon from, and a flattened `.icns` fallback for
# everything older and for the places that only read `CFBundleIconFile` (Finder's Get Info,
# document-type icons). Rebuilt every run so a change to the document cannot be silently
# missing from a release. Needs full Xcode, not just the Command Line Tools.

say "icon"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
icon_doc="$repo/assets/mac_icon/icon.icon"
icon_name="$(basename "$icon_doc" .icon)"
[[ -d "$icon_doc" ]] || die "missing $icon_doc"
mkdir -p "$tmp/icon"
xcrun actool "$icon_doc" --compile "$tmp/icon" --output-format human-readable-text \
    --notices --warnings --errors --output-partial-info-plist "$tmp/icon/partial.plist" \
    --app-icon "$icon_name" --include-all-app-icons --enable-on-demand-resources NO \
    --development-region en --target-device mac --minimum-deployment-target 11.0 \
    --platform macosx >/dev/null \
    || die "actool could not compile $icon_doc (needs full Xcode, not just the CLT)"
[[ -f "$tmp/icon/Assets.car" && -f "$tmp/icon/$icon_name.icns" ]] \
    || die "actool produced no Assets.car / $icon_name.icns"
icns="$tmp/icon/$icon_name.icns"
car="$tmp/icon/Assets.car"
echo "  Assets.car — $(stat -f%z "$car") bytes, $icon_name.icns — $(stat -f%z "$icns") bytes"

# ---------------------------------------------------------------------------------------------
# 5. The bundle
# ---------------------------------------------------------------------------------------------

app="$repo/target/$name.app"
say "bundle -> $app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/$name"
cp "$icns" "$app/Contents/Resources/$name.icns"
cp "$car" "$app/Contents/Resources/Assets.car"

# The manual's images. Its page *text* is compiled into the binary (invariant 12),
# so this is the only docs payload — and an bundle without it still shows every
# page, each image rendering as its alt text. `app`'s `docs_dir()` looks here,
# beside the binary's own directory.
if [[ -d "$repo/docs/book/src/en/images" ]]; then
  mkdir -p "$app/Contents/Resources/docs/en"
  cp -R "$repo/docs/book/src/en/images" "$app/Contents/Resources/docs/en/"
fi
# The license notices. The binary is statically linked, so it carries code from every
# project in docs/THIRD-PARTY-NOTICES.md, and their licenses require the notices to
# travel with it - the twin of the Windows installer's [Files] entries. Not optional:
# a bundle without them is non-compliant, so a missing one stops the build here.
for notice in LICENSE docs/THIRD-PARTY-NOTICES.md docs/CREDITS.md; do
  [[ -f "$repo/$notice" ]] || { echo "missing license file the bundle must ship: $notice" >&2; exit 1; }
done
cp "$repo/LICENSE" "$app/Contents/Resources/LICENSE.txt"
cp "$repo/docs/THIRD-PARTY-NOTICES.md" "$repo/docs/CREDITS.md" "$app/Contents/Resources/"
mkdir -p "$app/Contents/Resources/licenses"
cp "$repo"/licenses/*.txt "$app/Contents/Resources/licenses/"
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
    <!-- Names the layered icon in Assets.car (macOS 26); CFBundleIconFile is the fallback. -->
    <key>CFBundleIconName</key>           <string>$icon_name</string>
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
# Named <exeName>-<version>.dmg, the same scheme as the Windows installer.
dmg="$out_dir/$exe-$version.dmg"
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
# 9. The .dmg's own Finder icon
# ---------------------------------------------------------------------------------------------
# Without this the .dmg gets the generic disk-image document icon. What goes on instead
# is that same generic icon with the cube composited over its disk graphic, so it still
# reads as a disk image at a glance and is unmistakably ours. The base comes from
# NSWorkspace rather than a checked-in PNG, so it is whatever the running macOS draws
# for a .dmg and cannot go stale.
#
# This runs *last*, after stapling, because a custom file icon is not in the file's
# data: it is a resource fork in the `com.apple.ResourceFork` xattr plus a flag in
# `com.apple.FinderInfo`, and codesign, notarytool and stapler all rewrite the .dmg.
# Applied here it costs the signature nothing — section 10 re-checks it to prove that.
#
# Know one limit before relying on it: xattrs travel only where the transport carries
# them, which means AirDrop, a `ditto`-made zip, or a copy between Macs. A plain HTTPS
# download strips the fork and the .dmg lands generic again. This is
# cosmetic-on-your-Mac, not branding for the web.

say "dmg icon"
dmg_iconset="$tmp/dmg.iconset"
mkdir -p "$dmg_iconset"
cat > "$tmp/dmg-icon.js" <<'JS'
ObjC.import('AppKit');

// The cube's edge, and its centre measured from the top-left, as fractions of the
// icon's edge. It replaces the generic download arrow, and it has to stay inside the
// disk graphic drawn on the document — the cube overhanging that frame reads as a
// mistake rather than as a badge.
//
// Fitted to the disk's interior, measured off the system artwork at 512²: x 165..348
// and y 146..338 (338 being the top of the darker bar along its bottom), so a
// 183 x 192 opening. Unlike a tall narrow logo this master is full-bleed in *both*
// directions — its ink is 996 x 1024 of 1024² — so both dimensions bind at once and
// the horizontal margin is the tighter of the two: 0.33 draws the ink 169 tall and
// 164 wide, leaving ~12px on every side. Raising it much past 0.35 starts crowding
// the disk's edge.
var LOGO_SCALE = 0.33, LOGO_CX = 0.5, LOGO_CY = 0.473;

// argv: <logo.png> <size>:<dst.png>...
function run(argv) {
    var img = $.NSImage.alloc.initWithContentsOfFile(argv[0]);
    if (img.isNil()) throw new Error('cannot read ' + argv[0]);
    // Deprecated in favour of -iconForContentType:, which does not bridge cleanly
    // through JXA. It still returns the current system artwork, with reps up to 2048²,
    // so 1024² is a downsample rather than an upscale.
    var base = $.NSWorkspace.sharedWorkspace.iconForFileType('dmg');

    argv.slice(1).forEach(function (spec) {
        var colon = spec.indexOf(':');
        var size = parseInt(spec.slice(0, colon), 10), dst = spec.slice(colon + 1);

        var rep = $.NSBitmapImageRep.alloc
            .initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
                $(), size, size, 8, 4, true, false, $.NSCalibratedRGBColorSpace, 0, 0);
        rep = rep.bitmapImageRepByRetaggingWithColorSpace($.NSColorSpace.sRGBColorSpace);

        $.NSGraphicsContext.saveGraphicsState;
        $.NSGraphicsContext.setCurrentContext(
            $.NSGraphicsContext.graphicsContextWithBitmapImageRep(rep));
        $.NSGraphicsContext.currentContext.setImageInterpolation(3); // .high
        base.drawInRectFromRectOperationFraction(
            $.NSMakeRect(0, 0, size, size), $.NSZeroRect, $.NSCompositingOperationSourceOver, 1.0);
        var e = size * LOGO_SCALE;
        img.drawInRectFromRectOperationFraction(
            $.NSMakeRect(size * LOGO_CX - e / 2, size * (1 - LOGO_CY) - e / 2, e, e),
            $.NSZeroRect, $.NSCompositingOperationSourceOver, 1.0);
        $.NSGraphicsContext.restoreGraphicsState;

        var png = rep.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $({}));
        if (!png.writeToFileAtomically(dst, true)) throw new Error('cannot write ' + dst);
    });
}
JS

dmg_renders=()
for size in 16 32 128 256 512; do
    dmg_renders+=("$size:$dmg_iconset/icon_${size}x${size}.png")
    dmg_renders+=("$((size * 2)):$dmg_iconset/icon_${size}x${size}@2x.png")
done
# The *transparent* master here, not the flattened tile: the cube is being badged onto
# the system's disk graphic, and a grey square would cover it.
osascript -l JavaScript "$tmp/dmg-icon.js" "$repo/assets/icons/application-logo.png" \
    "${dmg_renders[@]}" || die "could not render the .dmg icon"
(( $(ls "$dmg_iconset" | wc -l) == ${#dmg_renders[@]} )) || die "the .dmg iconset is short of images"
dmg_icns="$tmp/dmg.icns"
iconutil -c icns "$dmg_iconset" -o "$dmg_icns"

# -setIcon:forFile:options: writes the fork and sets the flag in one call, which is the
# whole reason not to do this with Rez and SetFile.
osascript -l JavaScript -e '
    ObjC.import("AppKit");
    function run(argv) {
        var img = $.NSImage.alloc.initWithContentsOfFile(argv[0]);
        if (img.isNil()) throw new Error("cannot read " + argv[0]);
        if (!$.NSWorkspace.sharedWorkspace.setIconForFileOptions(img, argv[1], 0))
            throw new Error("setIcon:forFile: refused " + argv[1]);
    }' "$dmg_icns" "$dmg" || die "could not set the .dmg's Finder icon"
echo "  applied to $(basename "$dmg")"

# ---------------------------------------------------------------------------------------------
# 10. Verify what actually ships
# ---------------------------------------------------------------------------------------------
# Deliberately after the icon, not before: these have to be true of the bytes that leave
# the Mac, and the icon is the last thing to touch them.

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
