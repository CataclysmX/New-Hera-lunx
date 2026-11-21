#!/usr/bin/env python3
"""Find a hex byte pattern in a file; print offsets (max 50) and count.
Usage: python3 -I find_bytes.py <file> <hex> [context_bytes=0]
"""
import sys

data = open(sys.argv[1], 'rb').read()
pat = bytes.fromhex(sys.argv[2])
ctx = int(sys.argv[3]) if len(sys.argv) > 3 else 0
i, n = data.find(pat), 0
while i >= 0:
    if n < 50:
        print(f'{i:#010x}', data[max(0, i - ctx):i + len(pat) + ctx].hex(' ') if ctx else '')
    n += 1
    i = data.find(pat, i + 1)
print('count', n)
