/* libusb transport for the Hercules Stream 100 - see include/hsm01/usb.h. */
#define _POSIX_C_SOURCE 200809L
#include "hsm01/usb.h"

#include <errno.h>
#include <libusb.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define CTRL_TIMEOUT_MS 1000
#define ISO_TIMEOUT_MS 1000
#define KEEPALIVE_MS 500

struct hsm01_device {
    libusb_context *ctx;
    libusb_device_handle *h;
    pthread_mutex_t send_lock; /* serialises sequence numbers and isochronous writes */
    uint16_t seq;
    atomic_bool auto_ack;
    /* keep-alive thread */
    pthread_t ka_thread;
    bool ka_running;
    bool ka_stop;
    pthread_mutex_t ka_lock;
    pthread_cond_t ka_cond;
};

int hsm01_open(hsm01_device **out)
{
    *out = NULL;
    hsm01_device *d = calloc(1, sizeof(*d));
    if (!d)
        return HSM01_ERR_NOMEM;
    pthread_mutex_init(&d->send_lock, NULL);
    pthread_mutex_init(&d->ka_lock, NULL);
    pthread_cond_init(&d->ka_cond, NULL);
    atomic_init(&d->auto_ack, true);

    int rc = HSM01_ERR_USB;
    if (libusb_init(&d->ctx) < 0)
        goto fail;
    d->h = libusb_open_device_with_vid_pid(d->ctx, HSM01_VID, HSM01_PID);
    if (!d->h) {
        rc = HSM01_ERR_NOT_FOUND;
        goto fail;
    }
    libusb_set_auto_detach_kernel_driver(d->h, 1); /* no kernel driver binds, be tolerant */
    int cfg = 0;
    if (libusb_get_configuration(d->h, &cfg) < 0 || cfg != 1) {
        if (libusb_set_configuration(d->h, 1) < 0)
            goto fail;
    }
    if (libusb_claim_interface(d->h, 0) < 0 || libusb_claim_interface(d->h, 1) < 0)
        goto fail;
    if (libusb_set_interface_alt_setting(d->h, 1, 1) < 0)
        goto fail;
    *out = d;
    return HSM01_OK;

fail:
    if (d->h)
        libusb_close(d->h);
    if (d->ctx)
        libusb_exit(d->ctx);
    pthread_cond_destroy(&d->ka_cond);
    pthread_mutex_destroy(&d->ka_lock);
    pthread_mutex_destroy(&d->send_lock);
    free(d);
    return rc;
}

void hsm01_close(hsm01_device *d)
{
    if (!d)
        return;
    hsm01_stop_keepalive(d);
    libusb_set_interface_alt_setting(d->h, 1, 0);
    libusb_release_interface(d->h, 1);
    libusb_release_interface(d->h, 0);
    libusb_close(d->h);
    libusb_exit(d->ctx);
    pthread_cond_destroy(&d->ka_cond);
    pthread_mutex_destroy(&d->ka_lock);
    pthread_mutex_destroy(&d->send_lock);
    free(d);
}

int hsm01_firmware_version(hsm01_device *d, uint8_t *major, uint8_t *minor)
{
    uint8_t b[2];
    int n = libusb_control_transfer(d->h, 0xc0, HSM01_REQ_GET_FW_VERSION, 0, 0, b, sizeof(b), CTRL_TIMEOUT_MS);
    if (n < 0)
        return HSM01_ERR_USB;
    if (n != 2)
        return HSM01_ERR_BAD_REPLY;
    *major = b[1];
    *minor = b[0];
    return HSM01_OK;
}

int hsm01_serial(hsm01_device *d, uint8_t serial[8])
{
    uint8_t b[9];
    int n = libusb_control_transfer(d->h, 0xc0, HSM01_REQ_READ_INFO, 1, 0, b, sizeof(b), CTRL_TIMEOUT_MS);
    if (n < 0)
        return HSM01_ERR_USB;
    if (n != 9 || b[0] != 8)
        return HSM01_ERR_BAD_REPLY;
    memcpy(serial, b + 1, 8);
    return HSM01_OK;
}

static void LIBUSB_CALL iso_done(struct libusb_transfer *t) { *(int *)t->user_data = 1; }

/* Synchronous isochronous OUT transfer; `len` is a multiple of 952. */
static int write_iso(hsm01_device *d, uint8_t *buf, size_t len)
{
    int packets = (int)(len / HSM01_ISO_PACKET);
    struct libusb_transfer *t = libusb_alloc_transfer(packets);
    if (!t)
        return HSM01_ERR_NOMEM;
    int completed = 0;
    libusb_fill_iso_transfer(t, d->h, HSM01_EP_ISO_OUT, buf, (int)len, packets, iso_done, &completed,
                             ISO_TIMEOUT_MS);
    libusb_set_iso_packet_lengths(t, HSM01_ISO_PACKET);
    if (libusb_submit_transfer(t) < 0) {
        libusb_free_transfer(t);
        return HSM01_ERR_USB;
    }
    while (!completed) {
        int rc = libusb_handle_events_completed(d->ctx, &completed);
        if (rc < 0 && rc != LIBUSB_ERROR_INTERRUPTED)
            libusb_cancel_transfer(t);
    }
    int status = t->status;
    libusb_free_transfer(t);
    return status == LIBUSB_TRANSFER_COMPLETED ? HSM01_OK : HSM01_ERR_ISO;
}

int hsm01_send_raw(hsm01_device *d, const uint8_t *cmds, size_t n)
{
    hsm01_buf b;
    hsm01_buf_init(&b);
    pthread_mutex_lock(&d->send_lock);
    int rc = hsm01_build_frame(d->seq, cmds, n, &b);
    if (rc == HSM01_OK) {
        d->seq++;
        rc = write_iso(d, b.data, b.len);
    }
    pthread_mutex_unlock(&d->send_lock);
    hsm01_buf_free(&b);
    return rc;
}

int hsm01_send(hsm01_device *d, hsm01_buf *cmds)
{
    hsm01_cmd_end(cmds);
    if (cmds->oom)
        return HSM01_ERR_NOMEM;
    return hsm01_send_raw(d, cmds->data, cmds->len);
}

int hsm01_keepalive(hsm01_device *d)
{
    uint8_t ka[26];
    hsm01_buf b;
    hsm01_buf_init(&b);
    hsm01_cmd_keepalive(&b);
    int rc = b.oom ? HSM01_ERR_NOMEM : HSM01_OK;
    if (rc == HSM01_OK) {
        memcpy(ka, b.data, b.len);
        rc = hsm01_send_raw(d, ka, b.len);
    }
    hsm01_buf_free(&b);
    return rc;
}

void hsm01_set_auto_ack(hsm01_device *d, bool on) { atomic_store(&d->auto_ack, on); }

int hsm01_poll(hsm01_device *d, unsigned timeout_ms, hsm01_report *out)
{
    uint8_t buf[HSM01_INT_REPORT];
    int n = 0;
    int rc = libusb_interrupt_transfer(d->h, HSM01_EP_INT_IN, buf, sizeof(buf), &n, timeout_ms);
    if (rc == LIBUSB_ERROR_TIMEOUT)
        return 0;
    if (rc < 0)
        return HSM01_ERR_USB;
    rc = hsm01_parse_report(buf, (size_t)n, out);
    if (rc < 0)
        return rc;
    if (out->kind == HSM01_MSG_INFO && atomic_load(&d->auto_ack)) {
        const uint8_t ack[2] = {0x14, 0x00};
        rc = hsm01_send_raw(d, ack, sizeof(ack));
        if (rc < 0)
            return rc;
    }
    return 1;
}

static void *keepalive_main(void *arg)
{
    hsm01_device *d = arg;
    pthread_mutex_lock(&d->ka_lock);
    while (!d->ka_stop) {
        pthread_mutex_unlock(&d->ka_lock);
        hsm01_keepalive(d);
        pthread_mutex_lock(&d->ka_lock);
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_nsec += KEEPALIVE_MS * 1000000L;
        ts.tv_sec += ts.tv_nsec / 1000000000L;
        ts.tv_nsec %= 1000000000L;
        while (!d->ka_stop && pthread_cond_timedwait(&d->ka_cond, &d->ka_lock, &ts) != ETIMEDOUT) {
        }
    }
    pthread_mutex_unlock(&d->ka_lock);
    return NULL;
}

int hsm01_start_keepalive(hsm01_device *d)
{
    if (d->ka_running)
        return HSM01_OK;
    d->ka_stop = false;
    if (pthread_create(&d->ka_thread, NULL, keepalive_main, d) != 0)
        return HSM01_ERR_NOMEM;
    d->ka_running = true;
    return HSM01_OK;
}

void hsm01_stop_keepalive(hsm01_device *d)
{
    if (!d->ka_running)
        return;
    pthread_mutex_lock(&d->ka_lock);
    d->ka_stop = true;
    pthread_cond_signal(&d->ka_cond);
    pthread_mutex_unlock(&d->ka_lock);
    pthread_join(d->ka_thread, NULL);
    d->ka_running = false;
}
