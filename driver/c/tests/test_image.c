/* Unit tests for the CLI's pictures (tools/image.c): every format written here then read back
 * (BMP 8 / 24 / 32 bits, top-down and bottom-up; PPM; PNG, JPEG, GIF when the libraries are
 * there), the scaling that fills the screen (proportions kept, centred crop), errors in words.
 * Files go to the current directory (CTest: the build directory). */
#include "image.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef HSM01_HAVE_PNG
#include <png.h>
#endif
#ifdef HSM01_HAVE_JPEG
#include <jpeglib.h>
#endif
#ifdef HSM01_HAVE_GIF
#include <gif_lib.h>
#endif

static int failures;
#define CHECK(cond) \
    do { \
        if (!(cond)) { \
            fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++; \
        } \
    } while (0)

/* A 4 x 2 picture: red, green, blue, white / black, grey, yellow, cyan. */
static const uint8_t PIX[8][3] = {{255, 0, 0}, {0, 255, 0}, {0, 0, 255}, {255, 255, 255},
                                  {0, 0, 0}, {128, 128, 128}, {255, 255, 0}, {0, 255, 255}};

static void put16(FILE *f, unsigned v) { fputc((int)(v & 0xff), f), fputc((int)(v >> 8 & 0xff), f); }
static void put32(FILE *f, unsigned long v) { put16(f, (unsigned)(v & 0xffff)), put16(f, (unsigned)(v >> 16)); }

static void write_bmp(const char *path, int bpp, int top_down)
{
    FILE *f = fopen(path, "wb");
    int stride = ((4 * bpp / 8) + 3) & ~3, palette = bpp == 8 ? 256 * 4 : 0;
    unsigned long offset = 54ul + (unsigned long)palette + (bpp == 32 ? 12 : 0);
    fputs("BM", f);
    put32(f, offset + (unsigned long)stride * 2), put32(f, 0), put32(f, offset);
    put32(f, 40), put32(f, 4), put32(f, top_down ? 0xfffffffeul : 2); /* height -2: top-down */
    put16(f, 1), put16(f, (unsigned)bpp), put32(f, bpp == 32 ? 3 : 0);
    put32(f, 0), put32(f, 2835), put32(f, 2835), put32(f, bpp == 8 ? 8 : 0), put32(f, 0);
    if (bpp == 32) /* bit fields: B G R in the low bytes, alpha on top */
        put32(f, 0x00ff0000ul), put32(f, 0x0000ff00ul), put32(f, 0x000000fful);
    if (bpp == 8)
        for (int i = 0; i < 256; i++) {
            const uint8_t *c = PIX[i % 8];
            fputc(c[2], f), fputc(c[1], f), fputc(c[0], f), fputc(0, f);
        }
    for (int r = 0; r < 2; r++) {
        int y = top_down ? r : 1 - r, written = 0;
        for (int x = 0; x < 4; x++) {
            const uint8_t *c = PIX[y * 4 + x];
            if (bpp == 8)
                fputc(y * 4 + x, f), written += 1;
            else {
                fputc(c[2], f), fputc(c[1], f), fputc(c[0], f), written += 3;
                if (bpp == 32)
                    fputc(0xff, f), written++;
            }
        }
        for (; written < stride; written++)
            fputc(0, f);
    }
    fclose(f);
}

static void check_pixels(const char *path, int exact)
{
    int w = 0, h = 0;
    char err[256] = "";
    uint8_t *px = hsm01_image_decode(path, &w, &h, err, sizeof(err));
    CHECK(px != NULL);
    if (!px) {
        fprintf(stderr, "%s\n", err);
        return;
    }
    CHECK(w == 4 && h == 2);
    for (int i = 0; i < 8 && w == 4 && h == 2; i++)
        for (int c = 0; c < 3; c++) {
            int d = px[i * 4 + c] - PIX[i][c];
            if (exact ? d != 0 : (d < -40 || d > 40)) {
                fprintf(stderr, "%s pixel %d channel %d: %d instead of %d\n", path, i, c, px[i * 4 + c], PIX[i][c]);
                failures++;
            }
        }
    free(px);
}

static void test_formats(void)
{
    write_bmp("test_image24.bmp", 24, 0);
    check_pixels("test_image24.bmp", 1);
    write_bmp("test_image32.bmp", 32, 1);
    check_pixels("test_image32.bmp", 1);
    write_bmp("test_image8.bmp", 8, 0);
    check_pixels("test_image8.bmp", 1);
    FILE *f = fopen("test_image.ppm", "wb");
    fputs("P6\n# comment\n4 2\n255\n", f);
    fwrite(PIX, 3, 8, f);
    fclose(f);
    check_pixels("test_image.ppm", 1);
#ifdef HSM01_HAVE_PNG
    {
        uint8_t rgba[8 * 4];
        for (int i = 0; i < 8; i++)
            memcpy(rgba + i * 4, PIX[i], 3), rgba[i * 4 + 3] = 255;
        png_image img;
        memset(&img, 0, sizeof(img));
        img.version = PNG_IMAGE_VERSION, img.width = 4, img.height = 2, img.format = PNG_FORMAT_RGBA;
        CHECK(png_image_write_to_file(&img, "test_image.png", 0, rgba, 0, NULL));
        check_pixels("test_image.png", 1);
    }
#endif
#ifdef HSM01_HAVE_JPEG
    {
        struct jpeg_compress_struct c;
        struct jpeg_error_mgr e;
        FILE *jf = fopen("test_image.jpg", "wb");
        c.err = jpeg_std_error(&e);
        jpeg_create_compress(&c);
        jpeg_stdio_dest(&c, jf);
        c.image_width = 4, c.image_height = 2, c.input_components = 3, c.in_color_space = JCS_RGB;
        jpeg_set_defaults(&c);
        jpeg_set_quality(&c, 100, TRUE);
        c.comp_info[0].h_samp_factor = 1, c.comp_info[0].v_samp_factor = 1; /* no chroma subsampling */
        jpeg_start_compress(&c, TRUE);
        for (int y = 0; y < 2; y++) {
            JSAMPROW row = (JSAMPROW)(void *)PIX[y * 4];
            jpeg_write_scanlines(&c, &row, 1);
        }
        jpeg_finish_compress(&c);
        jpeg_destroy_compress(&c);
        fclose(jf);
        check_pixels("test_image.jpg", 0); /* lossy: close */
    }
#endif
#ifdef HSM01_HAVE_GIF
    {
        int e = 0;
        GifColorType colors[8];
        for (int i = 0; i < 8; i++)
            colors[i].Red = PIX[i][0], colors[i].Green = PIX[i][1], colors[i].Blue = PIX[i][2];
        ColorMapObject *cm = GifMakeMapObject(8, colors);
        GifFileType *g = EGifOpenFileName("test_image.gif", false, &e);
        CHECK(g && cm);
        if (g && cm) {
            EGifPutScreenDesc(g, 4, 2, 8, 0, cm);
            EGifPutImageDesc(g, 0, 0, 4, 2, false, NULL);
            for (int y = 0; y < 2; y++) {
                GifPixelType line[4] = {(GifPixelType)(y * 4), (GifPixelType)(y * 4 + 1), (GifPixelType)(y * 4 + 2),
                                        (GifPixelType)(y * 4 + 3)};
                EGifPutLine(g, line, 4);
            }
            EGifCloseFile(g, &e);
            check_pixels("test_image.gif", 1);
        }
        GifFreeMapObject(cm);
    }
#endif
    int w, h;
    char err[256] = "";
    f = fopen("test_image.txt", "wb");
    fputs("not a picture", f);
    fclose(f);
    CHECK(hsm01_image_decode("test_image.txt", &w, &h, err, sizeof(err)) == NULL && strstr(err, "unknown format"));
    CHECK(hsm01_image_decode("test_image_absent.png", &w, &h, err, sizeof(err)) == NULL && strstr(err, "cannot open"));
    printf("formats: %s\n", hsm01_image_formats());
}

static void test_fill(void)
{
    /* a 960 x 544 picture: left half red, right half blue -> halves kept, scaled by 1/2 */
    int w = 960, h = 544;
    uint8_t *src = malloc((size_t)w * h * 4);
    uint32_t *out = malloc(sizeof(uint32_t) * 480 * 272);
    for (int i = 0; i < w * h; i++) {
        int right = i % w >= w / 2;
        src[i * 4] = right ? 0 : 255, src[i * 4 + 1] = 0, src[i * 4 + 2] = right ? 255 : 0, src[i * 4 + 3] = 255;
    }
    CHECK(hsm01_image_fill(src, w, h, 480, 272, out) == 0);
    CHECK(out[0] == 0xffff0000u && out[100 * 480 + 200] == 0xffff0000u);
    CHECK(out[479] == 0xff0000ffu && out[271 * 480 + 300] == 0xff0000ffu);
    /* taller (480 x 544): the width fills, the centre rows are kept (rows 136..408) */
    w = 480;
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) {
            uint8_t *p = src + ((size_t)y * w + x) * 4;
            p[0] = (uint8_t)(y < 136 || y >= 408 ? 255 : 0), p[1] = (uint8_t)(y < 136 || y >= 408 ? 0 : 200), p[2] = 0;
        }
    CHECK(hsm01_image_fill(src, w, h, 480, 272, out) == 0);
    CHECK(out[5 * 480 + 10] == 0xff00c800u && out[266 * 480 + 400] == 0xff00c800u);
    /* smaller (48 x 27, one colour): scaled up, still that colour */
    for (int i = 0; i < 48 * 27; i++)
        src[i * 4] = 10, src[i * 4 + 1] = 20, src[i * 4 + 2] = 30;
    CHECK(hsm01_image_fill(src, 48, 27, 480, 272, out) == 0);
    CHECK(out[0] == 0xff0a141eu && out[271 * 480 + 479] == 0xff0a141eu && out[136 * 480 + 240] == 0xff0a141eu);
    free(src), free(out);
}

int main(void)
{
    test_formats();
    test_fill();
    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return 1;
    }
    puts("test_image: ok");
    return 0;
}
