#!/usr/bin/env python3
"""Record the raw knob wipers of the custom firmware (8 x 12-bit ADC, report every 10 ms) to a CSV,
to design the dual-wiper endless-potentiometer decoder.
Usage: python3 -I tools/fw_knobs.py [secs=40] [out.csv]   (default out: re_work/knobs/<time>.csv)
Suggested: each knob in turn, one slow full turn clockwise, then one counter-clockwise.
"""
import os
import struct
import sys
import time

import usb.core
import usb.util

NAMES = ['k1a', 'k1b', 'k2a', 'k2b', 'k3a', 'k3b', 'k4a', 'k4b']


def main():
    secs = float(sys.argv[1]) if len(sys.argv) > 1 else 40
    out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
        're_work', 'knobs', time.strftime('%Y%m%d-%H%M%S') + '.csv')
    os.makedirs(os.path.dirname(out) or '.', exist_ok=True)
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    rows, t0 = [], time.monotonic()
    print(f'recording {secs:.0f} s -> {out}  (turn one knob at a time)')
    while time.monotonic() - t0 < secs:
        try:
            r = bytes(dev.read(0x81, 64, timeout=200))
        except usb.core.USBTimeoutError:
            continue
        if len(r) < 21 or r[0] != 1:
            continue
        w = struct.unpack_from('<8H', r, 4)
        rows.append((time.monotonic() - t0, r[1], struct.unpack_from('<H', r, 2)[0], *w))
    usb.util.release_interface(dev, 0)
    with open(out, 'w') as f:
        f.write('t,seq,buttons,' + ','.join(NAMES) + '\n')
        for row in rows:
            f.write(f'{row[0]:.3f},{row[1]},{row[2]},' + ','.join(map(str, row[3:])) + '\n')
    print(f'{len(rows)} reports')
    for i, n in enumerate(NAMES):
        col = [row[3 + i] for row in rows]
        if col:
            print(f'  {n}: min {min(col):4d}  max {max(col):4d}  span {max(col) - min(col):4d}')


if __name__ == '__main__':
    main()
