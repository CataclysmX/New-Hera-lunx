#!/usr/bin/env python3
"""VU-meter smoothness demo on the custom firmware: 8 bars (4 channels x 2) drawn host-side with
FILL commands (~700 bytes per frame), green/yellow/red segments + falling peak hold.
Usage: python3 -I tools/fw_vu_demo.py [fps=60] [secs=15]
Compare e.g. `... 12 10` (choppy, like a low host update rate) with `... 60 10`.
"""
import math
import random
import struct
import sys
import time

import usb.core
import usb.util

W, H = 480, 272
BARS, BAR_W, GAP, TOP, BAR_H = 8, 34, 22, 40, 200
X0 = (W - (BARS * BAR_W + (BARS - 1) * GAP)) // 2
GREEN, YELLOW, RED, OFF, PEAK = 0x07E0, 0xFFE0, 0xF800, 0x2104, 0xFFFF


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def fill(x, y, w, h, c):
    return msg(0x01, struct.pack('<5H', x, y, w, h, c)) if w > 0 and h > 0 else b''


def bar(i, level, peak):
    """Commands for bar i: lit part in 3 colour zones, dark rest, 2-px peak marker."""
    x, lit = X0 + i * (BAR_W + GAP), int(level * BAR_H)
    out = fill(x, TOP, BAR_W, BAR_H - lit, OFF)
    for lo, hi, c in ((0.0, 0.6, GREEN), (0.6, 0.85, YELLOW), (0.85, 1.0, RED)):
        a, b = int(lo * BAR_H), min(int(hi * BAR_H), lit)
        if b > a:
            out += fill(x, TOP + BAR_H - b, BAR_W, b - a, c)
    py = TOP + BAR_H - max(int(peak * BAR_H), 2)
    return out + fill(x, py, BAR_W, 2, PEAK)


def main():
    fps = float(sys.argv[1]) if len(sys.argv) > 1 else 60
    secs = float(sys.argv[2]) if len(sys.argv) > 2 else 15
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    dev.write(0x01, msg(0x10, bytes([100])) + fill(0, 0, W, H, 0), timeout=5000)
    peaks, frames, sent, t0 = [0.0] * BARS, 0, 0, time.monotonic()
    level = [0.0] * BARS
    while (now := time.monotonic()) - t0 < secs:
        t = now - t0
        s = b''
        for i in range(BARS):
            # music-like envelope: slow sine + beat + noise, smoothed like a real VU ballistic
            target = 0.45 + 0.3 * math.sin(t * (1.1 + 0.37 * i)) + 0.25 * max(0, math.sin(t * 7.3 + i))
            target = min(1.0, max(0.0, target + random.uniform(-0.08, 0.08)))
            level[i] += (target - level[i]) * (0.6 if target > level[i] else 0.15)
            peaks[i] = max(level[i], peaks[i] - 0.4 / max(fps, 1))
            s += bar(i, level[i], peaks[i])
        dev.write(0x01, s, timeout=1000)
        frames, sent = frames + 1, sent + len(s)
        time.sleep(max(0.0, t0 + frames / fps - time.monotonic()))
    dt = time.monotonic() - t0
    usb.util.release_interface(dev, 0)
    print(f'{frames} frames in {dt:.1f} s = {frames / dt:.1f} fps, {sent / frames:.0f} B/frame, '
          f'{sent / dt / 1024:.0f} KiB/s')


if __name__ == '__main__':
    main()
