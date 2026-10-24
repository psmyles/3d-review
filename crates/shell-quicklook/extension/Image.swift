// What both Quick Look extensions share: turning the dylib's RGBA image into a
// CGImage. Compiled into each of them (scripts/build-quicklook.sh).

import CoreGraphics

/// A copy of `image` as a CGImage.
func makeImage(_ image: ReviewQlImage, alpha: CGImageAlphaInfo) -> CGImage? {
    guard let rgba = image.rgba, image.width > 0, image.height > 0,
        image.len == Int(image.width) * Int(image.height) * 4,
        let data = CFDataCreate(nil, rgba, image.len),
        let provider = CGDataProvider(data: data),
        let space = CGColorSpace(name: CGColorSpace.sRGB)
    else { return nil }
    return CGImage(
        width: Int(image.width),
        height: Int(image.height),
        bitsPerComponent: 8,
        bitsPerPixel: 32,
        bytesPerRow: Int(image.width) * 4,
        space: space,
        bitmapInfo: CGBitmapInfo(rawValue: alpha.rawValue),
        provider: provider,
        decode: nil,
        shouldInterpolate: true,
        intent: .defaultIntent)
}
