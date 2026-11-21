/* hsm01-c - command-line tool for the Hercules Stream 100 (C port of the Rust `hsm01`). */
#define _POSIX_C_SOURCE 200809L
#include "hsm01/background.h"
#include "hsm01/usb.h"
#include "image.h"

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static const char USAGE[] =
    "usage: hsm01-c <command>\n"
    "  info                 firmware, serial and device-info message\n"
    "  monitor [secs]       print button / knob / screen events (default 30 s)\n"
    "  demo [secs]          test pattern: icons, labels, LEDs, animated VU meters (default 20 s)\n"
    "  brightness <0-100>   set the screen backlight\n"
    "  led <0-3> <off|on|blink>\n"
    "  background <image> [secs]   full-screen picture, RAM only (default 15 s)\n"
    "  splash <image> --yes        write the boot logo to the device flash\n"
    "  splash default --yes        restore the factory boot logo\n"
    "  pictures: PNG, JPEG, GIF, BMP, PPM, scaled to fill 480x272 (256 colours)\n";

/* tools/hsm01_fw.c */
extern const char HSM01_FW_USAGE[];
int hsm01_fw_main(int argc, char **argv);

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static void sleep_ms(long ms)
{
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

static int check(int rc)
{
    if (rc < 0)
        fprintf(stderr, "hsm01-c: %s\n", hsm01_strerror(rc));
    return rc;
}

static int cmd_info(hsm01_device *d)
{
    uint8_t maj, min, sn[8];
    if (check(hsm01_firmware_version(d, &maj, &min)) < 0 || check(hsm01_serial(d, sn)) < 0)
        return -1;
    printf("firmware %u.%02u  serial", maj, min);
    for (int i = 0; i < 8; i++)
        printf(" %02x", sn[i]);
    printf("\n");
    hsm01_start_keepalive(d);
    double end = now_s() + 3;
    while (now_s() < end) {
        hsm01_report r;
        int rc = hsm01_poll(d, 100, &r);
        if (check(rc) < 0)
            return -1;
        if (rc == 1 && r.kind == HSM01_MSG_INFO) {
            const hsm01_device_info *i = &r.info;
            printf("type %u fw %u.%02u screen %ux%u icon %ux%u label %ux%u buttons %u leds %u\n", i->device_type,
                   i->fw_major, i->fw_minor, i->screen_w, i->screen_h, i->icon_w, i->icon_h, i->label_w, i->label_h,
                   i->button_count, i->led_count);
            return 0;
        }
    }
    printf("no device-info message received\n");
    return 0;
}

static int cmd_monitor(hsm01_device *d, double secs)
{
    hsm01_start_keepalive(d);
    printf("monitoring for %.0f s (Ctrl+C to stop)\n", secs);
    double end = now_s() + secs;
    hsm01_controls prev;
    bool have_prev = false;
    while (now_s() < end) {
        hsm01_report r;
        int rc = hsm01_poll(d, 100, &r);
        if (check(rc) < 0)
            return -1;
        if (rc == 0)
            continue;
        if (r.kind == HSM01_MSG_STATUS && (r.status.error || r.status.last_invalid_cmd))
            printf("device error 0x%04x last invalid cmd 0x%02x\n", r.status.error, r.status.last_invalid_cmd);
        if (!r.has_controls)
            continue;
        const hsm01_controls *c = &r.controls;
        if (have_prev) {
            uint16_t changed = c->buttons ^ prev.buttons;
            for (int bit = 0; bit < 9; bit++)
                if (changed & (1u << bit))
                    printf("%s %s\n", hsm01_button_names[bit], (c->buttons & (1u << bit)) ? "pressed" : "released");
            for (int k = 0; k < 4; k++) {
                int16_t delta = hsm01_encoder_delta(prev.encoders[k], c->encoders[k]);
                if (delta)
                    printf("knob%d %+d\n", k + 1, delta);
            }
            if (c->screen_on != prev.screen_on || c->brightness != prev.brightness)
                printf("screen %s %u%%\n", c->screen_on ? "on" : "off", c->brightness);
        }
        prev = *c;
        have_prev = true;
    }
    return 0;
}

static void draw_icon(uint32_t *px, uint32_t color)
{
    for (int y = 0; y < HSM01_ICON_H; y++)
        for (int x = 0; x < HSM01_ICON_W; x++) {
            int dx = x * 2 - 31, dy = y * 2 - 31;
            px[y * HSM01_ICON_W + x] = dx * dx + dy * dy <= 30 * 30 ? color : 0;
        }
}

static void draw_label(uint32_t *px, uint32_t color, int n)
{
    memset(px, 0, sizeof(uint32_t) * HSM01_LABEL_W * HSM01_LABEL_H);
    uint32_t r = (color >> 16) & 0xff, g = (color >> 8) & 0xff, b = color & 0xff;
    for (int y = 2; y < HSM01_LABEL_H - 2; y++)
        for (int x = 0; x < HSM01_LABEL_W; x++) {
            uint32_t k = 64 + (uint32_t)(x * 191 / HSM01_LABEL_W);
            px[y * HSM01_LABEL_W + x] = HSM01_RGB(r * k / 255, g * k / 255, b * k / 255);
        }
    for (int i = 0; i < n; i++)
        for (int y = 4; y < HSM01_LABEL_H - 4; y++)
            for (int x = 0; x < 3; x++)
                px[y * HSM01_LABEL_W + 8 + i * 8 + x] = HSM01_RGB(255, 255, 255);
}

static int cmd_demo(hsm01_device *d, double secs)
{
    static const uint32_t colors[HSM01_CHANNELS] = {HSM01_RGB(255, 64, 64), HSM01_RGB(64, 255, 64),
                                                    HSM01_RGB(64, 128, 255), HSM01_RGB(255, 200, 0)};
    static const uint8_t blocks[4][6] = {{0x81, 1, 7, 7, 0, 0x64}, {0x81, 1, 7, 7, 0, 0x64},
                                         {0x81, 1, 7, 7, 0, 0x64}, {0x81, 1, 7, 7, 0, 0x64}};
    static uint32_t icon[HSM01_ICON_W * HSM01_ICON_H], label[HSM01_LABEL_W * HSM01_LABEL_H];
    hsm01_buf b, rle_i, rle_l;
    hsm01_buf_init(&b);
    hsm01_buf_init(&rle_i);
    hsm01_buf_init(&rle_l);
    hsm01_start_keepalive(d);

    /* same configuration as the official app at start-up (captured) */
    hsm01_cmd_screen_on(&b);
    hsm01_cmd_screen_brightness(&b, 100);
    hsm01_cmd_led_intensity(&b, 90);
    hsm01_cmd_channel_display(&b, blocks);
    hsm01_cmd_screen_config(&b, 0, 4, 0xffffffffu, 0xffffffffu, 4, 0xffffffffu);
    for (uint8_t ch = 0; ch < HSM01_CHANNELS; ch++)
        for (uint8_t sub = 0; sub < 2; sub++) {
            const uint16_t vu_colors[3] = {0x2104, 0xf800, 0xffff};
            const uint16_t grad[2] = {sub ? 0x8410 : 0x2945, sub ? 0x8410 : 0x2945};
            hsm01_cmd_vu_style(&b, ch, sub, vu_colors, grad, 2);
        }
    int rc = check(hsm01_send(d, &b));
    for (uint8_t ch = 0; rc == 0 && ch < HSM01_CHANNELS; ch++) {
        draw_icon(icon, colors[ch]);
        draw_label(label, colors[ch], ch + 1);
        hsm01_buf_clear(&rle_i);
        hsm01_buf_clear(&rle_l);
        hsm01_rle_encode(icon, HSM01_ICON_W, HSM01_ICON_H, &rle_i);
        hsm01_rle_encode(label, HSM01_LABEL_W, HSM01_LABEL_H, &rle_l);
        for (int bank = 0; rc == 0 && bank < 2; bank++) {
            hsm01_buf_clear(&b);
            hsm01_cmd_icon(&b, (hsm01_bank)bank, ch, rle_i.data, rle_i.len);
            hsm01_cmd_label(&b, (hsm01_bank)bank, ch, rle_l.data, rle_l.len);
            rc = check(hsm01_send(d, &b));
        }
    }
    printf("demo running for %.0f s: icons/labels on 4 channels, LED chaser, animated VU meters\n", secs);
    double t0 = now_s();
    unsigned errors = 0;
    while (rc == 0 && now_s() - t0 < secs) {
        double t = now_s() - t0;
        int lit = (int)(t * 4.0) % HSM01_LEDS;
        hsm01_buf_clear(&b);
        for (uint8_t i = 0; i < HSM01_LEDS; i++)
            hsm01_cmd_led(&b, i, i == lit ? HSM01_LED_ON : HSM01_LED_OFF);
        for (uint8_t ch = 0; ch < HSM01_CHANNELS; ch++)
            for (uint8_t sub = 0; sub < 2; sub++) {
                double phase = t * (1.5 + ch * 0.4) + sub;
                uint16_t v = (uint16_t)((sin(phase) * 0.5 + 0.5) * 65535.0);
                hsm01_cmd_vu_level(&b, ch, sub, v, v, v > 62000);
            }
        rc = check(hsm01_send(d, &b));
        hsm01_report r;
        while (rc == 0 && hsm01_poll(d, 1, &r) == 1)
            if (r.kind == HSM01_MSG_STATUS && (r.status.error || r.status.last_invalid_cmd))
                errors++;
        sleep_ms(40);
    }
    printf("done, status reports with errors: %u\n", errors);
    hsm01_buf_free(&b);
    hsm01_buf_free(&rle_i);
    hsm01_buf_free(&rle_l);
    return rc;
}

/* ---- background / splash (port of the Rust CLI's logo.rs) ----------------------------------- */

/* The picture at `path`, scaled to the screen and reduced to 256 colours; NULL (said) on error. */
static hsm01_indexed_image *load_indexed(const char *path)
{
    char err[300];
    uint32_t *argb = malloc(sizeof(uint32_t) * HSM01_SCREEN_W * HSM01_SCREEN_H);
    hsm01_indexed_image *img = malloc(sizeof(*img));
    if (!argb || !img) {
        free(argb), free(img);
        check(HSM01_ERR_NOMEM);
        return NULL;
    }
    if (hsm01_load_image(path, argb, err, sizeof(err))) {
        fprintf(stderr, "hsm01-c: %s\n", err);
        free(argb), free(img);
        return NULL;
    }
    if (check(hsm01_quantize(argb, img)) < 0) {
        free(argb), free(img);
        return NULL;
    }
    free(argb);
    return img;
}

/* Show the picture as custom background (RAM only): the palette and the 32 tiles. */
static int upload(hsm01_device *d, const hsm01_indexed_image *img)
{
    uint8_t tile[HSM01_BG_TILE_BYTES];
    hsm01_buf b;
    hsm01_buf_init(&b);
    hsm01_cmd_screen_on(&b);
    /* host mode starts with the backlight at 0: without this the screen looks off (live test) */
    hsm01_cmd_screen_brightness(&b, 100);
    hsm01_cmd_screen_config(&b, 0, 0xff /* custom image */, 0, 0, 0, 0);
    int rc = check(hsm01_send(d, &b));
    for (int t = 0; rc == 0 && t < HSM01_BG_TILES; t++) {
        hsm01_buf_clear(&b);
        if (t == 0)
            hsm01_cmd_palette(&b, 0, img->palette, img->colors);
        hsm01_bg_tile(img, t, tile);
        hsm01_cmd_background_tile(&b, (uint8_t)t, tile, sizeof(tile));
        rc = check(b.oom ? HSM01_ERR_NOMEM : hsm01_send(d, &b));
        /* the SDK sends one tile every other frame: give the firmware time to blit it */
        for (double until = now_s() + 0.04; rc == 0 && now_s() < until;) {
            hsm01_report r;
            int got = hsm01_poll(d, 5, &r);
            if (got < 0) {
                rc = check(got);
            } else if (got > 0 && r.kind == HSM01_MSG_STATUS && (r.status.error || r.status.last_invalid_cmd)) {
                fprintf(stderr, "hsm01-c: device rejected tile %d: error 0x%04x\n", t, r.status.error);
                rc = -1;
            }
        }
    }
    hsm01_buf_free(&b);
    if (rc == 0)
        printf("background uploaded (%zu colors)\n", img->colors);
    return rc;
}

static int cmd_background(hsm01_device *d, const hsm01_indexed_image *img, double secs)
{
    int rc = upload(d, img);
    if (rc)
        return rc;
    printf("preview for %.0f s (RAM only, nothing written to flash)\n", secs);
    /* keep reading reports (poll() acknowledges device-info messages): a host that only sends
     * keep-alives is dropped by the official firmware after a few seconds (live test 2026-11-20) */
    for (double end = now_s() + secs; now_s() < end;) {
        hsm01_report r;
        int got = hsm01_poll(d, 100, &r);
        if (got < 0)
            return check(got);
    }
    return 0;
}

/* Wait for the flash write to finish (status byte: 0..100 progress, 0xfe error, 0xff idle). */
static int wait_flash(hsm01_device *d)
{
    int last = -1, started = 0;
    for (double deadline = now_s() + 60; now_s() < deadline;) {
        hsm01_report r;
        int got = hsm01_poll(d, 100, &r);
        if (got < 0)
            return check(got);
        if (got == 0 || r.kind != HSM01_MSG_STATUS)
            continue;
        if (r.status.error & 0x100) {
            fputs("hsm01-c: device refused the flash write (incomplete background?)\n", stderr);
            return -1;
        }
        int p = r.status.flash_status;
        if (p == 0xfe) {
            fputs("hsm01-c: flash write failed (device status 0xfe)\n", stderr);
            return -1;
        }
        if (p <= 100) {
            started = 1;
            if (p != last)
                printf("flash write %d%%\n", last = p);
            if (p == 100)
                return 0;
        } else if (started) {
            return 0; /* back to idle after completion */
        }
    }
    fputs("hsm01-c: timeout waiting for the flash write\n", stderr);
    return -1;
}

/* `img` NULL: the factory logo back. */
static int cmd_splash(hsm01_device *d, const hsm01_indexed_image *img)
{
    int rc = img ? upload(d, img) : 0;
    if (rc == 0) {
        hsm01_buf b;
        hsm01_buf_init(&b);
        hsm01_cmd_splash_flash(&b, img != NULL);
        rc = check(hsm01_send(d, &b));
        hsm01_buf_free(&b);
    }
    if (rc == 0)
        rc = wait_flash(d);
    if (rc == 0)
        puts("boot logo updated - unplug/replug the device to see it");
    return rc;
}

/* `background <image> [secs]`, `splash <image|default> --yes`. */
static int picture_command(int argc, char **argv)
{
    const char *cmd = argv[1], *target = NULL;
    int yes = 0;
    for (int i = 2; i < argc; i++) {
        if (!strcmp(argv[i], "--yes"))
            yes = 1;
        else if (!target)
            target = argv[i];
    }
    int splash = !strcmp(cmd, "splash");
    if (!target) {
        fputs(USAGE, stderr);
        return 2;
    }
    if (splash && !yes) {
        fputs("hsm01-c: this writes the boot logo to the device flash; re-run with --yes "
              "(`hsm01-c splash default --yes` restores the factory logo)\n",
              stderr);
        return 1;
    }
    double secs = 15;
    if (!splash && argc > 3) {
        char *end;
        secs = strtod(argv[3], &end);
        if (*end || secs <= 0)
            secs = 15;
    }
    hsm01_indexed_image *img = NULL;
    if (!(splash && !strcmp(target, "default")) && !(img = load_indexed(target)))
        return 1;
    hsm01_device *d;
    if (check(hsm01_open(&d)) < 0) {
        free(img);
        return 1;
    }
    hsm01_start_keepalive(d);
    int rc = splash ? cmd_splash(d, img) : cmd_background(d, img, secs);
    hsm01_close(d);
    free(img);
    return rc < 0 ? 1 : 0;
}

static int one_shot(hsm01_device *d, hsm01_buf *b)
{
    int rc = check(hsm01_keepalive(d));
    if (rc == 0)
        rc = check(hsm01_send(d, b));
    hsm01_buf_free(b);
    return rc;
}

int main(int argc, char **argv)
{
    if (argc < 2) {
        fputs(USAGE, stderr);
        fputs(HSM01_FW_USAGE, stderr);
        return 2;
    }
    const char *cmd = argv[1];
    if (!strcmp(cmd, "fw"))
        return hsm01_fw_main(argc - 1, argv + 1);
    if (!strcmp(cmd, "background") || !strcmp(cmd, "splash"))
        return picture_command(argc, argv);
    double secs = argc > 2 ? atof(argv[2]) : 0;
    hsm01_buf b;
    hsm01_buf_init(&b);
    if (!strcmp(cmd, "brightness")) {
        if (argc < 3) {
            fputs(USAGE, stderr);
            return 2;
        }
        hsm01_cmd_screen_on(&b);
        hsm01_cmd_screen_brightness(&b, (uint8_t)atoi(argv[2]));
    } else if (!strcmp(cmd, "led")) {
        int idx = argc > 2 ? atoi(argv[2]) : -1;
        const char *s = argc > 3 ? argv[3] : "";
        hsm01_led_state st = !strcmp(s, "on") ? HSM01_LED_ON : !strcmp(s, "blink") ? HSM01_LED_BLINK : HSM01_LED_OFF;
        if (idx < 0 || idx >= HSM01_LEDS || (strcmp(s, "on") && strcmp(s, "off") && strcmp(s, "blink"))) {
            fputs(USAGE, stderr);
            return 2;
        }
        hsm01_cmd_led(&b, (uint8_t)idx, st);
    } else if (strcmp(cmd, "info") && strcmp(cmd, "monitor") && strcmp(cmd, "demo")) {
        fputs(USAGE, stderr);
        fputs(HSM01_FW_USAGE, stderr);
        return 2;
    }

    hsm01_device *d;
    if (check(hsm01_open(&d)) < 0) {
        hsm01_buf_free(&b);
        return 1;
    }
    int rc;
    if (!strcmp(cmd, "info"))
        rc = cmd_info(d);
    else if (!strcmp(cmd, "monitor"))
        rc = cmd_monitor(d, secs > 0 ? secs : 30);
    else if (!strcmp(cmd, "demo"))
        rc = cmd_demo(d, secs > 0 ? secs : 20);
    else
        rc = one_shot(d, &b);
    hsm01_buf_free(&b);
    hsm01_close(d);
    return rc < 0 ? 1 : 0;
}
