/*
 * Pictures for the CLI (backgrounds, boot logo, custom-firmware images), as the Rust CLI's
 * `load_image`: any PNG / JPEG / GIF (first frame) / BMP / PPM, scaled keeping its proportions to
 * fill 480x272 (Lanczos 3) and cropped in the centre. PNG, JPEG and GIF need libpng, libjpeg and
 * giflib at build time (HSM01_HAVE_PNG / _JPEG / _GIF); BMP and PPM are read here.
 */
#ifndef HSM01_IMAGE_H
#define HSM01_IMAGE_H

#include <stddef.h>
#include <stdint.h>

/* The formats this build reads, e.g. "PNG, JPEG, GIF, BMP, PPM". */
const char *hsm01_image_formats(void);

/* Decode `path` to 8-bit RGBA (w * h * 4 bytes, to free()); NULL and a message in `err`. */
uint8_t *hsm01_image_decode(const char *path, int *w, int *h, char *err, size_t errlen);

/* Scale `rgba` (w x h) keeping its proportions to fill ow x oh, crop the centre: ARGB8888
 * (alpha ignored, as the Rust CLI) in `argb` (ow * oh). 0, or -1 when out of memory. */
int hsm01_image_fill(const uint8_t *rgba, int w, int h, int ow, int oh, uint32_t *argb);

/* Both, to the screen size (480 x 272): 0, or -1 and a message in `err`. */
int hsm01_load_image(const char *path, uint32_t *argb, char *err, size_t errlen);

#endif
