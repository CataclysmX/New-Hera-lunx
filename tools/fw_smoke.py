#!/usr/bin/env python3
"""End-to-end smoke test of the custom firmware "free screen" protocol (docs/firmware/protocol.md).
Draws: blue background, 4 coloured squares (FILL), a gradient (BLIT), stripes (BLIT_RLE);
sets backlight + LEDs; prints a few input reports and measures bulk throughput.
Usage: python3 -I tools/fw_smoke.py
"""
import struct
import sys
import time

import usb.core
import usb.util

W, H = 480, 272


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def rect(x, y, w, h):
    return struct.pack('<4H', x, y, w, h)


def rgb565(r, g, b):
    return ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3)


def main():
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    v = bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))
    print(f'firmware {v[1]}.{v[0]:02d}  product {usb.util.get_string(dev, dev.iProduct)!r}')
    if v[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)

    s = msg(0x01, rect(0, 0, W, H) + struct.pack('<H', rgb565(10, 20, 60)))
    for i, c in enumerate([(255, 64, 64), (64, 255, 64), (64, 128, 255), (255, 200, 0)]):
        s += msg(0x01, rect(20 + i * 115, 20, 100, 100) + struct.pack('<H', rgb565(*c)))
    grad = b''.join(struct.pack('<H', rgb565(x * 255 // 439, 0, 255 - x * 255 // 439))
                    for _y in range(40) for x in range(440))
    s += msg(0x02, rect(20, 140, 440, 40) + grad)
    runs = b''.join(struct.pack('<BH', 20, rgb565(255, 255, 255) if k % 2 else rgb565(0, 0, 0))
                    for k in range(22)) * 40   # 22 runs × 20 px = 440 px per row
    s += msg(0x03, rect(20, 200, 440, 40) + runs)
    s += msg(0x10, bytes([90])) + msg(0x11, bytes([100, 50, 20, 5]))
    t = time.monotonic()
    dev.write(0x01, s, timeout=5000)
    print(f'sent {len(s)} bytes in {(time.monotonic() - t) * 1e3:.0f} ms')

    for _ in range(3):
        r = bytes(dev.read(0x81, 64, timeout=1000))
        seq, btn = r[1], struct.unpack_from('<H', r, 2)[0]
        wipers = struct.unpack_from('<8H', r, 4)
        print(f'report seq={seq} buttons={btn:#06x} wipers={wipers} backlight={r[20]}%')

    # throughput: 10 raw full-screen frames
    frame = msg(0x02, rect(0, 0, W, H) + struct.pack('<H', rgb565(10, 20, 60)) * (W * H))
    t = time.monotonic()
    for _ in range(10):
        dev.write(0x01, frame, timeout=10000)
    dt = time.monotonic() - t
    print(f'raw full-screen: {10 / dt:.1f} fps, {10 * len(frame) / 1024 / dt:.0f} KiB/s')
    dev.write(0x01, s, timeout=5000)  # redraw the test scene
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
