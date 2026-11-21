/*
 * libusb transport for the Hercules Stream 100 - port of the Rust crate `hsm01-usb`.
 * Thread-safe: hsm01_send*() may be called from several threads (the keep-alive thread
 * included) while another thread calls hsm01_poll().
 */
#ifndef HSM01_USB_H
#define HSM01_USB_H

#include "hsm01/proto.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hsm01_device hsm01_device;

/* Open the first Stream 100 and run the init sequence (config 1, interfaces 0/1, alt 1). */
int hsm01_open(hsm01_device **out);
/* Stop the keep-alive thread (if any), release interfaces, close. */
void hsm01_close(hsm01_device *dev);

int hsm01_firmware_version(hsm01_device *dev, uint8_t *major, uint8_t *minor);
int hsm01_serial(hsm01_device *dev, uint8_t serial[8]);

/* Frame raw command bytes and send them as one isochronous transfer. */
int hsm01_send_raw(hsm01_device *dev, const uint8_t *cmds, size_t n);
/* Append the END marker to `cmds` and send. */
int hsm01_send(hsm01_device *dev, hsm01_buf *cmds);
int hsm01_keepalive(hsm01_device *dev);

/* Read one report: returns 1 (report), 0 (timeout) or a negative error.
 * Device-info messages are acknowledged automatically (0x14) unless disabled. */
int hsm01_poll(hsm01_device *dev, unsigned timeout_ms, hsm01_report *out);
void hsm01_set_auto_ack(hsm01_device *dev, bool on);

/* Background keep-alive every 500 ms (required while driving the device). */
int hsm01_start_keepalive(hsm01_device *dev);
void hsm01_stop_keepalive(hsm01_device *dev);

#ifdef __cplusplus
}
#endif
#endif
