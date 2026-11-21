/*
 * libusb transport for the custom firmware ("free screen" protocol) - port of the Rust
 * `hsm01_usb::free::FreeDevice`: bulk OUT command stream + interrupt IN input reports, interface 0.
 */
#ifndef HSM01_FREE_USB_H
#define HSM01_FREE_USB_H

#include "hsm01/free.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct hsm01_free_device hsm01_free_device;

/* Open the device; HSM01_ERR_WRONG_FW if it runs the official firmware (version stored in major, minor). */
int hsm01_free_open(hsm01_free_device **out, uint8_t *major, uint8_t *minor);
void hsm01_free_close(hsm01_free_device *dev);

/* Send an encoded command stream (hsm01_free_* encoders). */
int hsm01_free_send(hsm01_free_device *dev, const uint8_t *data, size_t n);
/* Next input report: 1 (report), 0 (timeout) or a negative error. */
int hsm01_free_read_input(hsm01_free_device *dev, unsigned timeout_ms, hsm01_free_report *out);
/* Reboot into the ST DFU bootloader (0483:df11), same request as the official firmware. */
int hsm01_free_reboot_to_dfu(hsm01_free_device *dev);

#ifdef __cplusplus
}
#endif
#endif
