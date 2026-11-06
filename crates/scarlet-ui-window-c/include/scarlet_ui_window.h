#ifndef SCARLET_UI_WINDOW_H
#define SCARLET_UI_WINDOW_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* All operations and destruction use the creating UI thread. Rendered pixels
 * and clip rectangles remain borrowed until the next call on that frame.
 * Pixel format is premultiplied little-endian BGRA, dimensions physical pixels.
 * These functions own no platform window, SWS connection or application image. */
typedef struct ScarletUiFrame ScarletUiFrame;
typedef struct ScarletUiRect { uint32_t x, y, width, height; } ScarletUiRect;
typedef struct ScarletUiInsets { uint32_t left, top, right, bottom; } ScarletUiInsets;
typedef struct ScarletUiFrameView {
    uint32_t width, height, stride, margin;
    ScarletUiInsets insets;
    const void *pixels;
    size_t bytes;
    const ScarletUiRect *clip;
    size_t clip_count;
} ScarletUiFrameView;
enum { SCARLET_UI_FRAME_DECORATED=1, SCARLET_UI_FRAME_RESIZABLE=2 };
enum { SCARLET_UI_FRAME_CLOSE=1, SCARLET_UI_FRAME_MOVE=2,
       SCARLET_UI_FRAME_MINIMIZE=4, SCARLET_UI_FRAME_TOGGLE_MAXIMIZE=8 };
enum { SCARLET_UI_POINTER_MOVE=0, SCARLET_UI_POINTER_PRESS=1, SCARLET_UI_POINTER_RELEASE=2 };
/* Returns the ABI version (1), not the ScarletUI package version. */
uint32_t scarlet_ui_frame_abi(void);
int32_t scarlet_ui_frame_insets(uint32_t scale_milli, uint32_t flags, ScarletUiInsets *out);
ScarletUiFrame *scarlet_ui_frame_create(void);
int32_t scarlet_ui_frame_configure(ScarletUiFrame *, const char *title, uint32_t width,
                                  uint32_t height, uint32_t scale_milli, uint32_t flags);
int32_t scarlet_ui_frame_render(ScarletUiFrame *, ScarletUiFrameView *out);
int32_t scarlet_ui_frame_pointer(ScarletUiFrame *, uint32_t kind, int32_t x, int32_t y,
                                uint32_t *actions);
int32_t scarlet_ui_frame_dirty(ScarletUiFrame *);
void scarlet_ui_frame_destroy(ScarletUiFrame *);
#ifdef __cplusplus
}
#endif
#endif
