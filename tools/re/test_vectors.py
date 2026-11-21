#!/usr/bin/env python3
"""Print SM messages (trimmed to their length) from a capture TSV, for use as unit-test vectors.
Usage: python3 -I test_vectors.py <capture.tsv> <max_len> [count=3]
"""
import struct
import sys

path, max_len = sys.argv[1], int(sys.argv[2])
count = int(sys.argv[3]) if len(sys.argv) > 3 else 3
ISO = 952
seen = set()
for line in open(path):
    c = line.rstrip('\n').split('\t')
    if len(c) < 7 or c[4] != 'ISOCH_TRANSFER' or c[2] != 'host':
        continue
    d = bytes.fromhex(c[6])
    if d[:8] != b'HERCULES' or d[ISO:ISO + 2] != b'SM':
        continue
    m = d[ISO:]
    n = struct.unpack_from('<H', m, 2)[0]
    key = m[8:n]
    if n <= max_len and key not in seen:
        seen.add(key)
        print(f'frame {c[0]} len {n}: {m[:n].hex()}')
        count -= 1
        if count == 0:
            break
