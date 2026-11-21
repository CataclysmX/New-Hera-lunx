/*
 * Full-screen custom background (480x272, 8-bit indexed + RGB565 palette) - pure C11, no I/O.
 * Port of `hsm01_proto::background` (same median cut, same results: tests/test_background.c).
 *
 * This is also the source of the boot logo: once the 32 tiles are on the device,
 * hsm01_cmd_splash_flash(b, true) (0x50 0x80) writes the current background to flash.
 *
 * Upload (official DLL, firmware 0x33 / 0x37 handlers):
 *  - 0x33 palette (256 RGB565 entries at most),
 *  - 32 x 0x37 tiles of 4080 bytes: progressive interlacing - tile t carries the pixels at
 *    (8*col + dx, 4*row + dy) for 68 rows x 60 columns, the firmware fills a w x h block for each
 *    sample so the picture refines pass after pass (hsm01_bg_tiles),
 *  - the firmware keeps a bitmask of received tiles; 0x50 0x80 is refused (status error 0x100)
 *    unless all 32 tiles were received.
 */
#ifndef HSM01_BACKGROUND_H
#define HSM01_BACKGROUND_H

#include <stddef.h>
#include <stdint.h>

#include "hsm01/proto.h"

#ifdef __cplusplus
extern "C" {
#endif

#define HSM01_BG_TILES 32
#define HSM01_BG_TILE_BYTES ((HSM01_SCREEN_H / 4) * (HSM01_SCREEN_W / 8)) /* 4080 */

/* (dx, dy, block_w, block_h) of each tile (DLL table @0x18010fbc0). */
extern const uint8_t hsm01_bg_tiles[HSM01_BG_TILES][4];

/* A 480x272 indexed picture (about 128 KiB: allocate it, do not put it on the stack). */
typedef struct {
    uint16_t palette[256]; /* RGB565 */
    size_t colors;         /* entries used in `palette`, 256 at most */
    uint8_t pixels[HSM01_SCREEN_W * HSM01_SCREEN_H]; /* palette indices, row major */
} hsm01_indexed_image;

/* Median-cut quantization of 480x272 ARGB8888 pixels (alpha ignored) to 256 colours at most.
 * HSM01_OK, or HSM01_ERR_NOMEM. */
int hsm01_quantize(const uint32_t *argb, hsm01_indexed_image *out);

/* The 4080 samples of tile t (0..31). */
void hsm01_bg_tile(const hsm01_indexed_image *img, int t, uint8_t out[HSM01_BG_TILE_BYTES]);

/* The picture the firmware shows after receiving tiles 0..last (preview, tests): 480x272
 * palette indices in `fb`. */
void hsm01_bg_render_progressive(const hsm01_indexed_image *img, int last, uint8_t *fb);

#ifdef __cplusplus
}
#endif
#endif
