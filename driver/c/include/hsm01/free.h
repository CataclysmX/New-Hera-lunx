/*
 * "Free screen" protocol of the custom firmware (firmware/rust, version 240.xx) - pure C11, no I/O.
 * Port of the Rust module `hsm01_proto::free`; see docs/firmware/protocol.md and knobs.md.
 *
 * Host -> device: byte stream on bulk EP 0x01, messages [0xA5][op][0 0][len u32 LE][payload].
 * Device -> host: 24-byte input report on interrupt EP 0x81 every 10 ms.
 */
#ifndef HSM01_FREE_H
#define HSM01_FREE_H

#include "hsm01/proto.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Major version reported by vendor request 0x02 on the custom firmware. */
#define HSM01_FREE_FW_MAJOR 0xF0
#define HSM01_FREE_EP_BULK_OUT 0x01
#define HSM01_FREE_EP_INT_IN 0x81

enum {
    HSM01_FREE_OP_FILL = 0x01,
    HSM01_FREE_OP_BLIT = 0x02,
    HSM01_FREE_OP_BLIT_RLE = 0x03,
    HSM01_FREE_OP_COPY = 0x04, /* 240.21 and later */
    HSM01_FREE_OP_BACKLIGHT = 0x10,
    HSM01_FREE_OP_LEDS = 0x11,
    HSM01_FREE_OP_NOP = 0x7F,
};

/* First firmware minor version (240.xx) that knows COPY. */
#define HSM01_FREE_COPY_SINCE 0x15
/* First minor version with the screen saver: without a host reading the input reports (or
 * sending) for 3 s it takes the screen back (hsm01_free_report.screen_epoch). */
#define HSM01_FREE_SAVER_SINCE 0x16

/* RGB888 -> RGB565. */
uint16_t hsm01_rgb565(uint8_t r, uint8_t g, uint8_t b);

/* Encoders: append one message to `out`. Pixels are row-major RGB565, w*h entries. */
void hsm01_free_fill(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t color);
void hsm01_free_blit(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px);
void hsm01_free_blit_rle(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px);
/* Smallest of raw / RLE. */
void hsm01_free_blit_auto(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *px);
/* Copy the rectangle (x, y, w, h) of the device's screen so that its top-left corner goes to
 * (to_x, to_y), overlap allowed (scrolling) - firmware 240.21 and later (HSM01_FREE_COPY_SINCE). */
void hsm01_free_copy(hsm01_buf *out, uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t to_x,
                     uint16_t to_y);
void hsm01_free_backlight(hsm01_buf *out, uint8_t percent);
void hsm01_free_leds(hsm01_buf *out, const uint8_t percent[4]);

/* Input report (interrupt IN). */
typedef struct {
    uint8_t seq;
    uint16_t buttons;    /* same bits as the official firmware: HSM01_BTN_* */
    uint16_t wipers[8];  /* raw 12-bit ADC: knob1 A/B, knob2 A/B, knob3 A/B, knob4 A/B */
    uint8_t backlight;   /* percent */
    /* Changes each time the firmware took the screen back (screen saver, HSM01_FREE_SAVER_SINCE):
     * what the host drew is gone, redraw everything. Always 0 before. */
    uint8_t screen_epoch;
} hsm01_free_report;

/* Returns true if `b` holds a valid report. */
bool hsm01_free_parse_report(const uint8_t *b, size_t n, hsm01_free_report *out);

/* ---- endless dual-wiper knobs (quadrature triangle wipers, docs/firmware/knobs.md) ---------- */
/* Counts per cycle (~ one turn), same unit and sign as the official firmware's accumulators. */
#define HSM01_KNOB_COUNTS_PER_CYCLE 512

/* Position on the wiper diamond, in 1/65536 cycle (gain-normalised). */
uint16_t hsm01_knob_phase(uint16_t a, uint16_t b);

typedef struct {
    bool valid;
    uint16_t reference;
} hsm01_knob_decoder;

void hsm01_knob_init(hsm01_knob_decoder *d);
/* Delta in counts since the previous call (positive = clockwise), one count of hysteresis. */
int hsm01_knob_update(hsm01_knob_decoder *d, uint16_t a, uint16_t b);

#ifdef __cplusplus
}
#endif
#endif
