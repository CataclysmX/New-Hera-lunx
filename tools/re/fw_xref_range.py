#!/usr/bin/env python3
"""Find decompiled functions that use literal-pool words pointing into [lo, hi).
Usage: python3 -I fw_xref_range.py <fw_body.bin> <decomp.c> <lo_hex> <hi_hex>
Prints: literal address, value, DAT_ name, functions using it.
"""
import re
import struct
import sys

data = open(sys.argv[1], 'rb').read()
src = open(sys.argv[2]).read().splitlines()
lo, hi = int(sys.argv[3], 16), int(sys.argv[4], 16)
func_at = []
cur = None
for line in src:
    m = re.match(r'// ==== (\S+) @', line)
    if m:
        cur = m.group(1)
    func_at.append(cur)
for off in range(0, len(data) - 3, 4):
    v = struct.unpack_from('<I', data, off)[0]
    if lo <= v < hi:
        name = f'DAT_{0x08000000 + off:08x}'
        users = sorted({func_at[i] for i, l in enumerate(src) if name in l and func_at[i]})
        if users:
            print(f'{0x08000000 + off:#010x} = {v:#010x}  {name}  -> {" ".join(users)}')
