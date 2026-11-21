#!/usr/bin/env python3
"""Geometry test pattern for the custom firmware: 1-px white border, vertical lines every 60 px,
horizontal lines every 68 px, coloured 40x40 corner squares (TL red, TR green, BL blue, BR yellow).
Straight vertical lines + full border + squares in the corners = LTDC geometry OK.
Usage: python3 -I tools/fw_geometry.py
"""
import struct
import sys

import usb.core
import usb.util

W, H = 480, 272


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def fill(x, y, w, h, c):
    return msg(0x01, struct.pack('<5H', x, y, w, h, c))


def hold(dev):
    """From 240.22 the firmware's screen saver takes the screen back 3 s after its host stops
    reading the input reports: read them until Ctrl+C so that the picture stays."""
    print('kept on the device until Ctrl+C')
    try:
        while True:
            try:
                dev.read(0x81, 64, timeout=500)
            except usb.core.USBTimeoutError:
                pass
    except KeyboardInterrupt:
        pass


def main():
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    white = 0xFFFF
    s = fill(0, 0, W, H, 0)
    for x in range(0, W, 60):
        s += fill(x, 0, 1, H, white)
    for y in range(0, H, 68):
        s += fill(0, y, W, 1, white)
    s += fill(W - 1, 0, 1, H, white) + fill(0, H - 1, W, 1, white)
    s += fill(0, 0, 40, 40, 0xF800) + fill(W - 40, 0, 40, 40, 0x07E0)
    s += fill(0, H - 40, 40, 40, 0x001F) + fill(W - 40, H - 40, 40, 40, 0xFFE0)
    dev.write(0x01, s, timeout=5000)
    print('geometry pattern drawn')
    hold(dev)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
