/* `hsm01-c fw ...` - commands for the custom firmware ("free screen" protocol), port of the Rust fw.rs. */
#define _POSIX_C_SOURCE 200809L
#include "hsm01/free_usb.h"
#include "image.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

const char HSM01_FW_USAGE[] =
    "custom firmware (firmware/rust, reports version 240.xx):\n"
    "  fw info                    custom firmware version\n"
    "  fw fill <r> <g> <b>        fill the whole screen\n"
    "  fw image <file>            show a picture (PNG, JPEG, GIF, BMP, PPM), scaled to fill 480x272\n"
    "  fw backlight <0-100>       screen backlight\n"
    "  fw leds <a> <b> <c> <d>    LED intensities 0-100\n"
    "  fw monitor [secs]          buttons + knob counts (512 per turn)\n"
    "  fw bench [frames]          full-screen update throughput\n"
    "  fw dfu                     reboot into the ST DFU bootloader\n";

int hsm01_fw_main(int argc, char **argv);

static double now_s(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec + (double)ts.tv_nsec / 1e9;
}

static int check(int rc)
{
    if (rc < 0)
        fprintf(stderr, "hsm01-c: %s\n", hsm01_strerror(rc));
    return rc;
}

/* Parse a 0-255 argument; -1 if missing or out of range. */
static int byte_arg(int argc, char **argv, int i)
{
    if (i >= argc)
        return -1;
    char *end;
    long v = strtol(argv[i], &end, 10);
    return *end || v < 0 || v > 255 ? -1 : (int)v;
}

/* Firmware with the screen saver (HSM01_FREE_SAVER_SINCE, 240.22): what was drawn stays only
 * while a host reads the input reports - read them until Ctrl+C, as the Rust CLI. */
static int hold(hsm01_free_device *d, uint8_t minor)
{
    if (minor < HSM01_FREE_SAVER_SINCE)
        return 0;
    puts("kept on the device until Ctrl+C");
    fflush(stdout);
    for (;;) {
        hsm01_free_report r;
        int rc = hsm01_free_read_input(d, 500, &r);
        if (rc < 0 && rc != HSM01_ERR_TRUNCATED)
            return check(rc);
    }
}

static int cmd_image(hsm01_free_device *d, const char *path)
{
    char err[300];
    uint32_t *argb = malloc(sizeof(uint32_t) * HSM01_SCREEN_W * HSM01_SCREEN_H);
    uint16_t *px = malloc(sizeof(uint16_t) * HSM01_SCREEN_W * HSM01_SCREEN_H);
    if (!argb || !px) {
        free(argb), free(px);
        return check(HSM01_ERR_NOMEM);
    }
    if (hsm01_load_image(path, argb, err, sizeof(err))) {
        fprintf(stderr, "hsm01-c: %s\n", err);
        free(argb), free(px);
        return -1;
    }
    for (size_t i = 0; i < (size_t)HSM01_SCREEN_W * HSM01_SCREEN_H; i++)
        px[i] = hsm01_rgb565((uint8_t)(argb[i] >> 16), (uint8_t)(argb[i] >> 8), (uint8_t)argb[i]);
    hsm01_buf b;
    hsm01_buf_init(&b);
    hsm01_free_blit_auto(&b, 0, 0, HSM01_SCREEN_W, HSM01_SCREEN_H, px);
    double t = now_s();
    int rc = b.oom ? HSM01_ERR_NOMEM : hsm01_free_send(d, b.data, b.len);
    if (check(rc) == 0)
        printf("sent %zu KiB in %.0f ms\n", b.len / 1024, (now_s() - t) * 1e3);
    hsm01_buf_free(&b);
    free(px);
    free(argb);
    return rc;
}

static int cmd_monitor(hsm01_free_device *d, double secs)
{
    printf("monitoring %.0f s - knobs in official-firmware counts (%d per cycle)\n", secs,
           HSM01_KNOB_COUNTS_PER_CYCLE);
    hsm01_knob_decoder dec[4];
    for (int k = 0; k < 4; k++)
        hsm01_knob_init(&dec[k]);
    int pending[4] = {0}, total[4] = {0};
    unsigned last_btn = 0xffff;
    double end = now_s() + secs, last_print = now_s();
    while (now_s() < end) {
        hsm01_free_report r;
        int rc = hsm01_free_read_input(d, 100, &r);
        if (rc == HSM01_ERR_TRUNCATED)
            continue;
        if (check(rc) < 0)
            return rc;
        if (rc == 0)
            continue;
        if (r.buttons != last_btn) {
            printf("buttons 0x%04x [", r.buttons);
            const char *sep = "";
            for (int bit = 0; bit < 9; bit++)
                if (r.buttons & (1u << bit)) {
                    printf("%s%s", sep, hsm01_button_names[bit]);
                    sep = ", ";
                }
            printf("]\n");
            last_btn = r.buttons;
        }
        for (int k = 0; k < 4; k++)
            pending[k] += hsm01_knob_update(&dec[k], r.wipers[2 * k], r.wipers[2 * k + 1]);
        if (now_s() - last_print > 0.05) {
            for (int k = 0; k < 4; k++)
                if (pending[k]) {
                    total[k] += pending[k];
                    printf("knob%d %+d (total %+d)\n", k + 1, pending[k], total[k]);
                    pending[k] = 0;
                }
            last_print = now_s();
        }
    }
    return 0;
}

static int cmd_bench(hsm01_free_device *d, long frames)
{
    static const uint8_t colors[4][3] = {{255, 0, 0}, {0, 255, 0}, {0, 0, 255}, {255, 255, 255}};
    uint16_t *px = malloc(sizeof(uint16_t) * HSM01_SCREEN_W * HSM01_SCREEN_H);
    if (!px)
        return check(HSM01_ERR_NOMEM);
    hsm01_buf b;
    hsm01_buf_init(&b);
    size_t bytes = 0;
    int rc = 0;
    double t = now_s(), sending = 0;
    for (long i = 0; rc == 0 && i < frames; i++) {
        /* raw full-screen blit: worst case (no compression) */
        const uint8_t *c = colors[i % 4];
        uint16_t v = hsm01_rgb565(c[0], c[1], c[2]);
        for (size_t j = 0; j < HSM01_SCREEN_W * HSM01_SCREEN_H; j++)
            px[j] = v;
        hsm01_buf_clear(&b);
        hsm01_free_blit(&b, 0, 0, HSM01_SCREEN_W, HSM01_SCREEN_H, px);
        double t1 = now_s();
        rc = check(b.oom ? HSM01_ERR_NOMEM : hsm01_free_send(d, b.data, b.len));
        sending += now_s() - t1;
        bytes += b.len;
    }
    double s = now_s() - t;
    if (rc == 0)
        printf("%ld raw full-screen frames in %.2f s: %.1f fps, %.0f KiB/s (USB %.0f KiB/s)\n", frames, s,
               (double)frames / s, (double)bytes / 1024.0 / s, (double)bytes / 1024.0 / sending);
    hsm01_buf_free(&b);
    free(px);
    return rc;
}

int hsm01_fw_main(int argc, char **argv)
{
    /* argv[0] = "fw", argv[1] = sub-command */
    const char *sub = argc > 1 ? argv[1] : "";
    hsm01_buf b;
    hsm01_buf_init(&b);
    int a[4] = {byte_arg(argc, argv, 2), byte_arg(argc, argv, 3), byte_arg(argc, argv, 4), byte_arg(argc, argv, 5)};
    if (!strcmp(sub, "fill") && a[0] >= 0 && a[1] >= 0 && a[2] >= 0) {
        hsm01_free_fill(&b, 0, 0, HSM01_SCREEN_W, HSM01_SCREEN_H, hsm01_rgb565((uint8_t)a[0], (uint8_t)a[1], (uint8_t)a[2]));
    } else if (!strcmp(sub, "backlight") && a[0] >= 0) {
        hsm01_free_backlight(&b, (uint8_t)a[0]);
    } else if (!strcmp(sub, "leds") && a[0] >= 0 && a[1] >= 0 && a[2] >= 0 && a[3] >= 0) {
        const uint8_t pct[4] = {(uint8_t)a[0], (uint8_t)a[1], (uint8_t)a[2], (uint8_t)a[3]};
        hsm01_free_leds(&b, pct);
    } else if (strcmp(sub, "info") && strcmp(sub, "monitor") && strcmp(sub, "bench") && strcmp(sub, "dfu") &&
               (strcmp(sub, "image") || argc < 3)) {
        fputs(HSM01_FW_USAGE, stderr);
        return 2;
    }

    hsm01_free_device *d;
    uint8_t maj = 0, min = 0;
    int rc = hsm01_free_open(&d, &maj, &min);
    if (rc == HSM01_ERR_WRONG_FW)
        fprintf(stderr, "hsm01-c: device runs firmware %u.%02u, not the custom firmware (240.xx)\n", maj, min);
    if (rc < 0) {
        if (rc != HSM01_ERR_WRONG_FW)
            check(rc);
        hsm01_buf_free(&b);
        return 1;
    }
    if (!strcmp(sub, "info"))
        printf("custom firmware %u.%02u (stage %u)\n", maj, min, min);
    else if (!strcmp(sub, "image"))
        rc = cmd_image(d, argv[2]) < 0 ? -1 : hold(d, min);
    else if (!strcmp(sub, "monitor"))
        rc = cmd_monitor(d, argc > 2 && atof(argv[2]) > 0 ? atof(argv[2]) : 30);
    else if (!strcmp(sub, "bench"))
        rc = cmd_bench(d, argc > 2 && atol(argv[2]) > 0 ? atol(argv[2]) : 30);
    else if (!strcmp(sub, "dfu")) {
        rc = hsm01_free_reboot_to_dfu(d);
        printf("rebooting into DFU (0483:df11)\n");
    } else {
        rc = check(b.oom ? HSM01_ERR_NOMEM : hsm01_free_send(d, b.data, b.len));
        if (rc == 0)
            rc = hold(d, min);
    }
    hsm01_buf_free(&b);
    hsm01_free_close(d);
    return rc < 0 ? 1 : 0;
}
