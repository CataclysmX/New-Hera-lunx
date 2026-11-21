#!/usr/bin/env python3
"""Carve uncompressed Qt rcc payloads (SVG / XML / JSON) from a binary.

Qt rcc stores each file as [u32 BE size][bytes]. We look for payload starts ('<?xml', '<svg',
'{') preceded by a plausible BE size whose end is consistent, and dump them.
Usage: python3 -I qrc_carve.py <binary> <empty_out_dir>
"""
import os
import re
import struct
import sys

src, out = sys.argv[1], sys.argv[2]
data = open(src, 'rb').read()
os.makedirs(out, exist_ok=True)
n = 0
for m in re.finditer(rb'<\?xml|<svg', data):
    s = m.start()
    if s < 4:
        continue
    size = struct.unpack_from('>I', data, s - 4)[0]
    if not (32 <= size <= 4_000_000) or s + size > len(data):
        continue
    blob = data[s:s + size]
    if b'</svg>' not in blob[-64:] and not blob.rstrip().endswith(b'>'):
        continue
    ids = re.findall(rb'id="([^"]+)"', blob)[:3]
    tag = b'_'.join(ids).decode('ascii', 'replace')[:60] or 'noid'
    tag = re.sub(r'[^A-Za-z0-9_.-]', '_', tag)
    name = f'{s:08x}_{tag}.svg'
    with open(os.path.join(out, name), 'xb') as f:
        f.write(blob)
    n += 1
print('carved', n)

# zlib-compressed rcc entries: [u32 BE stored size][u32 BE uncompressed size][zlib stream]
import zlib  # noqa: E402

z = 0
for m in re.finditer(rb'\x78[\x9c\xda\x01\x5e]', data):
    s = m.start()
    if s < 8:
        continue
    stored, usize = struct.unpack_from('>II', data, s - 8)
    if not (16 <= stored <= 8_000_000 and stored - 4 <= usize <= 50_000_000) or s + stored - 4 > len(data):
        continue
    try:
        blob = zlib.decompress(data[s:s + stored - 4])
    except zlib.error:
        continue
    if len(blob) != usize:
        continue
    head = blob[:200].lstrip()
    ext = 'svg' if b'<svg' in blob[:2000] else 'xml' if head.startswith(b'<') else \
        'json' if head[:1] in (b'{', b'[') else 'png' if blob[:4] == b'\x89PNG' else 'bin'
    with open(os.path.join(out, f'z{s:08x}.{ext}'), 'xb') as f:
        f.write(blob)
    z += 1
print('zlib entries', z)
