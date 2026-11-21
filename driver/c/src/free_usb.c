/* libusb transport for the custom firmware - see include/hsm01/free_usb.h. */
#include "hsm01/free_usb.h"

#include <libusb.h>
#include <stdlib.h>

#define CTRL_TIMEOUT_MS 1000
#define BULK_TIMEOUT_MS 5000
#define BULK_CHUNK (64 * 1024) /* chunk size does not change throughput (measured) */

struct hsm01_free_device {
    libusb_context *ctx;
    libusb_device_handle *h;
};

int hsm01_free_open(hsm01_free_device **out, uint8_t *major, uint8_t *minor)
{
    *out = NULL;
    hsm01_free_device *d = calloc(1, sizeof(*d));
    if (!d)
        return HSM01_ERR_NOMEM;
    int rc = HSM01_ERR_USB;
    if (libusb_init(&d->ctx) < 0)
        goto fail;
    d->h = libusb_open_device_with_vid_pid(d->ctx, HSM01_VID, HSM01_PID);
    if (!d->h) {
        rc = HSM01_ERR_NOT_FOUND;
        goto fail;
    }
    uint8_t v[2];
    int n = libusb_control_transfer(d->h, 0xc0, HSM01_REQ_GET_FW_VERSION, 0, 0, v, sizeof(v), CTRL_TIMEOUT_MS);
    if (n < 0)
        goto fail;
    if (n != 2) {
        rc = HSM01_ERR_BAD_REPLY;
        goto fail;
    }
    *major = v[1];
    *minor = v[0];
    if (v[1] != HSM01_FREE_FW_MAJOR) {
        rc = HSM01_ERR_WRONG_FW;
        goto fail;
    }
    libusb_set_auto_detach_kernel_driver(d->h, 1);
    if (libusb_claim_interface(d->h, 0) < 0)
        goto fail;
    *out = d;
    return HSM01_OK;

fail:
    if (d->h)
        libusb_close(d->h);
    if (d->ctx)
        libusb_exit(d->ctx);
    free(d);
    return rc;
}

void hsm01_free_close(hsm01_free_device *d)
{
    if (!d)
        return;
    libusb_release_interface(d->h, 0);
    libusb_close(d->h);
    libusb_exit(d->ctx);
    free(d);
}

int hsm01_free_send(hsm01_free_device *d, const uint8_t *data, size_t n)
{
    for (size_t off = 0; off < n;) {
        int len = (int)(n - off < BULK_CHUNK ? n - off : BULK_CHUNK);
        int done = 0;
        /* libusb does not write through the pointer for OUT transfers */
        if (libusb_bulk_transfer(d->h, HSM01_FREE_EP_BULK_OUT, (uint8_t *)(uintptr_t)(data + off), len, &done,
                                 BULK_TIMEOUT_MS) < 0)
            return HSM01_ERR_USB;
        if (done != len)
            return HSM01_ERR_BAD_REPLY;
        off += (size_t)len;
    }
    return HSM01_OK;
}

int hsm01_free_read_input(hsm01_free_device *d, unsigned timeout_ms, hsm01_free_report *out)
{
    uint8_t buf[64];
    int n = 0;
    int rc = libusb_interrupt_transfer(d->h, HSM01_FREE_EP_INT_IN, buf, sizeof(buf), &n, timeout_ms);
    if (rc == LIBUSB_ERROR_TIMEOUT)
        return 0;
    if (rc < 0)
        return HSM01_ERR_USB;
    return hsm01_free_parse_report(buf, (size_t)n, out) ? 1 : HSM01_ERR_TRUNCATED;
}

int hsm01_free_reboot_to_dfu(hsm01_free_device *d)
{
    /* the device resets while answering: errors are expected and ignored */
    libusb_control_transfer(d->h, 0x40, 0xF2, 0, HSM01_PID, NULL, 0, CTRL_TIMEOUT_MS);
    return HSM01_OK;
}
