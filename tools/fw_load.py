#!/usr/bin/env python3
"""Input reports under display load (custom firmware): FILL messages are sent as fast as possible
while a second thread reads the input reports (interrupt IN 0x81, ~100/s when idle). Prints, per
load pattern, the fill speed and the input report rate / longest gap between two reports.
The companion app must be stopped (it holds the interface).
Usage: python3 -I tools/fw_load.py [seconds per pattern, default 4]
"""
import struct
import sys
import threading
import time

import usb.core
import usb.util


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def fill(x, y, w, h, color):
    return msg(0x01, struct.pack('<5H', x, y, w, h, color))


def columns(color):
    """48 full-height spectrum columns (the old app behaviour): ~92 000 pixels."""
    return b''.join(fill(x * 10, 32, 8, 240, color) for x in range(48)), 48 * 8 * 240


def full(color):
    return fill(0, 0, 480, 272, color), 480 * 272


def small(color):
    """Many small fills (row deltas): 200 × 8×4 pixels."""
    return b''.join(fill((i % 48) * 10, 32 + (i // 48) * 8, 8, 4, color) for i in range(200)), 200 * 32


def measure(dev, name, pattern, seconds):
    stop = threading.Event()
    times = []

    def reader():
        while not stop.is_set():
            try:
                dev.read(0x81, 64, timeout=200)
                times.append(time.monotonic())
            except usb.core.USBTimeoutError:
                pass

    t = threading.Thread(target=reader)
    t.start()
    t0, pixels, i = time.monotonic(), 0, 0
    while time.monotonic() - t0 < seconds:
        data, n = pattern(0x0841 if i % 2 else 0x10A2)
        if data:
            dev.write(0x01, data, timeout=5000)
        else:
            time.sleep(0.01)
        pixels += n
        i += 1
    dt = time.monotonic() - t0
    stop.set()
    t.join()
    gaps = [b - a for a, b in zip(times, times[1:])]
    print(f'{name:<8} {pixels / dt / 1e6:6.1f} Mpx/s   reports {len(times) / dt:5.0f}/s   '
          f'longest gap {max(gaps, default=dt) * 1000:6.0f} ms')


def main():
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 4.0
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    version = bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))
    if version[1] != 0xF0:
        sys.exit('not the custom firmware')
    print(f'firmware 240.{version[0]:02}')
    usb.util.claim_interface(dev, 0)
    measure(dev, 'idle', lambda c: (b'', 0), min(seconds, 2.0))
    for name, pattern in (('small', small), ('columns', columns), ('full', full)):
        measure(dev, name, pattern, seconds)
    dev.write(0x01, fill(0, 0, 480, 272, 0), timeout=5000)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
