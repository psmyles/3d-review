// The Quick Look preview extension: the model in the Quick Look panel (Space in
// Finder) and the column view's preview, drawn by libreview_quicklook
// (crates/shell-quicklook) and moved as in the viewer - left drag orbits, right
// or middle drag pans, Option + right drag and the wheel or a pinch zoom, a
// double-click or F reframes.
//
// The AppKit half only: a layer showing the frames the dylib draws, the timer
// that keeps drawing while the picture settles, and mouse, trackpad and key
// events turned into the pixel positions and buttons it takes. The model, the
// camera and the GPU are the dylib's.

import Cocoa
import QuickLookUI

// Named for the Objective-C runtime, so the extension's Info.plist can name the
// principal class without a Swift module prefix.
@objc(ReviewPreviewViewController)
final class PreviewViewController: NSViewController, QLPreviewingController {
    private let preview = PreviewView()

    override func loadView() {
        view = preview
    }

    func preparePreviewOfFile(at url: URL, completionHandler handler: @escaping (Error?) -> Void) {
        // Answered at once: the view says "Loading..." itself, as the Explorer
        // pane does, rather than leaving Quick Look's spinner up for a big file.
        preview.open(url)
        handler(nil)
    }
}

/// What the loading worker's callback reaches the view through. Retained across
/// the callback, and weak to the view, so a preview closed mid-load is simply
/// not woken.
private final class LoadToken {
    weak var view: PreviewView?
    init(_ view: PreviewView) { self.view = view }
}

/// Called once, on the dylib's loading worker.
private let previewLoaded: ReviewQlLoaded = { context in
    guard let context else { return }
    let token = Unmanaged<LoadToken>.fromOpaque(context).takeRetainedValue()
    DispatchQueue.main.async { token.view?.loadFinished() }
}

final class PreviewView: NSView {
    private var handle: OpaquePointer?
    private let status = NSTextField(labelWithString: "")
    private var settleTimer: Timer?

    /// The status line's colour on the black backdrop: the Explorer pane's
    /// (`shell-windows/src/preview/pane.rs`, `TEXT`).
    private static let statusColor = NSColor(srgbRed: 160 / 255, green: 160 / 255, blue: 160 / 255, alpha: 1)
    /// How often a settling picture is redrawn: the Explorer pane's 16 ms.
    private static let settleInterval: TimeInterval = 0.016

    init() {
        super.init(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
        wantsLayer = true
        layerContentsRedrawPolicy = .never
        status.textColor = Self.statusColor
        status.alignment = .center
        status.translatesAutoresizingMaskIntoConstraints = false
        addSubview(status)
        NSLayoutConstraint.activate([
            status.centerXAnchor.constraint(equalTo: centerXAnchor),
            status.centerYAnchor.constraint(equalTo: centerYAnchor),
            status.leadingAnchor.constraint(greaterThanOrEqualTo: leadingAnchor, constant: 8),
            status.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -8),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    deinit {
        settleTimer?.invalidate()
        review_ql_preview_close(handle)
    }

    // The viewer's backdrop in either appearance (see `BACKGROUND` in the dylib).
    override func makeBackingLayer() -> CALayer {
        let layer = CALayer()
        layer.backgroundColor = CGColor(gray: 0, alpha: 1)
        layer.contentsGravity = .resize
        return layer
    }

    override var isFlipped: Bool { true }
    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func open(_ url: URL) {
        review_ql_preview_close(handle)
        handle = nil
        let token = Unmanaged.passRetained(LoadToken(self))
        handle = url.withUnsafeFileSystemRepresentation { path in
            review_ql_preview_open(path, previewLoaded, token.toOpaque())
        }
        if handle == nil {
            token.release()
        }
        resizeToBacking()
        present()
    }

    fileprivate func loadFinished() {
        guard review_ql_preview_finish_load(handle) else { return }
        resizeToBacking()
        review_ql_preview_draw(handle)
        present()
    }

    // MARK: Size

    override func layout() {
        super.layout()
        resizeToBacking()
        review_ql_preview_draw(handle)
        present()
    }

    override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        layer?.contentsScale = backingScale
        resizeToBacking()
        review_ql_preview_draw(handle)
        present()
    }

    /// Points to pixels. Not `convertToBacking`: backing space counts *up* from
    /// the bottom whether or not the view is flipped, so it negates every y in
    /// this one — which ran each vertical drag backwards.
    private var backingScale: CGFloat { window?.backingScaleFactor ?? 1 }

    /// The view's size in pixels: what the dylib draws at, and what pointer
    /// positions are measured in, as the viewer's are.
    private func resizeToBacking() {
        let scale = backingScale
        review_ql_preview_resize(
            handle,
            UInt32(max((bounds.width * scale).rounded(), 1)),
            UInt32(max((bounds.height * scale).rounded(), 1)))
    }

    // MARK: Showing frames

    /// Show the last frame drawn, or the status line, and keep the settle timer
    /// running while the picture is still changing.
    private func present() {
        var frame = ReviewQlImage()
        if review_ql_preview_frame(handle, &frame), let image = makeImage(frame, alpha: .noneSkipLast) {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            layer?.contents = image
            CATransaction.commit()
        }
        if let text = review_ql_preview_status_text(handle) {
            status.stringValue = String(cString: text)
            status.isHidden = false
            layer?.contents = nil
        } else {
            status.isHidden = true
        }
        if review_ql_preview_is_settling(handle) {
            if settleTimer == nil {
                let timer = Timer(timeInterval: Self.settleInterval, repeats: true) { [weak self] _ in
                    self?.tick()
                }
                // Common modes, so the picture keeps settling while the panel is
                // being resized or a menu is open.
                RunLoop.main.add(timer, forMode: .common)
                settleTimer = timer
            }
        } else {
            settleTimer?.invalidate()
            settleTimer = nil
        }
    }

    private func tick() {
        review_ql_preview_tick(handle)
        present()
    }

    // MARK: Pointer

    /// `event`'s position in pixels from the view's top left (the view is
    /// flipped, so `convert` already counts down from the top).
    private func position(of event: NSEvent) -> CGPoint {
        let point = convert(event.locationInWindow, from: nil)
        return CGPoint(x: point.x * backingScale, y: point.y * backingScale)
    }

    private func press(_ button: Int, _ event: NSEvent) {
        window?.makeFirstResponder(self)
        let at = position(of: event)
        let flags = event.modifierFlags
        review_ql_preview_press(
            handle, UInt32(button), Float(at.x), Float(at.y), flags.contains(.shift), flags.contains(.option))
    }

    private func drag(_ event: NSEvent) {
        let at = position(of: event)
        if review_ql_preview_pointer_moved(handle, Float(at.x), Float(at.y)) {
            present()
        }
    }

    /// The left button; Control + click is the Mac's right click.
    private var leftIsRight = false

    override func mouseDown(with event: NSEvent) {
        if event.clickCount == 2 {
            review_ql_preview_reframe(handle)
            present()
            return
        }
        leftIsRight = event.modifierFlags.contains(.control)
        press(leftIsRight ? REVIEW_QL_BUTTON_RIGHT : REVIEW_QL_BUTTON_LEFT, event)
    }

    override func mouseDragged(with event: NSEvent) { drag(event) }

    override func mouseUp(with event: NSEvent) {
        review_ql_preview_release(handle, UInt32(leftIsRight ? REVIEW_QL_BUTTON_RIGHT : REVIEW_QL_BUTTON_LEFT))
    }

    override func rightMouseDown(with event: NSEvent) { press(REVIEW_QL_BUTTON_RIGHT, event) }
    override func rightMouseDragged(with event: NSEvent) { drag(event) }
    override func rightMouseUp(with event: NSEvent) {
        review_ql_preview_release(handle, UInt32(REVIEW_QL_BUTTON_RIGHT))
    }

    override func otherMouseDown(with event: NSEvent) {
        if event.buttonNumber == 2 { press(REVIEW_QL_BUTTON_MIDDLE, event) }
    }
    override func otherMouseDragged(with event: NSEvent) { drag(event) }
    override func otherMouseUp(with event: NSEvent) {
        if event.buttonNumber == 2 { review_ql_preview_release(handle, UInt32(REVIEW_QL_BUTTON_MIDDLE)) }
    }

    override func scrollWheel(with event: NSEvent) {
        // As winit reports them to the viewer: a trackpad's precise deltas in
        // pixels, a wheel's in notches, both positive away from the user.
        if event.hasPreciseScrollingDeltas {
            let scale = backingScale
            review_ql_preview_scroll_pixels(handle, Float(event.scrollingDeltaY * scale))
        } else {
            review_ql_preview_scroll_lines(handle, Float(event.scrollingDeltaY))
        }
        present()
    }

    override func magnify(with event: NSEvent) {
        review_ql_preview_pinch(handle, Float(event.magnification))
        present()
    }

    // MARK: Keys

    override func keyDown(with event: NSEvent) {
        let reframe =
            event.charactersIgnoringModifiers?.lowercased() == "f"
            || event.specialKey == .home
        guard reframe else {
            super.keyDown(with: event)
            return
        }
        review_ql_preview_reframe(handle)
        present()
    }
}
