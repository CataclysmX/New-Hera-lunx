/*
 * Hercules Stream 100 ("HSM01") wire protocol - pure C11, no I/O.
 * Port of the Rust crate `hsm01-proto`; see the docs/protocol directory.
 */
#ifndef HSM01_PROTO_H
#define HSM01_PROTO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define HSM01_VID 0x06f8
#define HSM01_PID 0xe053
#define HSM01_EP_ISO_OUT 0x01
#define HSM01_EP_INT_IN 0x81
#define HSM01_ISO_PACKET 952
#define HSM01_INT_REPORT 64
#define HSM01_MAX_SM_LEN (9 * HSM01_ISO_PACKET)

#define HSM01_SCREEN_W 480
#define HSM01_SCREEN_H 272
#define HSM01_CHANNELS 4
#define HSM01_LEDS 4
#define HSM01_ICON_W 32
#define HSM01_ICON_H 32
#define HSM01_LABEL_W 110
#define HSM01_LABEL_H 16

/* Vendor requests (bmRequestType 0xC0). 0xF2 (reboot to DFU) is deliberately not exposed. */
#define HSM01_REQ_GET_FW_VERSION 0x02
#define HSM01_REQ_READ_INFO 0x52

/* Error codes (negative). */
enum {
    HSM01_OK = 0,
    HSM01_ERR_NOT_FOUND = -1,
    HSM01_ERR_USB = -2,
    HSM01_ERR_ISO = -3,
    HSM01_ERR_TOO_LONG = -4,
    HSM01_ERR_BAD_HEADER = -5,
    HSM01_ERR_BAD_CRC = -6,
    HSM01_ERR_TRUNCATED = -7,
    HSM01_ERR_BAD_SIGNATURE = -8,
    HSM01_ERR_BAD_CHECKSUM = -9,
    HSM01_ERR_NOMEM = -10,
    HSM01_ERR_BAD_REPLY = -11,
    HSM01_ERR_INVALID = -12,
    HSM01_ERR_WRONG_FW = -13, /* custom-firmware API used on the official firmware (or vice versa) */
};

const char *hsm01_strerror(int err);

/* ---- growable byte buffer ---------------------------------------------------------------- */
typedef struct {
    uint8_t *data;
    size_t len;
    size_t cap;
    bool oom; /* set if an allocation failed; further appends are ignored */
} hsm01_buf;

void hsm01_buf_init(hsm01_buf *b);
void hsm01_buf_free(hsm01_buf *b);
void hsm01_buf_clear(hsm01_buf *b);
void hsm01_buf_put(hsm01_buf *b, const void *data, size_t n);
void hsm01_buf_put_u8(hsm01_buf *b, uint8_t v);
void hsm01_buf_put_u16(hsm01_buf *b, uint16_t v); /* little endian */
void hsm01_buf_put_u32(hsm01_buf *b, uint32_t v); /* little endian */

/* ---- CRC / framing (host -> device) ------------------------------------------------------ */
uint16_t hsm01_crc16(const uint8_t *data, size_t n, uint16_t init);
/* CRC of a complete SM message of `len` bytes (bytes 4..6 excluded). */
uint16_t hsm01_sm_crc(const uint8_t *msg, size_t len);
/* Append the SM message for `cmds` to `out`. */
int hsm01_build_sm(uint16_t seq, const uint8_t *cmds, size_t n, hsm01_buf *out);
/* Append the isochronous buffer ("HERCULES" packet + SM + zero padding) to `out`. */
int hsm01_build_frame(uint16_t seq, const uint8_t *cmds, size_t n, hsm01_buf *out);
/* Validate an SM message; on success returns its sequence and command bytes. */
int hsm01_parse_sm(const uint8_t *msg, size_t n, uint16_t *seq, const uint8_t **cmds, size_t *ncmds);

/* ---- commands (appended to a buffer) ----------------------------------------------------- */
typedef enum { HSM01_LED_OFF = 0, HSM01_LED_ON = 1, HSM01_LED_BLINK = 2 } hsm01_led_state;
typedef enum { HSM01_BANK_A = 0, HSM01_BANK_B = 1 } hsm01_bank;

void hsm01_cmd_host_info(hsm01_buf *b, const uint16_t os[4], uint8_t sdk_minor_bcd, uint8_t sdk_major);
void hsm01_cmd_ping(hsm01_buf *b, uint8_t v);
void hsm01_cmd_keepalive(hsm01_buf *b); /* HOST_INFO + 0x11 0x00, as the SDK every 500 ms */
void hsm01_cmd_screen_on(hsm01_buf *b);
void hsm01_cmd_screen_off(hsm01_buf *b);
void hsm01_cmd_ack_device_info(hsm01_buf *b);
void hsm01_cmd_reset_display(hsm01_buf *b);
void hsm01_cmd_led(hsm01_buf *b, uint8_t index, hsm01_led_state state);
void hsm01_cmd_screen_brightness(hsm01_buf *b, uint8_t percent);
void hsm01_cmd_led_intensity(hsm01_buf *b, uint8_t percent);
void hsm01_cmd_channel_display(hsm01_buf *b, const uint8_t blocks[4][6]);
void hsm01_cmd_vu_style(hsm01_buf *b, uint8_t channel, uint8_t sub, const uint16_t colors[3],
                        const uint16_t *gradient, size_t n);
void hsm01_cmd_icon(hsm01_buf *b, hsm01_bank bank, uint8_t channel, const uint8_t *rle, size_t n);
void hsm01_cmd_label(hsm01_buf *b, hsm01_bank bank, uint8_t channel, const uint8_t *rle, size_t n);
void hsm01_cmd_screen_config(hsm01_buf *b, uint8_t screen_type, uint8_t background_type, uint32_t darkest_rgb,
                             uint32_t lightest_rgb, uint8_t action_zone_style, uint32_t action_zone_rgb);
void hsm01_cmd_vu_markers(hsm01_buf *b, uint8_t channel, uint8_t sub, const uint16_t values[4], bool wide);
void hsm01_cmd_vu_level(hsm01_buf *b, uint8_t channel, uint8_t sub, uint16_t level, uint16_t peak, bool clip);
void hsm01_cmd_vu_inactive(hsm01_buf *b, uint8_t channel, uint8_t sub);
/* 0x33: background palette entries offset..offset+n (RGB565, 256 at most in total). */
void hsm01_cmd_palette(hsm01_buf *b, uint8_t offset, const uint16_t *colors, size_t n);
/* 0x37: background tile `tile` (0..31): 4080 palette indices (padded / cut to that size);
 * see background.h. Send each tile in its own frame. */
void hsm01_cmd_background_tile(hsm01_buf *b, uint8_t tile, const uint8_t *data, size_t n);
/* 0x50: boot logo in flash; custom = true stores the current background (all 32 tiles must
 * have been received, otherwise status error 0x100), false restores the factory logo. */
void hsm01_cmd_splash_flash(hsm01_buf *b, bool custom);
void hsm01_cmd_end(hsm01_buf *b);
/* Length of the command at `b[0]` (firmware dispatcher), or -1 if unknown/truncated. */
int hsm01_command_len(const uint8_t *b, size_t n);

/* ---- RLE images (icons 32x32, labels 110x16) --------------------------------------------- */
#define HSM01_RGB(r, g, b) (0xff000000u | ((uint32_t)(r) << 16) | ((uint32_t)(g) << 8) | (uint32_t)(b))
void hsm01_argb_to_a4_565(uint32_t argb, uint8_t *a4, uint16_t *rgb565);
int hsm01_rle_encode(const uint32_t *pixels, size_t width, size_t height, hsm01_buf *out);
/* Decode to per-pixel alpha4/rgb565 arrays of width*height entries. */
int hsm01_rle_decode(const uint8_t *data, size_t n, size_t width, size_t height, uint8_t *a4, uint16_t *rgb565);

/* ---- input reports (device -> host) ------------------------------------------------------ */
#define HSM01_BTN_KNOB1 (1u << 0)
#define HSM01_BTN_KNOB2 (1u << 1)
#define HSM01_BTN_KNOB3 (1u << 2)
#define HSM01_BTN_KNOB4 (1u << 3)
#define HSM01_BTN_BUTTON1 (1u << 4)
#define HSM01_BTN_BUTTON2 (1u << 5)
#define HSM01_BTN_BUTTON3 (1u << 6)
#define HSM01_BTN_BUTTON4 (1u << 7)
#define HSM01_BTN_POWER (1u << 8)
extern const char *const hsm01_button_names[9];

typedef struct {
    uint16_t buttons;
    int16_t encoders[4]; /* endless knob accumulators: use hsm01_encoder_delta() */
    bool screen_on;
    uint8_t brightness; /* percent */
} hsm01_controls;

typedef struct {
    uint8_t device_type;
    uint8_t fw_major, fw_minor;
    uint8_t serial[9]; /* hardware revision + 8-byte serial */
    uint16_t screen_w, screen_h;
    uint16_t icon_w, icon_h;
    uint16_t label_w, label_h;
    uint8_t button_count, led_count;
} hsm01_device_info;

typedef struct {
    uint16_t error; /* 0x08 CRC, 0x10 signature, 0x20 length, 0x40 unknown opcode */
    uint16_t last_seq;
    uint16_t error_count;
    uint8_t serial[9];
    uint16_t factory_code;
    uint8_t last_invalid_cmd;
    uint8_t render_ms;
    uint8_t flash_status;
} hsm01_status;

typedef enum { HSM01_MSG_NONE, HSM01_MSG_INFO, HSM01_MSG_STATUS, HSM01_MSG_UNKNOWN } hsm01_msg_kind;

typedef struct {
    bool has_controls;
    hsm01_controls controls;
    hsm01_msg_kind kind;
    uint8_t unknown_kind;
    hsm01_device_info info;   /* valid if kind == HSM01_MSG_INFO */
    hsm01_status status;      /* valid if kind == HSM01_MSG_STATUS */
} hsm01_report;

int hsm01_parse_message(const uint8_t *sm, size_t n, hsm01_report *out);
int hsm01_parse_report(const uint8_t *buf, size_t n, hsm01_report *out);
int16_t hsm01_encoder_delta(int16_t old_value, int16_t new_value);

#ifdef __cplusplus
}
#endif
#endif
