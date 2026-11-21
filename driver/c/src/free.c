/* "Free screen" protocol of the custom firmware - see include/hsm01/free.h. */
#include "hsm01/free.h"

#include <stdlib.h>

uint16_t hsm01_rgb565(uint8_t r, uint8_t g, uint8_t b)
{
    return (uint16_t)(((r & 0xf8u) << 8) | ((g & 0xfcu) << 3) | (b >> 3));
}

static void header(hsm01_buf *out, uint8_t op, size_t len)
{
    const uint8_t h[4] = {0xA5, op, 0, 0};
    hsm01_buf_put(out, h, sizeof(h));
    hsm01_buf_put_u32(out, (uint32_t)len);
}

static void rect(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h)
{
    hsm01_buf_put_u16(out, x);
    hsm01_buf_put_u16(out, y);
    hsm01_buf_put_u16(out, w);
    hsm01_buf_put_u16(out, h);
}

void hsm01_free_fill(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t color)
{
    header(out, HSM01_FREE_OP_FILL, 10);
    rect(out, x, y, w, h);
    hsm01_buf_put_u16(out, color);
}

void hsm01_free_blit(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px)
{
    size_t n = (size_t)w * h;
    header(out, HSM01_FREE_OP_BLIT, 8 + 2 * n);
    rect(out, x, y, w, h);
    for (size_t i = 0; i < n; i++)
        hsm01_buf_put_u16(out, px[i]);
}

/* Number of RLE runs ([count u8 >= 1][color u16], count <= 255). */
static size_t rle_runs(const uint16_t *px, size_t n)
{
    size_t runs = 0;
    for (size_t i = 0; i < n;) {
        size_t k = 1;
        while (k < 255 && i + k < n && px[i + k] == px[i])
            k++;
        runs++;
        i += k;
    }
    return runs;
}

void hsm01_free_blit_rle(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px)
{
    size_t n = (size_t)w * h;
    header(out, HSM01_FREE_OP_BLIT_RLE, 8 + 3 * rle_runs(px, n));
    rect(out, x, y, w, h);
    for (size_t i = 0; i < n;) {
        size_t k = 1;
        while (k < 255 && i + k < n && px[i + k] == px[i])
            k++;
        hsm01_buf_put_u8(out, (uint8_t)k);
        hsm01_buf_put_u16(out, px[i]);
        i += k;
    }
}

void hsm01_free_blit_auto(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px)
{
    size_t n = (size_t)w * h;
    if (3 * rle_runs(px, n) < 2 * n)
        hsm01_free_blit_rle(out, x, y, w, h, px);
    else
        hsm01_free_blit(out, x, y, w, h, px);
}

void hsm01_free_copy(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t to_x,
                     uint16_t to_y)
{
    header(out, HSM01_FREE_OP_COPY, 12);
    rect(out, x, y, w, h);
    hsm01_buf_put_u16(out, to_x);
    hsm01_buf_put_u16(out, to_y);
}

void hsm01_free_backlight(hsm01_buf *out, uint8_t percent)
{
    header(out, HSM01_FREE_OP_BACKLIGHT, 1);
    hsm01_buf_put_u8(out, percent > 100 ? 100 : percent);
}

void hsm01_free_leds(hsm01_buf *out, const uint8_t percent[4])
{
    header(out, HSM01_FREE_OP_LEDS, 4);
    for (int i = 0; i < 4; i++)
        hsm01_buf_put_u8(out, percent[i] > 100 ? 100 : percent[i]);
}

static uint16_t le16(const uint8_t *b) { return (uint16_t)(b[0] | (b[1] << 8)); }

bool hsm01_free_parse_report(const uint8_t *b, size_t n, hsm01_free_report *out)
{
    if (n < 21 || b[0] != 0x01)
        return false;
    out->seq = b[1];
    out->buttons = le16(b + 2);
    for (int i = 0; i < 8; i++)
        out->wipers[i] = le16(b + 4 + 2 * i);
    out->backlight = b[20];
    out->screen_epoch = n > 21 ? b[21] : 0;
    return true;
}

/* ---- knobs --------------------------------------------------------------------------------- */
#define KNOB_MID 2048
#define KNOB_CYCLE 65536
#define KNOB_STEP (KNOB_CYCLE / HSM01_KNOB_COUNTS_PER_CYCLE)

uint16_t hsm01_knob_phase(uint16_t a, uint16_t b)
{
    int32_t x = (int32_t)a - KNOB_MID, y = (int32_t)b - KNOB_MID;
    int32_t s = abs(x) + abs(y);
    if (s < 1)
        s = 1;
    const int32_t q = KNOB_CYCLE / 4;
    int32_t p;
    if (x >= 0 && y >= 0)
        p = y * q / s;
    else if (x < 0 && y >= 0)
        p = q + -x * q / s;
    else if (x < 0)
        p = 2 * q + -y * q / s;
    else
        p = 3 * q + x * q / s;
    return (uint16_t)(p & 0xffff); /* 65536 wraps to 0 */
}

void hsm01_knob_init(hsm01_knob_decoder *d)
{
    d->valid = false;
    d->reference = 0;
}

int hsm01_knob_update(hsm01_knob_decoder *d, uint16_t a, uint16_t b)
{
    uint16_t p = hsm01_knob_phase(a, b);
    if (!d->valid) {
        d->valid = true;
        d->reference = p;
        return 0;
    }
    int32_t diff = (int16_t)(uint16_t)(p - d->reference); /* shortest way round */
    int n = (int)(diff / KNOB_STEP);                     /* truncates toward zero, like Rust */
    d->reference = (uint16_t)(d->reference + n * KNOB_STEP);
    return n;
}
