#!/usr/bin/env python3
"""Read 32-bit little-endian literals from the raw firmware image (base 0x08000000).
Usage: python3 -I fw_word.py <fw_body.bin> <addr_hex> [addr_hex...]
"""
import struct
import sys

args = sys.argv[1:]
f64 = '--f64' in args  # read 64-bit doubles instead (e.g. Ghidra DAT_ constants passed in r2:r3)
args = [a for a in args if a != '--f64']
data = open(args[0], 'rb').read()
for a in args[1:]:
    off = int(a, 16) - 0x08000000
    if f64:
        print(f'{a}: {struct.unpack_from("<d", data, off)[0]!r}')
    else:
        print(f'{a}: {struct.unpack_from("<I", data, off)[0]:#010x}')
