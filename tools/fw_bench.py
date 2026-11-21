#!/usr/bin/env python3
"""Bulk OUT throughput of the custom firmware, per message kind, to locate the bottleneck:
NOP (op 0x7F, skipped by the parser: pure USB cost) vs raw BLIT (parser + framebuffer).
Usage: python3 -I tools/fw_bench.py [KiB per test, default 512]
"""
import struct
import sys
import time

import usb.core
import usb.util


def msg(op, payload):
    return struct.pack('<BBHI', 0xA5, op, 0, len(payload)) + payload


def run(dev, name, data, reps=1):
    t = time.monotonic()
    for _ in range(reps):
        dev.write(0x01, data, timeout=20000)
    dt = time.monotonic() - t
    print(f'{name:<28} {reps * len(data) / 1024 / dt:7.0f} KiB/s')


def main():
    kib = int(sys.argv[1]) if len(sys.argv) > 1 else 512
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    if bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))[1] != 0xF0:
        sys.exit('not the custom firmware')
    usb.util.claim_interface(dev, 0)
    n = kib * 1024
    run(dev, 'NOP (USB only)', msg(0x7F, bytes(n)))
    px = (n // 2) // 480 * 480
    run(dev, 'BLIT raw (480 wide)', msg(0x02, struct.pack('<4H', 0, 0, 480, px // 480)
                                         + struct.pack('<H', 0x0841) * px))
    for size in (64, 512, 4096):
        run(dev, f'NOP, {size}-byte writes', msg(0x7F, bytes(size - 8)), reps=n // size // 4)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
