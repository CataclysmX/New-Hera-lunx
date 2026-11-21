#!/usr/bin/env python3
"""Image-retention A/B test on the custom firmware (same pictures as tools/mk_ab_images.py):
checker = black/white checkerboard 80x68, grey = uniform 50 % grey; backlight 100 %.
Usage: python3 -I tools/fw_ab.py checker|grey
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
    mode = sys.argv[1] if len(sys.argv) > 1 else ''
    if mode not in ('checker', 'grey'):
        sys.exit(__doc__)
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    s = msg(0x10, bytes([100]))
    if mode == 'grey':
        s += fill(0, 0, W, H, 0x8410)
    else:
        for cy in range(0, H, 68):
            for cx in range(0, W, 80):
                s += fill(cx, cy, 80, 68, 0xFFFF if (cx // 80 + cy // 68) % 2 else 0)
    dev.write(0x01, s, timeout=5000)
    print(f'{mode} drawn')
    hold(dev)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
