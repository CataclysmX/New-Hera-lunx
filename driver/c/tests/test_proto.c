/* Unit tests for hsm01_proto, using the same captured vectors as the Rust crate. */
#include "hsm01/proto.h"

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

static size_t unhex(const char *s, uint8_t *out)
{
    size_t n = strlen(s) / 2;
    for (size_t i = 0; i < n; i++) {
        unsigned v;
        sscanf(s + 2 * i, "%2x", &v);
        out[i] = (uint8_t)v;
    }
    return n;
}

/* Captured from the official Windows app (frames 311, 2079, 2049). */
static const char *const VECTORS[] = {
    "534d2200b89a010010010a000000f465c80251010000000000000000000000001100",
    "534d1e007a7c24001431010064380004ffffffffffffffff04ffffffff00",
    "534d0a00ad1a1d001400",
};

static void test_frames(void)
{
    for (size_t v = 0; v < sizeof(VECTORS) / sizeof(*VECTORS); v++) {
        uint8_t m[256];
        size_t n = unhex(VECTORS[v], m);
        uint16_t seq;
        const uint8_t *cmds;
        size_t ncmds;
        CHECK(hsm01_parse_sm(m, n, &seq, &cmds, &ncmds) == HSM01_OK);
        hsm01_buf b;
        hsm01_buf_init(&b);
        CHECK(hsm01_build_sm(seq, cmds, ncmds, &b) == HSM01_OK);
        CHECK(b.len == n && memcmp(b.data, m, n) == 0);
        hsm01_buf_free(&b);
    }
    uint8_t m[16];
    size_t n = unhex(VECTORS[2], m);
    m[9] ^= 1;
    CHECK(hsm01_parse_sm(m, n, NULL, NULL, NULL) == HSM01_ERR_BAD_CRC);

    hsm01_buf f;
    hsm01_buf_init(&f);
    const uint8_t ack[2] = {0x14, 0x00};
    CHECK(hsm01_build_frame(0, ack, 2, &f) == HSM01_OK);
    CHECK(f.len == 2 * HSM01_ISO_PACKET);
    CHECK(memcmp(f.data, "HERCULES", 8) == 0 && f.data[HSM01_ISO_PACKET] == 'S');
    hsm01_buf_free(&f);
}

static void test_commands(void)
{
    hsm01_buf b;
    hsm01_buf_init(&b);
    const uint16_t os[4] = {10, 0, 26100, 712};
    hsm01_cmd_host_info(&b, os, 0x51, 1);
    hsm01_cmd_ping(&b, 0);
    uint8_t want[64];
    size_t n = unhex("10010a000000f465c80251010000000000000000000000001100", want);
    CHECK(b.len == n && memcmp(b.data, want, n) == 0);

    hsm01_buf_clear(&b);
    hsm01_cmd_ack_device_info(&b);
    hsm01_cmd_screen_brightness(&b, 100);
    hsm01_cmd_screen_config(&b, 0, 4, 0xffffffffu, 0xffffffffu, 4, 0xffffffffu);
    hsm01_cmd_end(&b);
    n = unhex("1431010064380004ffffffffffffffff04ffffffff00", want);
    CHECK(b.len == n && memcmp(b.data, want, n) == 0);

    /* every encoder agrees with the firmware length table */
    const uint16_t colors[3] = {0x2104, 0xf800, 0xffff}, grad[2] = {0x2945, 0x2945}, vals[4] = {1, 1, 2, 2};
    const uint8_t blocks[4][6] = {{0x81, 1, 7, 7, 0, 0x64}, {0x81, 1, 7, 7, 0, 0x64},
                                  {0x81, 1, 7, 7, 0, 0x64}, {0x81, 1, 7, 7, 0, 0x64}};
    uint8_t zeros[32] = {0};
    for (int k = 0; k < 6; k++) {
        hsm01_buf_clear(&b);
        switch (k) {
        case 0: hsm01_cmd_led(&b, 1, HSM01_LED_BLINK); break;
        case 1: hsm01_cmd_vu_style(&b, 2, 1, colors, grad, 2); break;
        case 2: hsm01_cmd_vu_markers(&b, 0, 0, vals, false); break;
        case 3: hsm01_cmd_vu_level(&b, 3, 0, 0xffff, 0xffff, false); break;
        case 4: hsm01_cmd_icon(&b, HSM01_BANK_A, 0, zeros, sizeof(zeros)); break;
        case 5: hsm01_cmd_channel_display(&b, blocks); break;
        }
        CHECK(hsm01_command_len(b.data, b.len) == (int)b.len);
    }
    hsm01_buf_free(&b);
}

static void test_rle(void)
{
    uint8_t a;
    uint16_t c;
    hsm01_argb_to_a4_565(HSM01_RGB(255, 255, 255), &a, &c);
    CHECK(a == 15 && c == 0xffff);
    hsm01_argb_to_a4_565(HSM01_RGB(255, 0, 0), &a, &c);
    CHECK(a == 15 && c == 0xf800);
    hsm01_argb_to_a4_565(HSM01_RGB(0, 255, 0), &a, &c);
    CHECK(a == 15 && c == 0x07e0);

    static uint32_t icon[32 * 32];
    hsm01_buf b;
    hsm01_buf_init(&b);
    CHECK(hsm01_rle_encode(icon, 32, 32, &b) == HSM01_OK);
    CHECK(b.len == 32); /* captured: fully transparent icon = 32 x 00 */
    for (size_t i = 0; i < b.len; i++)
        CHECK(b.data[i] == 0);

    static uint32_t img[110 * 16];
    for (size_t i = 0; i < 110 * 16; i++)
        img[i] = (i / 7) % 3 == 0 ? HSM01_RGB(255, 0, 0) : (i % 5 == 0 ? 0 : HSM01_RGB(0, 0, 255));
    hsm01_buf_clear(&b);
    CHECK(hsm01_rle_encode(img, 110, 16, &b) == HSM01_OK);
    static uint8_t da[110 * 16];
    static uint16_t dc[110 * 16];
    CHECK(hsm01_rle_decode(b.data, b.len, 110, 16, da, dc) == HSM01_OK);
    for (size_t i = 0; i < 110 * 16; i++) {
        hsm01_argb_to_a4_565(img[i], &a, &c);
        CHECK(da[i] == a && (a == 0 || dc[i] == c));
    }
    hsm01_buf_free(&b);
}

static void test_input(void)
{
    uint8_t r[64];
    size_t n = unhex("0c0000a4fff8ff0f002400bc736d2100e8d10200003c0109005a03026ec2a707079b00000300000000030e0cff6e0010"
                     "00040904000000000000000000000000",
                     r);
    hsm01_report rep;
    CHECK(hsm01_parse_report(r, n, &rep) == HSM01_OK);
    CHECK(rep.has_controls && rep.controls.buttons == 0);
    CHECK(rep.controls.encoders[0] == -92 && rep.controls.encoders[1] == -8);
    CHECK(rep.controls.encoders[2] == 15 && rep.controls.encoders[3] == 36);
    CHECK(rep.controls.screen_on && rep.controls.brightness == 60);
    CHECK(rep.kind == HSM01_MSG_STATUS);
    CHECK(rep.status.error == 0 && rep.status.error_count == 9);
    CHECK(rep.status.factory_code == 3 && rep.status.flash_status == 0xff);
    r[20] ^= 1; /* corrupt the sm message */
    CHECK(hsm01_parse_report(r, n, &rep) == HSM01_ERR_BAD_CHECKSUM);

    CHECK(hsm01_encoder_delta(32767, -32768) == 1);
    CHECK(hsm01_encoder_delta(215, 219) == 4);
}

int main(void)
{
    test_frames();
    test_commands();
    test_rle();
    test_input();
    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return EXIT_FAILURE;
    }
    puts("all hsm01_proto tests passed");
    return EXIT_SUCCESS;
}
