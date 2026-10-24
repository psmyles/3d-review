#!/usr/bin/env bash
# Build the Quick Look extensions and put them in an app bundle — Finder's
# thumbnails and Quick Look preview of an .fbx (docs/ARCHITECTURE.md, D23).
#
# Called by scripts/build-mac.sh (the release bundle) and scripts/dev-app.sh (the
# development one); not usually run by hand. It needs only the Command Line Tools:
# `swiftc` compiles the two extensions and `cargo` the dylib they share, with no
# Xcode project.
#
#   scripts/build-quicklook.sh <app> <bundle-id> [options]
#
# What it adds to <app>/Contents:
#
#   Frameworks/libreview_quicklook.dylib   crates/shell-quicklook: the renderer and
#                                          everything it embeds, shipped once
#   PlugIns/ReviewThumbnail.appex          QLThumbnailProvider (Finder thumbnails)
#   PlugIns/ReviewPreview.appex            QLPreviewingController (Space, and the
#                                          column view's preview)
#
# Each extension is a few hundred lines of Swift over the dylib's C ABI
# (crates/shell-quicklook/include/review_quicklook.h), sandboxed as Quick Look
# requires, and finds the dylib through an rpath into the containing app.
#
# Options
#   --debug              build the dylib unoptimized (dev-app.sh --debug).
#   --no-build           reuse the dylib already built.
#   --sign-id <identity> sign the dylib and both extensions with it, hardened
#                        runtime and a secure timestamp, as notarization requires.
#                        The default signs them ad hoc ("-"), which runs on this Mac
#                        only — enough for Quick Look to load them here.
#   --no-sign            leave them unsigned. Quick Look will not load them.
#
# The caller signs the app itself afterwards: nested code first, then the bundle.
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
crate="$repo/crates/shell-quicklook"

[[ $# -ge 2 ]] || { sed -n '2,33p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 2; }
app="$1"; bundle_id="$2"; shift 2
profile=release-shell
build=1
sign_id="-"
sign=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --debug)    profile=debug; shift ;;
        --no-build) build=0; shift ;;
        --sign-id)  sign_id="$2"; shift 2 ;;
        --no-sign)  sign=0; shift ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

die() { echo "error: $*" >&2; exit 1; }
[[ -d "$app/Contents" ]] || die "no app bundle at $app"

meta() { plutil -extract "$1" raw -o - "$repo/product.json"; }
name="$(meta productName)"
version="$(meta version)"
copyright="$(meta copyright)"
content_type="$(meta fileAssociation.macContentType)"
# The extensions' deployment target is the app's (LSMinimumSystemVersion).
min_macos=11.0

# ---------------------------------------------------------------------------------
# The dylib
# ---------------------------------------------------------------------------------
# `release-shell`, not `release`: the workspace's release profile aborts on panic,
# and every entry point of the dylib catches one instead, so a model that trips a
# bug costs one thumbnail rather than the extension process.

if [[ $build -eq 1 ]]; then
    if [[ $profile == debug ]]; then
        cargo build --manifest-path "$repo/Cargo.toml" -p review-shell-quicklook
    else
        cargo build --manifest-path "$repo/Cargo.toml" -p review-shell-quicklook --profile "$profile"
    fi
fi
dylib="$repo/target/$profile/libreview_quicklook.dylib"
[[ -f "$dylib" ]] || die "no dylib at $dylib — drop --no-build?"

frameworks="$app/Contents/Frameworks"
plugins="$app/Contents/PlugIns"
mkdir -p "$frameworks" "$plugins"
cp "$dylib" "$frameworks/"

# ---------------------------------------------------------------------------------
# The extensions
# ---------------------------------------------------------------------------------
# An app extension is an executable whose entry point is Foundation's
# NSExtensionMain, which reads NSExtensionPrincipalClass from the Info.plist and
# instantiates it — hence `-parse-as-library` (no Swift `main`) and `-e
# _NSExtensionMain`. `-application-extension` restricts the Swift side to
# extension-safe API, as Xcode does. The rpath climbs from
# <app>/Contents/PlugIns/X.appex/Contents/MacOS to <app>/Contents/Frameworks.

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# extension <executable> <bundle-id suffix> <extension point> <principal class>
#           <attributes plist fragment> <framework> <swift sources...>
extension() {
    local exe="$1" suffix="$2" point="$3" principal="$4" attributes="$5" framework="$6"
    shift 6
    local appex="$plugins/$exe.appex"
    rm -rf "$appex"
    mkdir -p "$appex/Contents/MacOS"
    xcrun swiftc -O -swift-version 5 -parse-as-library -application-extension \
        -target "arm64-apple-macos$min_macos" \
        -module-name "$exe" \
        -import-objc-header "$crate/include/review_quicklook.h" \
        "$@" \
        -L "$frameworks" -lreview_quicklook \
        -framework "$framework" \
        -Xlinker -e -Xlinker _NSExtensionMain \
        -Xlinker -rpath -Xlinker @executable_path/../../../../Frameworks \
        -o "$appex/Contents/MacOS/$exe" \
        || die "swiftc could not build $exe"
    cat > "$appex/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>         <string>$exe</string>
    <key>CFBundleIdentifier</key>         <string>$bundle_id.$suffix</string>
    <key>CFBundleName</key>               <string>$exe</string>
    <!-- What System Settings → Extensions → Quick Look lists it as. -->
    <key>CFBundleDisplayName</key>        <string>$name</string>
    <key>CFBundlePackageType</key>        <string>XPC!</string>
    <key>CFBundleShortVersionString</key> <string>$version</string>
    <key>CFBundleVersion</key>            <string>$version</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>NSHumanReadableCopyright</key>   <string>$copyright</string>
    <key>LSMinimumSystemVersion</key>     <string>$min_macos</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionAttributes</key>
        <dict>
            <key>QLSupportedContentTypes</key>
            <array>
                <string>$content_type</string>
            </array>
$attributes
        </dict>
        <key>NSExtensionPointIdentifier</key> <string>$point</string>
        <key>NSExtensionPrincipalClass</key>  <string>$principal</string>
    </dict>
</dict>
</plist>
PLIST
    plutil -lint "$appex/Contents/Info.plist" >/dev/null || die "the $exe Info.plist is malformed"
    echo "  $exe.appex ($point)"
}

extension ReviewThumbnail thumbnail com.apple.quicklook.thumbnail ReviewThumbnailProvider \
'            <!-- Every size Finder asks for, down to the list view'"'"'s 16 points. -->
            <key>QLThumbnailMinimumDimension</key> <integer>0</integer>' \
    QuickLookThumbnailing \
    "$crate/extension/ThumbnailProvider.swift" "$crate/extension/Image.swift"

extension ReviewPreview preview com.apple.quicklook.preview ReviewPreviewViewController \
'            <!-- A view the extension draws and takes input in, not a file it hands back. -->
            <key>QLIsDataBasedPreview</key> <false/>
            <key>QLSupportsSearchableItems</key> <false/>' \
    QuickLookUI \
    "$crate/extension/PreviewViewController.swift" "$crate/extension/Image.swift"

# ---------------------------------------------------------------------------------
# Signing
# ---------------------------------------------------------------------------------
# Inside out: the dylib, then each extension with the sandbox entitlement Quick
# Look requires. The app is the caller's, signed last.

if [[ $sign -eq 1 ]]; then
    flags=(--force --sign "$sign_id")
    # A Developer ID signature gets the hardened runtime and a secure timestamp,
    # which notarization requires; an ad-hoc one can have neither.
    [[ "$sign_id" == "-" ]] || flags+=(--options runtime --timestamp)
    codesign "${flags[@]}" "$frameworks/libreview_quicklook.dylib"
    for appex in "$plugins/ReviewThumbnail.appex" "$plugins/ReviewPreview.appex"; do
        codesign "${flags[@]}" --entitlements "$crate/extension/QuickLook.entitlements" "$appex"
    done
    echo "  signed ($([[ "$sign_id" == "-" ]] && echo "ad hoc" || echo "$sign_id"))"
fi
