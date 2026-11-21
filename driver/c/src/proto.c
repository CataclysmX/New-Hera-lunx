/* Hercules Stream 100 wire protocol - see include/hsm01/proto.h and docs/protocol/. */
#include "hsm01/proto.h"

#include <stdlib.h>
#include <string.h>

const char *hsm01_strerror(int err)
{
    switch (err) {
    case HSM01_OK: return "ok";
    case HSM01_ERR_NOT_FOUND: return "Hercules Stream 100 (06f8:e053) not found";
    case HSM01_ERR_USB: return "usb error";
    case HSM01_ERR_ISO: return "isochronous transfer failed";
    case HSM01_ERR_TOO_LONG: return "SM message too long";
    case HSM01_ERR_BAD_HEADER: return "bad SM header";
    case HSM01_ERR_BAD_CRC: return "bad SM CRC";
    case HSM01_ERR_TRUNCATED: return "truncated report";
    case HSM01_ERR_BAD_SIGNATURE: return "bad 'sm' signature";
    case HSM01_ERR_BAD_CHECKSUM: return "bad 'sm' checksum";
    case HSM01_ERR_NOMEM: return "out of memory";
    case HSM01_ERR_BAD_REPLY: return "unexpected vendor-request reply";
    case HSM01_ERR_INVALID: return "invalid argument";
    case HSM01_ERR_WRONG_FW: return "wrong firmware (custom firmware reports version 240.xx)";
    default: return "unknown error";
    }
}

/* ---- buffer -------------------------------------------------------------------------------- */
void hsm01_buf_init(hsm01_buf *b) { memset(b, 0, sizeof(*b)); }

void hsm01_buf_free(hsm01_buf *b)
{
    free(b->data);
    hsm01_buf_init(b);
}

void hsm01_buf_clear(hsm01_buf *b)
{
    b->len = 0;
    b->oom = false;
}

void hsm01_buf_put(hsm01_buf *b, const void *data, size_t n)
{
    if (b->oom || n == 0)
        return;
    if (b->len + n > b->cap) {
        size_t cap = b->cap ? b->cap : 256;
        while (cap < b->len + n)
            cap *= 2;
        uint8_t *p = realloc(b->data, cap);
        if (!p) {
            b->oom = true;
            return;
        }
        b->data = p;
        b->cap = cap;
    }
    memcpy(b->data + b->len, data, n);
    b->len += n;
}

void hsm01_buf_put_u8(hsm01_buf *b, uint8_t v) { hsm01_buf_put(b, &v, 1); }

void hsm01_buf_put_u16(hsm01_buf *b, uint16_t v)
{
    uint8_t t[2] = {(uint8_t)v, (uint8_t)(v >> 8)};
    hsm01_buf_put(b, t, 2);
}

void hsm01_buf_put_u32(hsm01_buf *b, uint32_t v)
{
    uint8_t t[4] = {(uint8_t)v, (uint8_t)(v >> 8), (uint8_t)(v >> 16), (uint8_t)(v >> 24)};
    hsm01_buf_put(b, t, 4);
}

static uint16_t rd16(const uint8_t *p) { return (uint16_t)(p[0] | p[1] << 8); }

/* ---- CRC (DLL FUN_180016490 / FUN_180016570) ------------------------------------------------ */
static uint16_t crc_table[256];
static bool crc_ready;

static void crc_init(void)
{
    for (unsigned i = 0; i < 256; i++) {
        uint16_t neg = (uint16_t)(0u - (i & 1));
        uint16_t v = ((((neg & 5) ^ (i >> 1)) & 1) == 0) ? (uint16_t)(neg & 0x4002)
                                                         : (uint16_t)((neg & 0x4002) ^ 0x8005);
        for (unsigned k = 2; k < 8; k++) {
            uint16_t nv = v >> 1;
            if (((i >> k) ^ v) & 1)
                nv ^= 0x8005;
            v = nv;
        }
        crc_table[i] = v;
    }
    crc_ready = true;
}

uint16_t hsm01_crc16(const uint8_t *data, size_t n, uint16_t crc)
{
    if (!crc_ready)
        crc_init(); /* idempotent: concurrent first calls compute identical tables */
    for (size_t i = 0; i < n; i++)
        crc = (uint16_t)((crc >> 8) ^ crc_table[(crc ^ data[i]) & 0xff]);
    return crc;
}

uint16_t hsm01_sm_crc(const uint8_t *msg, size_t len)
{
    return hsm01_crc16(msg + 6, len - 6, hsm01_crc16(msg, 4, 0));
}

/* ---- framing -------------------------------------------------------------------------------- */
int hsm01_build_sm(uint16_t seq, const uint8_t *cmds, size_t n, hsm01_buf *out)
{
    size_t len = 8 + n, start = out->len;
    if (len > HSM01_MAX_SM_LEN)
        return HSM01_ERR_TOO_LONG;
    const uint8_t hdr[8] = {'S', 'M', (uint8_t)len, (uint8_t)(len >> 8), 0, 0, (uint8_t)seq, (uint8_t)(seq >> 8)};
    hsm01_buf_put(out, hdr, sizeof(hdr));
    hsm01_buf_put(out, cmds, n);
    if (out->oom)
        return HSM01_ERR_NOMEM;
    uint16_t crc = hsm01_sm_crc(out->data + start, len);
    out->data[start + 4] = (uint8_t)crc;
    out->data[start + 5] = (uint8_t)(crc >> 8);
    return HSM01_OK;
}

int hsm01_build_frame(uint16_t seq, const uint8_t *cmds, size_t n, hsm01_buf *out)
{
    static const uint8_t zeros[HSM01_ISO_PACKET];
    hsm01_buf_put(out, "HERCULES", 8);
    hsm01_buf_put(out, zeros, HSM01_ISO_PACKET - 8);
    int rc = hsm01_build_sm(seq, cmds, n, out);
    if (rc)
        return rc;
    size_t pad = (HSM01_ISO_PACKET - (8 + n) % HSM01_ISO_PACKET) % HSM01_ISO_PACKET;
    hsm01_buf_put(out, zeros, pad);
    return out->oom ? HSM01_ERR_NOMEM : HSM01_OK;
}

int hsm01_parse_sm(const uint8_t *msg, size_t n, uint16_t *seq, const uint8_t **cmds, size_t *ncmds)
{
    if (n < 8 || msg[0] != 'S' || msg[1] != 'M')
        return HSM01_ERR_BAD_HEADER;
    size_t len = rd16(msg + 2);
    if (len < 8 || len > n)
        return HSM01_ERR_BAD_HEADER;
    if (hsm01_sm_crc(msg, len) != rd16(msg + 4))
        return HSM01_ERR_BAD_CRC;
    if (seq)
        *seq = rd16(msg + 6);
    if (cmds)
        *cmds = msg + 8;
    if (ncmds)
        *ncmds = len - 8;
    return HSM01_OK;
}

/* ---- commands ------------------------------------------------------------------------------- */
static uint8_t clamp100(uint8_t v) { return v > 100 ? 100 : v; }

void hsm01_cmd_host_info(hsm01_buf *b, const uint16_t os[4], uint8_t sdk_minor_bcd, uint8_t sdk_major)
{
    static const uint8_t zeros[12];
    hsm01_buf_put_u8(b, 0x10);
    hsm01_buf_put_u8(b, 0x01);
    for (int i = 0; i < 4; i++)
        hsm01_buf_put_u16(b, os[i]);
    hsm01_buf_put_u8(b, sdk_minor_bcd);
    hsm01_buf_put_u8(b, sdk_major);
    hsm01_buf_put(b, zeros, sizeof(zeros));
}

void hsm01_cmd_ping(hsm01_buf *b, uint8_t v)
{
    const uint8_t c[2] = {0x11, v};
    hsm01_buf_put(b, c, 2);
}

void hsm01_cmd_keepalive(hsm01_buf *b)
{
    static const uint16_t os[4] = {6, 1, 0, 0};
    hsm01_cmd_host_info(b, os, 0x53, 0x01);
    hsm01_cmd_ping(b, 0);
}

static void put5(hsm01_buf *b, uint8_t op)
{
    const uint8_t c[5] = {op, 0, 0, 0, 0};
    hsm01_buf_put(b, c, 5);
}

void hsm01_cmd_screen_on(hsm01_buf *b) { put5(b, 0x12); }
void hsm01_cmd_screen_off(hsm01_buf *b) { put5(b, 0x13); }
void hsm01_cmd_ack_device_info(hsm01_buf *b) { hsm01_buf_put_u8(b, 0x14); }
void hsm01_cmd_reset_display(hsm01_buf *b) { hsm01_buf_put_u8(b, 0x17); }
void hsm01_cmd_end(hsm01_buf *b) { hsm01_buf_put_u8(b, 0x00); }

void hsm01_cmd_palette(hsm01_buf *b, uint8_t offset, const uint16_t *colors, size_t n)
{
    const uint8_t c[4] = {0x33, 0x00, 0x00, offset};
    if (n > 256u - offset)
        n = 256u - offset;
    hsm01_buf_put(b, c, 4);
    hsm01_buf_put_u16(b, (uint16_t)n);
    hsm01_buf_put_u8(b, 0x00);
    for (size_t i = 0; i < n; i++)
        hsm01_buf_put_u16(b, colors[i]);
}

void hsm01_cmd_background_tile(hsm01_buf *b, uint8_t tile, const uint8_t *data, size_t n)
{
    static const uint8_t zeros[64];
    const size_t want = 4080; /* HSM01_BG_TILE_BYTES */
    const uint8_t c[2] = {0x37, 0x01};
    hsm01_buf_put(b, c, 2);
    hsm01_buf_put_u32(b, tile & 0x1fu);
    hsm01_buf_put_u8(b, 0x00);
    hsm01_buf_put_u16(b, (uint16_t)want);
    if (n > want)
        n = want;
    hsm01_buf_put(b, data, n);
    for (size_t left = want - n; left > 0;) { /* padded with zeros */
        size_t k = left < sizeof(zeros) ? left : sizeof(zeros);
        hsm01_buf_put(b, zeros, k);
        left -= k;
    }
}

void hsm01_cmd_splash_flash(hsm01_buf *b, bool custom)
{
    const uint8_t c[2] = {0x50, custom ? 0x80 : 0x40};
    hsm01_buf_put(b, c, 2);
}

void hsm01_cmd_led(hsm01_buf *b, uint8_t index, hsm01_led_state state)
{
    const uint8_t c[3] = {0x30, index, (uint8_t)state};
    hsm01_buf_put(b, c, 3);
}

void hsm01_cmd_screen_brightness(hsm01_buf *b, uint8_t percent)
{
    const uint8_t c[4] = {0x31, 0x01, 0x00, clamp100(percent)};
    hsm01_buf_put(b, c, 4);
}

void hsm01_cmd_led_intensity(hsm01_buf *b, uint8_t percent)
{
    const uint8_t c[4] = {0x39, 0x01, 0x00, clamp100(percent)};
    hsm01_buf_put(b, c, 4);
}

void hsm01_cmd_channel_display(hsm01_buf *b, const uint8_t blocks[4][6])
{
    hsm01_buf_put_u8(b, 0x32);
    hsm01_buf_put_u8(b, 0x00);
    for (int i = 0; i < 4; i++)
        hsm01_buf_put(b, blocks[i], 6);
}

void hsm01_cmd_vu_style(hsm01_buf *b, uint8_t channel, uint8_t sub, const uint16_t colors[3],
                        const uint16_t *gradient, size_t n)
{
    if (n > 16)
        n = 16;
    hsm01_buf_put_u8(b, 0x34);
    hsm01_buf_put_u8(b, (uint8_t)((channel & 3) | (sub & 1) << 7));
    for (int i = 0; i < 3; i++)
        hsm01_buf_put_u16(b, colors[i]);
    hsm01_buf_put_u8(b, (uint8_t)n);
    for (size_t i = 0; i < n; i++)
        hsm01_buf_put_u16(b, gradient[i]);
}

static void image(hsm01_buf *b, uint8_t op, hsm01_bank bank, uint8_t channel, const uint8_t *rle, size_t n)
{
    const uint8_t c[5] = {op, (uint8_t)bank, (uint8_t)(channel & 3), (uint8_t)n, (uint8_t)(n >> 8)};
    hsm01_buf_put(b, c, 5);
    hsm01_buf_put(b, rle, n);
}

void hsm01_cmd_icon(hsm01_buf *b, hsm01_bank bank, uint8_t channel, const uint8_t *rle, size_t n)
{
    image(b, 0x35, bank, channel, rle, n);
}

void hsm01_cmd_label(hsm01_buf *b, hsm01_bank bank, uint8_t channel, const uint8_t *rle, size_t n)
{
    image(b, 0x36, bank, channel, rle, n);
}

void hsm01_cmd_screen_config(hsm01_buf *b, uint8_t screen_type, uint8_t background_type, uint32_t darkest_rgb,
                             uint32_t lightest_rgb, uint8_t action_zone_style, uint32_t action_zone_rgb)
{
    const uint8_t c[3] = {0x38, screen_type, background_type};
    hsm01_buf_put(b, c, 3);
    hsm01_buf_put_u32(b, darkest_rgb);
    hsm01_buf_put_u32(b, lightest_rgb);
    hsm01_buf_put_u8(b, action_zone_style < 6 ? action_zone_style : 0);
    hsm01_buf_put_u32(b, action_zone_rgb);
}

void hsm01_cmd_vu_markers(hsm01_buf *b, uint8_t channel, uint8_t sub, const uint16_t values[4], bool wide)
{
    hsm01_buf_put_u8(b, 0x40);
    hsm01_buf_put_u8(b, (uint8_t)((channel & 3) | (wide ? 0x40 : 0) | (sub & 1) << 7));
    for (int i = 0; i < 4; i++) {
        if (wide)
            hsm01_buf_put_u16(b, values[i]);
        else
            hsm01_buf_put_u8(b, (uint8_t)values[i]);
    }
}

void hsm01_cmd_vu_level(hsm01_buf *b, uint8_t channel, uint8_t sub, uint16_t level, uint16_t peak, bool clip)
{
    uint8_t c = clip ? 1 : 0;
    hsm01_buf_put_u8(b, 0x41);
    hsm01_buf_put_u8(b, (uint8_t)((channel & 3) | c << 5 | c << 6 | (sub & 1) << 7));
    hsm01_buf_put_u16(b, level);
    hsm01_buf_put_u16(b, peak);
}

void hsm01_cmd_vu_inactive(hsm01_buf *b, uint8_t channel, uint8_t sub)
{
    hsm01_cmd_vu_level(b, channel, sub, 0xc000, 0xc000, false);
}

int hsm01_command_len(const uint8_t *b, size_t n)
{
    size_t len;
    if (n < 1)
        return -1;
#define NEED(k) \
    do { \
        if (n < (k)) \
            return -1; \
    } while (0)
    switch (b[0]) {
    case 0x00: case 0x14: case 0x15: case 0x16: case 0x17: len = 1; break;
    case 0x10: len = 24; break;
    case 0x11: case 0x20: case 0x50: len = 2; break;
    case 0x12: case 0x13: len = 5; break;
    case 0x21: NEED(2); len = 4u + b[1]; break;
    case 0x22: case 0x30: len = 3; break;
    case 0x31: case 0x39: NEED(2); len = 3u + (b[1] & 1); break;
    case 0x32: NEED(2); len = b[1] == 0 ? 26 : 18; break;
    case 0x33: NEED(6); len = 7u + 2u * rd16(b + 4); break;
    case 0x34: NEED(9); len = 9u + 2u * b[8]; break;
    case 0x35: case 0x36: NEED(5); len = 5u + rd16(b + 3); break;
    case 0x37: len = 9 + 4080; break;
    case 0x38: len = 16; break;
    case 0x40: NEED(2); len = (b[1] & 0x40) ? 10 : 6; break;
    case 0x41: len = 6; break;
    default: return -1;
    }
#undef NEED
    return len <= n ? (int)len : -1;
}

/* ---- RLE (DLL FUN_180004900 / FUN_1800046a0) ------------------------------------------------ */
void hsm01_argb_to_a4_565(uint32_t argb, uint8_t *a4_out, uint16_t *rgb565)
{
    uint32_t r = (argb >> 3) & 0x1f0000u;
    uint32_t c = (((r | (argb & 0xfc00u)) >> 2) | (argb & 0xf8u)) >> 3;
    uint32_t a4, v;
    if ((uint32_t)((argb & 0xff000000u) + 0x80000000u) < 0x4000001u) {
        a4 = 7;
    } else {
        a4 = argb >> 28;
    }
    if (a4 & 8) {
        v = c;
        if (!(a4 & 4))
            v -= (c >> 2) & 0x39e7u;
        if (!(a4 & 2))
            v -= (c >> 3) & 0x18e3u;
        if (!(a4 & 1))
            v -= (c >> 4) & 0x861u;
    } else {
        v = (a4 & 4) ? (c >> 2) & 0x39e7u : 0;
        if (a4 & 2)
            v += (c >> 3) & 0x18e3u;
        if (a4 & 1)
            v += (c >> 4) & 0x861u;
    }
    *a4_out = (uint8_t)a4;
    *rgb565 = (uint16_t)v;
}

int hsm01_rle_encode(const uint32_t *px, size_t w, size_t h, hsm01_buf *out)
{
    if (!px || w == 0)
        return HSM01_ERR_INVALID;
    for (size_t y = 0; y < h; y++) {
        const uint32_t *row = px + y * w;
        size_t x = 0;
        while (x < w) {
            uint8_t a, a2;
            uint16_t c, c2;
            hsm01_argb_to_a4_565(row[x], &a, &c);
            size_t n = 1;
            while (x + n < w) {
                hsm01_argb_to_a4_565(row[x + n], &a2, &c2);
                if (a2 != a || c2 != c)
                    break;
                n++;
            }
            size_t field;
            if (x + n == w) {
                field = 0;
                x = w;
            } else {
                field = n < 15 ? n : 15;
                x += field;
            }
            hsm01_buf_put_u8(out, (uint8_t)(a | field << 4));
            if (a)
                hsm01_buf_put_u16(out, c);
        }
    }
    return out->oom ? HSM01_ERR_NOMEM : HSM01_OK;
}

int hsm01_rle_decode(const uint8_t *d, size_t n, size_t w, size_t h, uint8_t *a4, uint16_t *rgb565)
{
    size_t i = 0, o = 0;
    for (size_t y = 0; y < h; y++) {
        size_t x = 0;
        while (x < w) {
            if (i >= n)
                return HSM01_ERR_TRUNCATED;
            uint8_t b = d[i++], a = b & 0x0f;
            uint16_t c = 0;
            if (a) {
                if (i + 2 > n)
                    return HSM01_ERR_TRUNCATED;
                c = rd16(d + i);
                i += 2;
            }
            size_t run = b >> 4;
            if (run == 0 || run > w - x)
                run = w - x;
            for (size_t k = 0; k < run; k++, o++) {
                a4[o] = a;
                rgb565[o] = c;
            }
            x += run;
        }
    }
    return i == n ? HSM01_OK : HSM01_ERR_INVALID;
}

/* ---- input ---------------------------------------------------------------------------------- */
const char *const hsm01_button_names[9] = {"knob1",   "knob2",   "knob3",   "knob4", "button1",
                                           "button2", "button3", "button4", "power"};

int16_t hsm01_encoder_delta(int16_t old_value, int16_t new_value)
{
    return (int16_t)(uint16_t)((uint16_t)new_value - (uint16_t)old_value);
}

int hsm01_parse_message(const uint8_t *sm, size_t n, hsm01_report *out)
{
    if (n < 7)
        return HSM01_ERR_TRUNCATED;
    if (sm[0] != 's' || sm[1] != 'm')
        return HSM01_ERR_BAD_SIGNATURE;
    size_t len = sm[2];
    if (len < 7 || len > n)
        return HSM01_ERR_TRUNCATED;
    uint8_t sum = 0;
    for (size_t i = 0; i < len; i++)
        sum = (uint8_t)(sum + sm[i]);
    if (sum)
        return HSM01_ERR_BAD_CHECKSUM;
    if (sm[6] == 1 && len >= 40) {
        hsm01_device_info *i = &out->info;
        out->kind = HSM01_MSG_INFO;
        i->device_type = sm[7];
        i->fw_minor = sm[8];
        i->fw_major = sm[9];
        memcpy(i->serial, sm + 10, 9);
        i->screen_w = rd16(sm + 21);
        i->screen_h = rd16(sm + 23);
        i->icon_w = rd16(sm + 29);
        i->icon_h = rd16(sm + 31);
        i->label_w = rd16(sm + 33);
        i->label_h = rd16(sm + 35);
        i->button_count = sm[38];
        i->led_count = sm[39];
    } else if (sm[6] == 2 && len >= 33) {
        hsm01_status *s = &out->status;
        out->kind = HSM01_MSG_STATUS;
        s->error = rd16(sm + 7);
        s->last_seq = rd16(sm + 9);
        s->error_count = rd16(sm + 11);
        memcpy(s->serial, sm + 15, 9);
        s->factory_code = rd16(sm + 24);
        s->last_invalid_cmd = sm[28];
        s->render_ms = sm[29];
        s->flash_status = sm[32];
    } else {
        out->kind = HSM01_MSG_UNKNOWN;
        out->unknown_kind = sm[6];
    }
    return HSM01_OK;
}

int hsm01_parse_report(const uint8_t *buf, size_t n, hsm01_report *out)
{
    memset(out, 0, sizeof(*out));
    if (n < 1)
        return HSM01_ERR_TRUNCATED;
    size_t c = buf[0];
    if (c > n)
        return HSM01_ERR_TRUNCATED;
    if (c >= 12) {
        out->has_controls = true;
        out->controls.buttons = rd16(buf + 1);
        for (int k = 0; k < 4; k++)
            out->controls.encoders[k] = (int16_t)rd16(buf + 3 + 2 * k);
        out->controls.screen_on = (buf[11] & 0x80) != 0;
        out->controls.brightness = buf[11] & 0x7f;
    }
    if (n - c >= 2 && buf[c] == 's' && buf[c + 1] == 'm')
        return hsm01_parse_message(buf + c, n - c, out);
    return HSM01_OK;
}
