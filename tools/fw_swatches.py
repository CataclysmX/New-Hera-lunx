#!/usr/bin/env python3
"""Show colour swatches on the custom firmware to pick a colour as the panel renders it
(2 rows x 6, column number = white dots under each swatch). Backlight 80 % like the boot screen.
Usage: python3 -I tools/fw_swatches.py [#rrggbb ...]   (default: 12 emerald candidates)
"""
import struct
import sys

import usb.core
import usb.util

W, H = 480, 272
DEFAULT = ['00674f', '005c4b', '00524a', '004a44', '00463c', '003d36',
           '1e6655', '23594d', '2c5f55', '164f45', '0f4a3f', '2a4f47']


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def fill(x, y, w, h, c):
    return msg(0x01, struct.pack('<5H', x, y, w, h, c))


def rgb565(h):
    r, g, b = int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
    return ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3)


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
    colours = [a.lstrip('#') for a in sys.argv[1:]] or DEFAULT
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    s = msg(0x10, bytes([80])) + fill(0, 0, W, H, 0)
    sw, sh, gap = 66, 100, 12
    x0 = (W - 6 * sw - 5 * gap) // 2
    for i, c in enumerate(colours[:12]):
        row, col = divmod(i, 6)
        x, y = x0 + col * (sw + gap), 16 + row * (sh + 30)
        s += fill(x, y, sw, sh, rgb565(c))
        for d in range(col + 1):  # column number as dots
            s += fill(x + 4 + d * 10, y + sh + 6, 5, 5, 0xFFFF)
        print(f'row {row + 1} col {col + 1}: #{c} (RGB565 0x{rgb565(c):04x})')
    dev.write(0x01, s, timeout=5000)
    hold(dev)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
