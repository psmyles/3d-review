/*
 * The calls the two bridge translation units make into each other.
 *
 * The bridge is split across `ufbx_bridge.c` (the geometry walk and the one
 * entry point Rust calls) and `ufbx_extras.c` (the source-property capture it
 * runs inside that call). Everything each exposes to the other is declared
 * here, once, so a changed signature is a compile error in both files rather
 * than a hand-copied prototype that silently drifts. Nothing in this header is
 * part of the C ABI Rust binds: that is `ufbx_bridge.h` + `ufbx_extras.h`.
 */
#ifndef REVIEW_UFBX_INTERNAL_H
#define REVIEW_UFBX_INTERNAL_H

#include "ufbx_bridge.h"

#include "ufbx.h"

/* Defined in ufbx_bridge.c: writes `message` into the bridge's error record. */
void review_import_set_error_message(review_import_error *out_error, const char *message);

/* Defined in ufbx_extras.c: captures every source property the exporter gives
   back, indexed like the geometry pass's output. Returns 1 on success, 0 with
   `out` freed and `out_error` set; the parameters are documented there. */
int review_import_capture_extras(const ufbx_scene *scene,
                                 const ufbx_material *const *material_sources,
                                 size_t material_count,
                                 const uint32_t *channel_of_element,
                                 const int32_t *clip_of_stack,
                                 review_import_extras *out,
                                 review_import_error *out_error);

#endif
