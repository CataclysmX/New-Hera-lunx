/* Unit tests for the custom-firmware protocol (free.h), mirroring the Rust `hsm01_proto::free` tests. */
#include "hsm01/free.h"

#include <math.h>
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

static void test_message_layout(void)
{
    static const uint8_t want[] = {0xA5, 0x01, 0, 0, 10, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 0x00, 0xf8};
    hsm01_buf b;
    hsm01_buf_init(&b);
    hsm01_free_fill(&b, 1, 2, 3, 4, 0xf800);
    CHECK(b.len == sizeof(want) && !memcmp(b.data, want, sizeof(want)));

    uint8_t leds[4] = {100, 150, 0, 7}; /* values are clamped to 100 */
    hsm01_buf_clear(&b);
    hsm01_free_leds(&b, leds);
    static const uint8_t want_leds[] = {0xA5, 0x11, 0, 0, 4, 0, 0, 0, 100, 100, 0, 7};
    CHECK(b.len == sizeof(want_leds) && !memcmp(b.data, want_leds, sizeof(want_leds)));
    CHECK(hsm01_rgb565(255, 0, 0) == 0xf800 && hsm01_rgb565(0, 255, 0) == 0x07e0 && hsm01_rgb565(0, 0, 255) == 0x001f);
    hsm01_buf_free(&b);
}

static void test_rle(void)
{
    uint16_t px[600];
    for (int i = 0; i < 600; i++)
        px[i] = 7;
    hsm01_buf b;
    hsm01_buf_init(&b);
    hsm01_free_blit_rle(&b, 0, 0, 600, 1, px);
    const uint8_t *p = b.data + 16;
    size_t payload = b.len - 16, total = 0;
    uint32_t len = (uint32_t)(b.data[4] | b.data[5] << 8 | b.data[6] << 16 | (uint32_t)b.data[7] << 24);
    CHECK(len == b.len - 8);
    CHECK(payload % 3 == 0);
    for (size_t i = 0; i < payload; i += 3) {
        CHECK(p[i] >= 1);
        total += p[i];
    }
    CHECK(total == 600);

    /* auto picks raw for noise, RLE for flat */
    hsm01_buf_clear(&b);
    hsm01_free_blit_auto(&b, 0, 0, 600, 1, px);
    CHECK(b.data[1] == HSM01_FREE_OP_BLIT_RLE);
    for (int i = 0; i < 600; i++)
        px[i] = (uint16_t)(i * 37);
    hsm01_buf_clear(&b);
    hsm01_free_blit_auto(&b, 0, 0, 600, 1, px);
    CHECK(b.data[1] == HSM01_FREE_OP_BLIT && b.len == 16 + 1200);

    /* copy (240.21): rect then destination, 12 bytes of payload */
    static const uint8_t copy[] = {0xA5, 0x04, 0, 0, 12, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0};
    hsm01_buf_clear(&b);
    hsm01_free_copy(&b, 1, 2, 3, 4, 5, 6);
    CHECK(b.len == sizeof(copy) && memcmp(b.data, copy, sizeof(copy)) == 0);
    hsm01_buf_free(&b);
}

static void test_report(void)
{
    uint8_t r[24] = {0};
    r[0] = 1;
    r[1] = 9;
    r[2] = 0x01;
    r[3] = 0x01;
    r[4] = 0xff;
    r[5] = 0x0f;
    r[20] = 80;
    r[21] = 3;
    hsm01_free_report rep;
    CHECK(hsm01_free_parse_report(r, sizeof(r), &rep));
    CHECK(rep.seq == 9 && rep.buttons == 0x0101 && rep.wipers[0] == 0x0fff && rep.backlight == 80);
    CHECK(rep.screen_epoch == 3);
    CHECK(hsm01_free_parse_report(r, 21, &rep) && rep.screen_epoch == 0); /* before 240.22 */
    r[0] = 2;
    CHECK(!hsm01_free_parse_report(r, sizeof(r), &rep));
    CHECK(!hsm01_free_parse_report(r, 20, &rep));
}

/* Quadrature triangle wipers at t cycles (a leads). */
static uint16_t tri(double u)
{
    u -= floor(u);
    double v = u < 0.5 ? 1.0 - 4.0 * u : 4.0 * u - 3.0;
    return (uint16_t)lround(2048.0 + v * 2047.0);
}

static void test_knob(void)
{
    for (int dir = -1; dir <= 1; dir += 2) {
        hsm01_knob_decoder d;
        hsm01_knob_init(&d);
        int total = 0;
        for (int i = 0; i <= 1000; i++) {
            double t = 0.1 + dir * i / 1000.0;
            total += hsm01_knob_update(&d, tri(t), tri(t - 0.25));
        }
        CHECK(abs(total - dir * HSM01_KNOB_COUNTS_PER_CYCLE) <= 1);
    }
    hsm01_knob_decoder d;
    hsm01_knob_init(&d);
    uint16_t a = tri(0.3), b = tri(0.05);
    int moved = 0;
    static const int noise[5] = {0, 7, -7, 3, -5};
    for (int i = 0; i < 1000; i++)
        moved += abs(hsm01_knob_update(&d, (uint16_t)(a + noise[i % 5]), (uint16_t)(b - noise[i % 5])));
    CHECK(moved == 0);
    /* same phase as the Rust implementation at the quadrant boundaries */
    CHECK(hsm01_knob_phase(4095, 2048) == 0 && hsm01_knob_phase(2048, 4095) == 16384);
    CHECK(hsm01_knob_phase(0, 2048) == 32768 && hsm01_knob_phase(2048, 0) == 49152);
}

int main(void)
{
    test_message_layout();
    test_rle();
    test_report();
    test_knob();
    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return EXIT_FAILURE;
    }
    puts("all hsm01 free-protocol tests passed");
    return EXIT_SUCCESS;
}
