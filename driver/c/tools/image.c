/* Pictures for the CLI - see image.h. */
#include "image.h"

#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "hsm01/proto.h"

#ifdef HSM01_HAVE_PNG
#include <png.h>
#endif
#ifdef HSM01_HAVE_JPEG
#include <jpeglib.h>
#include <setjmp.h>
#endif
#ifdef HSM01_HAVE_GIF
#include <gif_lib.h>
#endif

#define PI 3.14159265358979323846

const char *hsm01_image_formats(void)
{
    return ""
#ifdef HSM01_HAVE_PNG
           "PNG, "
#endif
#ifdef HSM01_HAVE_JPEG
           "JPEG, "
#endif
#ifdef HSM01_HAVE_GIF
           "GIF, "
#endif
           "BMP, PPM";
}

static void fail(char *err, size_t n, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(err, n, fmt, ap);
    va_end(ap);
}

static uint8_t *alloc_rgba(int w, int h)
{
    if (w <= 0 || h <= 0 || w > 32768 || h > 32768)
        return NULL;
    return malloc((size_t)w * (size_t)h * 4);
}

/* ---- PPM (P6, maxval <= 255) ---------------------------------------------------------------- */
static int ppm_token(FILE *f, long *out)
{
    int c;
    do {
        c = fgetc(f);
        if (c == '#')
            while (c != '\n' && c != EOF)
                c = fgetc(f);
    } while (c == ' ' || c == '\t' || c == '\n' || c == '\r');
    if (c < '0' || c > '9')
        return -1;
    long v = 0;
    while (c >= '0' && c <= '9') {
        v = v * 10 + (c - '0');
        if (v > 1000000)
            return -1;
        c = fgetc(f);
    }
    *out = v; /* `c` (one whitespace) consumed */
    return 0;
}

static uint8_t *load_ppm(FILE *f, int *w, int *h, char *err, size_t n)
{
    long lw, lh, maxval;
    if (fgetc(f) != 'P' || fgetc(f) != '6' || ppm_token(f, &lw) || ppm_token(f, &lh) || ppm_token(f, &maxval) ||
        maxval < 1 || maxval > 255) {
        fail(err, n, "not a binary PPM (P6, maxval <= 255)");
        return NULL;
    }
    uint8_t *out = alloc_rgba((int)lw, (int)lh);
    if (!out) {
        fail(err, n, "PPM of %ldx%ld: too big or empty", lw, lh);
        return NULL;
    }
    size_t count = (size_t)lw * (size_t)lh;
    for (size_t i = 0; i < count; i++) {
        uint8_t px[3];
        if (fread(px, 1, 3, f) != 3) {
            free(out);
            fail(err, n, "PPM truncated");
            return NULL;
        }
        for (int c = 0; c < 3; c++)
            out[i * 4 + (size_t)c] = (uint8_t)(px[c] * 255 / maxval);
        out[i * 4 + 3] = 255;
    }
    *w = (int)lw, *h = (int)lh;
    return out;
}

/* ---- BMP (uncompressed 8 / 24 / 32 bits, 32 bits with bit fields) -------------------------- */
static uint32_t le32(const uint8_t *p) { return p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }
static uint16_t le16(const uint8_t *p) { return (uint16_t)(p[0] | p[1] << 8); }

static int shift_of(uint32_t mask)
{
    int s = 0;
    if (!mask)
        return 0;
    while (!(mask & 1)) {
        mask >>= 1;
        s++;
    }
    return s;
}

static uint8_t *load_bmp(FILE *f, int *w, int *h, char *err, size_t n)
{
    uint8_t hd[54 + 12];
    size_t got = fread(hd, 1, sizeof(hd), f);
    if (got < 54 || hd[0] != 'B' || hd[1] != 'M') {
        fail(err, n, "BMP header truncated");
        return NULL;
    }
    uint32_t offset = le32(hd + 10), dib = le32(hd + 14), compression = le32(hd + 30);
    int32_t bw = (int32_t)le32(hd + 18), bh = (int32_t)le32(hd + 22);
    uint16_t bpp = le16(hd + 28);
    uint32_t colors = le32(hd + 46);
    if (dib < 40 || bw <= 0 || bh == 0 || bh == INT32_MIN || !(bpp == 8 || bpp == 24 || bpp == 32) ||
        !(compression == 0 || (compression == 3 && bpp == 32 && got >= 66))) {
        fail(err, n, "BMP not supported (%u bits, compression %u): 8, 24 or 32 bits, uncompressed", bpp,
             compression);
        return NULL;
    }
    int top_down = bh < 0, height = top_down ? -bh : bh;
    uint32_t masks[3] = {0xff0000u, 0x00ff00u, 0x0000ffu};
    if (compression == 3)
        masks[0] = le32(hd + 54), masks[1] = le32(hd + 58), masks[2] = le32(hd + 62);
    uint8_t pal[256][4] = {{0}};
    if (bpp == 8) {
        size_t count = colors && colors <= 256 ? colors : 256;
        if (fseek(f, 14 + (long)dib, SEEK_SET) || fread(pal, 4, count, f) != count) {
            fail(err, n, "BMP palette truncated");
            return NULL;
        }
    }
    uint8_t *out = alloc_rgba(bw, height);
    size_t stride = ((size_t)bw * bpp / 8 + 3) & ~(size_t)3;
    uint8_t *row = malloc(stride);
    if (!out || !row || fseek(f, (long)offset, SEEK_SET)) {
        free(out), free(row);
        fail(err, n, "BMP of %dx%d: too big, or truncated", bw, height);
        return NULL;
    }
    for (int r = 0; r < height; r++) {
        if (fread(row, 1, stride, f) != stride) {
            free(out), free(row);
            fail(err, n, "BMP pixels truncated");
            return NULL;
        }
        uint8_t *dst = out + (size_t)(top_down ? r : height - 1 - r) * (size_t)bw * 4;
        for (int x = 0; x < bw; x++, dst += 4) {
            if (bpp == 8) {
                const uint8_t *c = pal[row[x]];
                dst[0] = c[2], dst[1] = c[1], dst[2] = c[0];
            } else if (bpp == 24) {
                const uint8_t *c = row + x * 3;
                dst[0] = c[2], dst[1] = c[1], dst[2] = c[0];
            } else {
                uint32_t v = le32(row + x * 4);
                for (int c = 0; c < 3; c++) {
                    uint32_t m = masks[c], part = (v & m) >> shift_of(m), top = m >> shift_of(m);
                    dst[c] = (uint8_t)(top ? part * 255 / top : 0);
                }
            }
            dst[3] = 255;
        }
    }
    free(row);
    *w = bw, *h = height;
    return out;
}

/* ---- PNG, JPEG, GIF (system libraries) ----------------------------------------------------- */
#ifdef HSM01_HAVE_PNG
static uint8_t *load_png(const char *path, int *w, int *h, char *err, size_t n)
{
    png_image img;
    memset(&img, 0, sizeof(img));
    img.version = PNG_IMAGE_VERSION;
    if (!png_image_begin_read_from_file(&img, path)) {
        fail(err, n, "PNG: %s", img.message);
        return NULL;
    }
    img.format = PNG_FORMAT_RGBA;
    uint8_t *out = alloc_rgba((int)img.width, (int)img.height);
    if (!out || !png_image_finish_read(&img, NULL, out, 0, NULL)) {
        fail(err, n, "PNG: %s", out ? img.message : "too big");
        free(out);
        png_image_free(&img);
        return NULL;
    }
    *w = (int)img.width, *h = (int)img.height;
    return out;
}
#endif

#ifdef HSM01_HAVE_JPEG
struct jpeg_fail {
    struct jpeg_error_mgr pub;
    jmp_buf jump;
    char msg[JMSG_LENGTH_MAX];
};

static void jpeg_exit(j_common_ptr c)
{
    struct jpeg_fail *e = (struct jpeg_fail *)(void *)c->err;
    (*c->err->format_message)(c, e->msg);
    longjmp(e->jump, 1);
}

static uint8_t *load_jpeg(FILE *f, int *w, int *h, char *err, size_t n)
{
    struct jpeg_decompress_struct cinfo;
    struct jpeg_fail jf;
    uint8_t *volatile out = NULL;
    uint8_t *volatile row = NULL;
    cinfo.err = jpeg_std_error(&jf.pub);
    jf.pub.error_exit = jpeg_exit;
    if (setjmp(jf.jump)) {
        fail(err, n, "JPEG: %s", jf.msg);
        jpeg_destroy_decompress(&cinfo);
        free(out), free(row);
        return NULL;
    }
    jpeg_create_decompress(&cinfo);
    jpeg_stdio_src(&cinfo, f);
    jpeg_read_header(&cinfo, TRUE);
    cinfo.out_color_space = JCS_RGB;
    jpeg_start_decompress(&cinfo);
    int jw = (int)cinfo.output_width, jh = (int)cinfo.output_height;
    out = alloc_rgba(jw, jh);
    row = malloc((size_t)jw * 3);
    if (!out || !row) {
        snprintf(jf.msg, sizeof(jf.msg), "%dx%d: too big", jw, jh);
        longjmp(jf.jump, 1);
    }
    while (cinfo.output_scanline < cinfo.output_height) {
        JSAMPROW rows[1] = {row};
        size_t y = cinfo.output_scanline;
        jpeg_read_scanlines(&cinfo, rows, 1);
        uint8_t *dst = out + y * (size_t)jw * 4;
        for (int x = 0; x < jw; x++) {
            dst[x * 4] = row[x * 3], dst[x * 4 + 1] = row[x * 3 + 1], dst[x * 4 + 2] = row[x * 3 + 2];
            dst[x * 4 + 3] = 255;
        }
    }
    jpeg_finish_decompress(&cinfo);
    jpeg_destroy_decompress(&cinfo);
    free(row);
    *w = jw, *h = jh;
    return out;
}
#endif

#ifdef HSM01_HAVE_GIF
/* The first frame on the GIF's canvas (background colour, transparency as background). */
static uint8_t *load_gif(const char *path, int *w, int *h, char *err, size_t n)
{
    int e = 0;
    GifFileType *g = DGifOpenFileName(path, &e);
    if (!g) {
        fail(err, n, "GIF: %s", GifErrorString(e));
        return NULL;
    }
    if (DGifSlurp(g) != GIF_OK || g->ImageCount < 1) {
        fail(err, n, "GIF: %s", GifErrorString(g->Error));
        DGifCloseFile(g, &e);
        return NULL;
    }
    const SavedImage *si = &g->SavedImages[0];
    const ColorMapObject *cm = si->ImageDesc.ColorMap ? si->ImageDesc.ColorMap : g->SColorMap;
    GraphicsControlBlock gcb;
    int transparent = -1;
    if (DGifSavedExtensionToGCB(g, 0, &gcb) == GIF_OK)
        transparent = gcb.TransparentColor;
    int gw = g->SWidth, gh = g->SHeight;
    uint8_t *out = alloc_rgba(gw, gh);
    if (!out || !cm) {
        fail(err, n, out ? "GIF without colour map" : "GIF too big");
        free(out);
        DGifCloseFile(g, &e);
        return NULL;
    }
    GifColorType bg = {0, 0, 0};
    if (g->SColorMap && g->SBackGroundColor < g->SColorMap->ColorCount)
        bg = g->SColorMap->Colors[g->SBackGroundColor];
    for (size_t i = 0; i < (size_t)gw * (size_t)gh; i++)
        out[i * 4] = bg.Red, out[i * 4 + 1] = bg.Green, out[i * 4 + 2] = bg.Blue, out[i * 4 + 3] = 255;
    const GifImageDesc *d = &si->ImageDesc;
    for (int y = 0; y < d->Height; y++)
        for (int x = 0; x < d->Width; x++) {
            int px = d->Left + x, py = d->Top + y, c = si->RasterBits[y * d->Width + x];
            if (px >= gw || py >= gh || c == transparent || c >= cm->ColorCount)
                continue;
            uint8_t *dst = out + ((size_t)py * (size_t)gw + (size_t)px) * 4;
            dst[0] = cm->Colors[c].Red, dst[1] = cm->Colors[c].Green, dst[2] = cm->Colors[c].Blue;
        }
    DGifCloseFile(g, &e);
    *w = gw, *h = gh;
    return out;
}
#endif

uint8_t *hsm01_image_decode(const char *path, int *w, int *h, char *err, size_t errlen)
{
    uint8_t magic[8] = {0};
    FILE *f = fopen(path, "rb");
    if (!f) {
        fail(err, errlen, "%s: cannot open", path);
        return NULL;
    }
    size_t got = fread(magic, 1, sizeof(magic), f);
    rewind(f);
    uint8_t *out = NULL;
    char why[200] = "";
    if (got >= 2 && magic[0] == 'P' && magic[1] == '6') {
        out = load_ppm(f, w, h, why, sizeof(why));
    } else if (got >= 2 && magic[0] == 'B' && magic[1] == 'M') {
        out = load_bmp(f, w, h, why, sizeof(why));
    } else if (got >= 8 && !memcmp(magic, "\x89PNG\r\n\x1a\n", 8)) {
#ifdef HSM01_HAVE_PNG
        out = load_png(path, w, h, why, sizeof(why));
#else
        snprintf(why, sizeof(why), "PNG not supported by this build (libpng)");
#endif
    } else if (got >= 3 && magic[0] == 0xff && magic[1] == 0xd8 && magic[2] == 0xff) {
#ifdef HSM01_HAVE_JPEG
        out = load_jpeg(f, w, h, why, sizeof(why));
#else
        snprintf(why, sizeof(why), "JPEG not supported by this build (libjpeg)");
#endif
    } else if (got >= 4 && !memcmp(magic, "GIF8", 4)) {
#ifdef HSM01_HAVE_GIF
        out = load_gif(path, w, h, why, sizeof(why));
#else
        snprintf(why, sizeof(why), "GIF not supported by this build (giflib)");
#endif
    } else {
        snprintf(why, sizeof(why), "unknown format (this build reads %s)", hsm01_image_formats());
    }
    fclose(f);
    if (!out)
        fail(err, errlen, "%s: %s", path, why);
    return out;
}

/* ---- scaling (Lanczos 3, as the Rust image crate's resize: kernel widened when shrinking) ---- */
static double sinc(double x)
{
    if (x == 0.0)
        return 1.0;
    x *= PI;
    return sin(x) / x;
}

static double lanczos3(double x) { return fabs(x) < 3.0 ? sinc(x) * sinc(x / 3.0) : 0.0; }

/* Weights of output samples first..first+count of a src -> dst resampling: for output i,
 * taps left[i]..left[i]+taps[i] with weights w[i * max + k]. */
typedef struct {
    int *left, *taps, max;
    double *w;
} kernel;

static int make_kernel(kernel *k, int src, int dst, int first, int count)
{
    double ratio = (double)src / dst, sratio = ratio < 1.0 ? 1.0 : ratio, support = 3.0 * sratio;
    k->max = (int)ceil(support) * 2 + 2;
    k->left = malloc(sizeof(int) * (size_t)count);
    k->taps = malloc(sizeof(int) * (size_t)count);
    k->w = malloc(sizeof(double) * (size_t)count * (size_t)k->max);
    if (!k->left || !k->taps || !k->w)
        return -1;
    for (int i = 0; i < count; i++) {
        double center = (first + i + 0.5) * ratio;
        int left = (int)floor(center - support), right = (int)ceil(center + support);
        left = left < 0 ? 0 : left > src - 1 ? src - 1 : left;
        right = right > src ? src : right < left + 1 ? left + 1 : right;
        if (right - left > k->max)
            right = left + k->max;
        double sum = 0.0;
        for (int t = left; t < right; t++)
            sum += k->w[i * k->max + (t - left)] = lanczos3((t - (center - 0.5)) / sratio);
        for (int t = left; t < right; t++)
            k->w[i * k->max + (t - left)] /= sum != 0.0 ? sum : 1.0;
        k->left[i] = left, k->taps[i] = right - left;
    }
    return 0;
}

static void free_kernel(kernel *k) { free(k->left), free(k->taps), free(k->w); }

int hsm01_image_fill(const uint8_t *rgba, int w, int h, int ow, int oh, uint32_t *argb)
{
    /* the size that fills ow x oh (rounded as image::resize_to_fill), then the centred crop */
    double ratio = fmax((double)ow / w, (double)oh / h);
    int nw = (int)lround(w * ratio), nh = (int)lround(h * ratio);
    nw = nw < ow ? ow : nw, nh = nh < oh ? oh : nh;
    int x0, y0;
    if ((long long)ow * nh > (long long)nw * oh)
        x0 = 0, y0 = (nh - oh) / 2;
    else
        x0 = (nw - ow) / 2, y0 = 0;
    kernel kx = {0}, ky = {0};
    double *mid = malloc(sizeof(double) * 3 * (size_t)ow * (size_t)h);
    int rc = 0;
    if (!mid || make_kernel(&kx, w, nw, x0, ow) || make_kernel(&ky, h, nh, y0, oh)) {
        rc = -1;
        goto done;
    }
    for (int y = 0; y < h; y++) /* horizontal: the ow columns kept */
        for (int x = 0; x < ow; x++) {
            double s[3] = {0, 0, 0};
            const double *wt = kx.w + (size_t)x * (size_t)kx.max;
            for (int t = 0; t < kx.taps[x]; t++) {
                const uint8_t *p = rgba + ((size_t)y * (size_t)w + (size_t)(kx.left[x] + t)) * 4;
                for (int c = 0; c < 3; c++)
                    s[c] += wt[t] * p[c];
            }
            memcpy(mid + ((size_t)y * (size_t)ow + (size_t)x) * 3, s, sizeof(s));
        }
    for (int y = 0; y < oh; y++) /* vertical: the oh rows kept */
        for (int x = 0; x < ow; x++) {
            double s[3] = {0, 0, 0};
            const double *wt = ky.w + (size_t)y * (size_t)ky.max;
            for (int t = 0; t < ky.taps[y]; t++) {
                const double *p = mid + ((size_t)(ky.left[y] + t) * (size_t)ow + (size_t)x) * 3;
                for (int c = 0; c < 3; c++)
                    s[c] += wt[t] * p[c];
            }
            uint32_t v[3];
            for (int c = 0; c < 3; c++)
                v[c] = (uint32_t)(s[c] < 0.0 ? 0.0 : s[c] > 255.0 ? 255.0 : floor(s[c] + 0.5));
            argb[(size_t)y * (size_t)ow + (size_t)x] = 0xff000000u | v[0] << 16 | v[1] << 8 | v[2];
        }
done:
    free(mid);
    free_kernel(&kx), free_kernel(&ky);
    return rc;
}

int hsm01_load_image(const char *path, uint32_t *argb, char *err, size_t errlen)
{
    int w, h;
    uint8_t *rgba = hsm01_image_decode(path, &w, &h, err, errlen);
    if (!rgba)
        return -1;
    int rc = hsm01_image_fill(rgba, w, h, HSM01_SCREEN_W, HSM01_SCREEN_H, argb);
    free(rgba);
    if (rc)
        fail(err, errlen, "%s: out of memory", path);
    return rc;
}
