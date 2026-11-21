/* Full-screen custom background - see include/hsm01/background.h; port of
 * driver/rust/hsm01-proto/src/background.rs, step by step (same box order, same stable sorts,
 * same ties) so both give the same palette and pixels. */
#include "hsm01/background.h"

#include <stdlib.h>
#include <string.h>

const uint8_t hsm01_bg_tiles[HSM01_BG_TILES][4] = {
    {0, 0, 8, 4}, {4, 0, 4, 4}, {0, 2, 4, 2}, {4, 2, 4, 2}, {2, 0, 2, 2}, {6, 0, 2, 2},
    {2, 2, 2, 2}, {6, 2, 2, 2}, {0, 1, 2, 1}, {2, 1, 2, 1}, {4, 1, 2, 1}, {6, 1, 2, 1},
    {0, 3, 2, 1}, {2, 3, 2, 1}, {4, 3, 2, 1}, {6, 3, 2, 1}, {1, 0, 1, 1}, {3, 0, 1, 1},
    {5, 0, 1, 1}, {7, 0, 1, 1}, {1, 2, 1, 1}, {3, 2, 1, 1}, {5, 2, 1, 1}, {7, 2, 1, 1},
    {1, 1, 1, 1}, {3, 1, 1, 1}, {5, 1, 1, 1}, {7, 1, 1, 1}, {1, 3, 1, 1}, {3, 3, 1, 1},
    {5, 3, 1, 1}, {7, 3, 1, 1},
};

#define NPIX (HSM01_SCREEN_W * HSM01_SCREEN_H)

static uint16_t rgb888_to_565(uint32_t p)
{
    return (uint16_t)(((p >> 8) & 0xf800u) | ((p >> 5) & 0x07e0u) | ((p >> 3) & 0x001fu));
}

static void expand565(uint16_t c, int e[3])
{
    int r = (c >> 11) & 0x1f, g = (c >> 5) & 0x3f, b = c & 0x1f;
    e[0] = (r << 3) | (r >> 2);
    e[1] = (g << 2) | (g >> 4);
    e[2] = (b << 3) | (b >> 2);
}

static int channel(uint16_t c, int ch)
{
    int e[3];
    expand565(c, e);
    return e[ch];
}

/* A colour of the histogram and its pixel count. */
typedef struct {
    uint16_t c;
    uint32_t n;
} entry;

/* A box: a range of the entries array (boxes never overlap; splitting keeps both halves in
 * place, so ranges stay contiguous). */
typedef struct {
    size_t start, len;
} box;

/* Stable sort of e[0..len) by channel `ch` (counting sort: keys 0..255, order kept on ties,
 * as Rust's sort_by_key). */
static void sort_box(entry *e, size_t len, int ch, entry *tmp)
{
    size_t count[257] = {0};
    for (size_t i = 0; i < len; i++)
        count[channel(e[i].c, ch) + 1]++;
    for (int k = 0; k < 256; k++)
        count[k + 1] += count[k];
    for (size_t i = 0; i < len; i++)
        tmp[count[channel(e[i].c, ch)]++] = e[i];
    memcpy(e, tmp, len * sizeof(*e));
}

int hsm01_quantize(const uint32_t *argb, hsm01_indexed_image *out)
{
    uint32_t *hist = calloc(1u << 16, sizeof(*hist));
    entry *entries = malloc((1u << 16) * sizeof(*entries));
    entry *tmp = malloc((1u << 16) * sizeof(*tmp));
    uint8_t *lut = calloc(1u << 16, 1);
    box boxes[256];
    size_t nboxes = 1, ncolors = 0;
    if (!hist || !entries || !tmp || !lut) {
        free(hist), free(entries), free(tmp), free(lut);
        return HSM01_ERR_NOMEM;
    }
    /* histogram in RGB565 space (the panel cannot show more anyway) */
    for (size_t i = 0; i < NPIX; i++)
        hist[rgb888_to_565(argb[i])]++;
    for (uint32_t c = 0; c < (1u << 16); c++)
        if (hist[c])
            entries[ncolors++] = (entry){(uint16_t)c, hist[c]};
    boxes[0] = (box){0, ncolors};

    while (nboxes < 256) {
        /* the box with the widest channel range (first found on ties) */
        size_t best_i = 0;
        int best_ch = -1, best_r = 0;
        for (size_t i = 0; i < nboxes; i++) {
            if (boxes[i].len < 2)
                continue;
            for (int ch = 0; ch < 3; ch++) {
                int lo = 255, hi = 0;
                for (size_t k = 0; k < boxes[i].len; k++) {
                    int v = channel(entries[boxes[i].start + k].c, ch);
                    lo = v < lo ? v : lo;
                    hi = v > hi ? v : hi;
                }
                if (best_ch < 0 || hi - lo > best_r) {
                    best_i = i, best_ch = ch, best_r = hi - lo;
                }
            }
        }
        if (best_ch < 0)
            break;
        box b = boxes[best_i];
        boxes[best_i] = boxes[--nboxes]; /* swap_remove */
        entry *e = entries + b.start;
        sort_box(e, b.len, best_ch, tmp);
        uint64_t total = 0, acc = 0;
        size_t cut = 1;
        for (size_t k = 0; k < b.len; k++)
            total += e[k].n;
        for (size_t k = 0; k < b.len; k++) {
            acc += e[k].n;
            if (acc * 2 >= total) {
                cut = k + 1;
                cut = cut < 1 ? 1 : cut > b.len - 1 ? b.len - 1 : cut;
                break;
            }
        }
        boxes[nboxes++] = (box){b.start, cut};
        boxes[nboxes++] = (box){b.start + cut, b.len - cut};
    }

    /* each box's average colour */
    int pal[256][3];
    for (size_t i = 0; i < nboxes; i++) {
        uint64_t s[3] = {0, 0, 0}, n = 0;
        for (size_t k = 0; k < boxes[i].len; k++) {
            const entry *x = &entries[boxes[i].start + k];
            int ex[3];
            expand565(x->c, ex);
            for (int j = 0; j < 3; j++)
                s[j] += (uint64_t)ex[j] * x->n;
            n += x->n;
        }
        if (!n)
            n = 1;
        uint32_t avg = (uint32_t)(s[0] / n) << 16 | (uint32_t)(s[1] / n) << 8 | (uint32_t)(s[2] / n);
        out->palette[i] = rgb888_to_565(avg);
        expand565(out->palette[i], pal[i]);
    }
    out->colors = nboxes;
    /* nearest palette entry (first one on ties) for every colour present */
    for (uint32_t c = 0; c < (1u << 16); c++) {
        if (!hist[c])
            continue;
        int e[3], best = -1;
        size_t idx = 0;
        expand565((uint16_t)c, e);
        for (size_t i = 0; i < nboxes; i++) {
            int d = 0;
            for (int j = 0; j < 3; j++)
                d += (pal[i][j] - e[j]) * (pal[i][j] - e[j]);
            if (best < 0 || d < best)
                best = d, idx = i;
        }
        lut[c] = (uint8_t)idx;
    }
    for (size_t i = 0; i < NPIX; i++)
        out->pixels[i] = lut[rgb888_to_565(argb[i])];
    free(hist), free(entries), free(tmp), free(lut);
    return HSM01_OK;
}

void hsm01_bg_tile(const hsm01_indexed_image *img, int t, uint8_t out[HSM01_BG_TILE_BYTES])
{
    const int dx = hsm01_bg_tiles[t][0], dy = hsm01_bg_tiles[t][1];
    size_t k = 0;
    for (int row = 0; row < HSM01_SCREEN_H / 4; row++)
        for (int col = 0; col < HSM01_SCREEN_W / 8; col++)
            out[k++] = img->pixels[(row * 4 + dy) * HSM01_SCREEN_W + col * 8 + dx];
}

void hsm01_bg_render_progressive(const hsm01_indexed_image *img, int last, uint8_t *fb)
{
    uint8_t data[HSM01_BG_TILE_BYTES];
    memset(fb, 0, NPIX);
    for (int t = 0; t <= last && t < HSM01_BG_TILES; t++) {
        const int dx = hsm01_bg_tiles[t][0], dy = hsm01_bg_tiles[t][1];
        const int bw = hsm01_bg_tiles[t][2], bh = hsm01_bg_tiles[t][3];
        hsm01_bg_tile(img, t, data);
        for (int row = 0; row < HSM01_SCREEN_H / 4; row++)
            for (int col = 0; col < HSM01_SCREEN_W / 8; col++) {
                uint8_t v = data[row * (HSM01_SCREEN_W / 8) + col];
                for (int y = 0; y < bh; y++)
                    for (int x = 0; x < bw; x++) {
                        int px = col * 8 + dx + x, py = row * 4 + dy + y;
                        if (px < HSM01_SCREEN_W && py < HSM01_SCREEN_H)
                            fb[py * HSM01_SCREEN_W + px] = v;
                    }
            }
    }
}
