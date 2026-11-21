#!/usr/bin/env python3
"""Dump Interrupt-IN reports of a capture TSV (from wsdump.py).

Usage: python3 -I in_reports.py <capture.tsv> [changes|sm]
  changes : print control block only when it changes (frame, time, bytes, diff mask)
  sm      : print distinct 'sm' packets (type, payload)
"""
import sys


def reports(path):
    for line in open(path):
        c = line.rstrip('\n').split('\t')
        if len(c) < 7 or c[4] != 'BULK_OR_INTERRUPT_TRANSFER' or c[3] != 'host' or not c[6]:
            continue
        d = bytes.fromhex(c[6])
        if len(d) < 2:
            continue
        yield c[0], c[1], d


def main():
    path = sys.argv[1]
    mode = sys.argv[2] if len(sys.argv) > 2 else 'changes'
    prev, seen = None, set()
    for frame, t, d in reports(path):
        n = d[0]
        ctl, sm = d[:n], d[n:]
        if mode == 'changes':
            if ctl != prev:
                diff = '' if prev is None or len(prev) != len(ctl) else ''.join(
                    '^^' if a != b else '..' for a, b in zip(prev, ctl))
                print(f'{frame:>6} {float(t):9.3f} {ctl.hex(" ")}   {diff}')
                prev = ctl
        elif mode == 'sm' and sm[:2] == b'sm':
            key = sm[6:sm[2]]
            k2 = (sm[6], key[1:] if sm[6] != 2 else key[1:])
            if k2 not in seen:
                seen.add(k2)
                print(f'{frame:>6} {float(t):9.3f} len={sm[2]} pk={sm[4]:02x} type={sm[6]} {sm[7:sm[2]].hex(" ")}')


if __name__ == '__main__':
    main()
