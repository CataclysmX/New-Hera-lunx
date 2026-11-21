/* Unit tests for background.h, as the Rust crate's (hsm01-proto/src/background.rs): tiles,
 * progressive reconstruction, exact few-colour palettes, command lengths, and the same
 * quantization results as the Rust port (FNV-1a 64 fingerprints of palette + pixels). */
#include "hsm01/background.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;
#define CHECK(cond) \
    do { \
        if (!(cond)) { \
            fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++; \
        } \
    } while (0)

#define W HSM01_SCREEN_W
#define H HSM01_SCREEN_H

/* (colours, fingerprint) of the Rust quantize() on the same pictures (background.rs tests). */
#define GRADIENT_COLORS 256
#define GRADIENT_FP 7991872763294034825ull
#define PATTERN_COLORS 256
#define PATTERN_FP 6255433330176470139ull

static void gradient(uint32_t *px)
{
    for (size_t i = 0; i < (size_t)W * H; i++) {
        size_t x = i % W, y = i / W;
        px[i] = HSM01_RGB(x * 255 / W, y * 255 / H, 128);
    }
}

static void pattern(uint32_t *px)
{
    for (size_t i = 0; i < (size_t)W * H; i++) {
        size_t x = i % W, y = i / W;
        px[i] = HSM01_RGB((x * 7 + y * 3) % 256, (x ^ y) % 256, (x * y / 64) % 256);
    }
}

static uint64_t fingerprint(const hsm01_indexed_image *img)
{
    uint64_t h = 0xcbf29ce484222325ull;
    for (size_t i = 0; i < img->colors; i++) {
        h = (h ^ (img->palette[i] & 0xffu)) * 0x100000001b3ull;
        h = (h ^ (img->palette[i] >> 8)) * 0x100000001b3ull;
    }
    for (size_t i = 0; i < (size_t)W * H; i++)
        h = (h ^ img->pixels[i]) * 0x100000001b3ull;
    return h;
}

static void test_tiles_cover_every_pixel_once(void)
{
    int seen[32] = {0};
    for (int t = 0; t < HSM01_BG_TILES; t++)
        seen[hsm01_bg_tiles[t][1] * 8 + hsm01_bg_tiles[t][0]]++;
    for (int i = 0; i < 32; i++)
        CHECK(seen[i] == 1);
}

static void test_upload(uint32_t *px, hsm01_indexed_image *img, uint8_t *fb)
{
    gradient(px);
    CHECK(hsm01_quantize(px, img) == HSM01_OK);
    CHECK(img->colors <= 256);
    hsm01_bg_render_progressive(img, 31, fb);
    CHECK(memcmp(fb, img->pixels, (size_t)W * H) == 0); /* the full upload rebuilds the picture */
    hsm01_bg_render_progressive(img, 0, fb);             /* first pass: 8x4 blocks */
    CHECK(fb[1] == img->pixels[0] && fb[3 * W + 7] == img->pixels[0]);
    /* the same results as the Rust port */
    CHECK(img->colors == GRADIENT_COLORS && fingerprint(img) == GRADIENT_FP);
    pattern(px);
    CHECK(hsm01_quantize(px, img) == HSM01_OK);
    CHECK(img->colors == PATTERN_COLORS && fingerprint(img) == PATTERN_FP);
    if (fingerprint(img) != PATTERN_FP)
        fprintf(stderr, "pattern: %zu colours, fingerprint %llu\n", img->colors,
                (unsigned long long)fingerprint(img));
}

static void test_few_colors_are_exact(uint32_t *px, hsm01_indexed_image *img)
{
    for (size_t i = 0; i < (size_t)W * H; i++)
        px[i] = i % 3 == 0 ? HSM01_RGB(255, 0, 0) : HSM01_RGB(0, 0, 255);
    CHECK(hsm01_quantize(px, img) == HSM01_OK);
    CHECK(img->colors == 2);
    CHECK((img->palette[0] == 0xf800 && img->palette[1] == 0x001f) ||
          (img->palette[0] == 0x001f && img->palette[1] == 0xf800));
}

static void test_commands(hsm01_indexed_image *img)
{
    hsm01_buf b;
    uint8_t tile[HSM01_BG_TILE_BYTES];
    hsm01_buf_init(&b);
    hsm01_cmd_palette(&b, 0, img->palette, img->colors);
    CHECK(b.len == 7 + 2 * img->colors && hsm01_command_len(b.data, b.len) == (int)b.len);
    CHECK(b.data[0] == 0x33 && b.data[4] == (img->colors & 0xff) && b.data[5] == (img->colors >> 8));
    hsm01_buf_clear(&b);
    hsm01_bg_tile(img, 5, tile);
    hsm01_cmd_background_tile(&b, 5, tile, sizeof(tile));
    CHECK(b.len == 9 + HSM01_BG_TILE_BYTES && hsm01_command_len(b.data, b.len) == (int)b.len);
    CHECK(b.data[0] == 0x37 && b.data[1] == 0x01 && b.data[2] == 5 && b.data[7] == 0xf0 && b.data[8] == 0x0f);
    CHECK(memcmp(b.data + 9, tile, sizeof(tile)) == 0);
    hsm01_buf_clear(&b);
    hsm01_cmd_background_tile(&b, 33, tile, 10); /* tile & 0x1f, short data padded */
    CHECK(b.len == 9 + HSM01_BG_TILE_BYTES && b.data[2] == 1 && b.data[9 + 10] == 0);
    hsm01_buf_clear(&b);
    hsm01_cmd_splash_flash(&b, true);
    hsm01_cmd_splash_flash(&b, false);
    CHECK(b.len == 4 && b.data[1] == 0x80 && b.data[3] == 0x40 && hsm01_command_len(b.data, b.len) == 2);
    hsm01_buf_clear(&b);
    uint16_t many[300] = {0};
    hsm01_cmd_palette(&b, 200, many, 300); /* 256 entries at most in total */
    CHECK(b.data[4] == 56 && b.len == 7 + 2 * 56);
    hsm01_buf_free(&b);
}

int main(void)
{
    uint32_t *px = malloc(sizeof(uint32_t) * W * H);
    hsm01_indexed_image *img = malloc(sizeof(*img));
    uint8_t *fb = malloc((size_t)W * H);
    if (!px || !img || !fb)
        return 2;
    test_tiles_cover_every_pixel_once();
    test_upload(px, img, fb);
    test_commands(img);
    test_few_colors_are_exact(px, img);
    free(px), free(img), free(fb);
    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return 1;
    }
    puts("test_background: ok");
    return 0;
}
