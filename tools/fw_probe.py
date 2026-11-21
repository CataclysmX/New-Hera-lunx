#!/usr/bin/env python3
"""Quick health check of the custom firmware: control endpoint (version), input reports (interrupt
IN 0x81) and display stream (bulk OUT 0x01, one small FILL), each with its own timeout.
The companion app must be stopped. Usage: python3 -I tools/fw_probe.py
"""
import struct
import sys
import time

import usb.core
import usb.util


def step(name, f):
    t = time.monotonic()
    try:
        r = f()
        print(f'OK  {name:<10} {1000 * (time.monotonic() - t):6.1f} ms  {r if r is not None else ""}')
    except usb.core.USBError as e:
        print(f'KO  {name:<10} {1000 * (time.monotonic() - t):6.1f} ms  {e}')


def main():
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    step('version', lambda: bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 500)).hex())
    usb.util.claim_interface(dev, 0)
    for i in range(5):
        step(f'report {i}', lambda: bytes(dev.read(0x81, 64, timeout=300))[:4].hex())
    fill = struct.pack('<BBHI5H', 0xA5, 0x01, 0, 10, 0, 0, 16, 16, 0x07E0)
    step('fill', lambda: dev.write(0x01, fill, timeout=500))
    step('version', lambda: bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 500)).hex())
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
