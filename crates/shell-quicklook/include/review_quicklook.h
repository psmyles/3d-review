/*
 * review_quicklook.h - the C ABI of libreview_quicklook.dylib
 * (crates/shell-quicklook), which the two Quick Look extensions call.
 *
 * Hand-kept against src/thumbnail.rs, src/preview.rs and src/image.rs: every
 * declaration here has exactly one `extern "C"` definition there, and nothing
 * else is exported.
 *
 * Images are RGBA8, top row first, `width * 4` bytes a row with no padding, in
 * the sRGB space. A thumbnail is *straight* alpha over nothing (the model alone,
 * as Finder lays it over its own background); a preview frame is opaque, already
 * laid over the viewer's black backdrop.
 *
 * Threads: every function may be called from any thread, and the GPU work is
 * serialized inside. A preview handle is not itself synchronized - call its
 * functions from one thread at a time (the view's main thread).
 */

#ifndef REVIEW_QUICKLOOK_H
#define REVIEW_QUICKLOOK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ReviewQlImage {
    uint32_t width;
    uint32_t height;
    /* `len` bytes: width * height * 4. */
    const uint8_t *rgba;
    size_t len;
} ReviewQlImage;

/* ---- Thumbnails -------------------------------------------------------- */

/*
 * Draw `path`'s thumbnail, `side` pixels square (capped at 1024), into `out`.
 * False - with `out` left empty - when there is no thumbnail to give: the file
 * cannot be read, has nothing to draw, or there is no GPU. Free a filled `out`
 * with review_ql_image_free.
 */
bool review_ql_thumbnail(const char *path, uint32_t side, ReviewQlImage *out);

/* Release what review_ql_thumbnail filled in, and empty it. */
void review_ql_image_free(ReviewQlImage *image);

/* ---- Interactive previews ---------------------------------------------- */

typedef struct ReviewQlPreview ReviewQlPreview;

/* Called exactly once, on a worker thread, when the model has loaded or failed
 * to: hop to the view's thread and call review_ql_preview_finish_load. */
typedef void (*ReviewQlLoaded)(void *context);

enum {
    REVIEW_QL_BUTTON_LEFT = 0,
    REVIEW_QL_BUTTON_RIGHT = 1,
    REVIEW_QL_BUTTON_MIDDLE = 2,
};

/* Start loading `path`. NULL only for a NULL path, in which case `loaded` is
 * never called. `loaded` may itself be NULL. */
ReviewQlPreview *review_ql_preview_open(const char *path, ReviewQlLoaded loaded, void *context);

/* Release the preview and its GPU resources. NULL is ignored. */
void review_ql_preview_close(ReviewQlPreview *preview);

/* Take what the load produced. True when there is something new to show. */
bool review_ql_preview_finish_load(ReviewQlPreview *preview);

/* What to say while there is no picture (UTF-8), or NULL once there is one.
 * Valid until the next call on this preview. */
const char *review_ql_preview_status_text(ReviewQlPreview *preview);

/* The view is now width x height pixels. Draws nothing. */
void review_ql_preview_resize(ReviewQlPreview *preview, uint32_t width, uint32_t height);

/* Draw a frame at the current size. */
void review_ql_preview_draw(ReviewQlPreview *preview);

/* The last frame drawn at the current size, borrowed until the next call on
 * this preview. False when there is none. */
bool review_ql_preview_frame(const ReviewQlPreview *preview, ReviewQlImage *out);

/* Whether the picture is still changing on its own (a camera glide, ambient
 * occlusion averaging in): run a ~60 Hz timer calling review_ql_preview_tick
 * while it is. */
bool review_ql_preview_is_settling(const ReviewQlPreview *preview);
void review_ql_preview_tick(ReviewQlPreview *preview);

/* Whether a button is down. */
bool review_ql_preview_is_dragging(const ReviewQlPreview *preview);

/* Pointer input, positions in pixels from the view's top left. Each of these
 * draws when the camera moved; pointer_moved says whether it did. */
void review_ql_preview_press(ReviewQlPreview *preview, uint32_t button, float x, float y,
                             bool shift, bool alt);
void review_ql_preview_release(ReviewQlPreview *preview, uint32_t button);
void review_ql_preview_cancel_drag(ReviewQlPreview *preview);
bool review_ql_preview_pointer_moved(ReviewQlPreview *preview, float x, float y);
/* A wheel's notches, positive away from the user (zoom in). */
void review_ql_preview_scroll_lines(ReviewQlPreview *preview, float notches);
/* A trackpad's precise scroll, in pixels, positive away from the user. */
void review_ql_preview_scroll_pixels(ReviewQlPreview *preview, float pixels);
/* A trackpad pinch: the magnification fraction, positive as the fingers spread
 * (zoom in). */
void review_ql_preview_pinch(ReviewQlPreview *preview, float scale);
/* Glide back to a framing of the whole model (F, or a double-click). */
void review_ql_preview_reframe(ReviewQlPreview *preview);

#ifdef __cplusplus
}
#endif

#endif /* REVIEW_QUICKLOOK_H */
