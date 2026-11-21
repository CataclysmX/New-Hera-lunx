#!/usr/bin/env python3
"""Dump the 0x80-byte FWFG header of a Hercules .hsm firmware file as integers/strings,
and check candidate CRC32 values over the payload.
Usage: python3 -I fwfg_header.py <file.hsm>
"""
import struct
import sys
import zlib

d = open(sys.argv[1], 'rb').read()
h, body = d[:0x80], d[0x80:]
print('magic', h[:4], 'size', len(d), 'body', len(body))
for off in range(0, 0x80, 4):
    w = struct.unpack_from('<I', h, off)[0]
    txt = ''.join(chr(c) if 32 <= c < 127 else '.' for c in h[off:off + 4])
    print(f'{off:#04x}: {w:#010x} {w:>11} {txt}')
crc = zlib.crc32(body)
print(f'crc32(body)={crc:#010x}  crc32(body)^~0={crc ^ 0xffffffff:#010x}')
words = {struct.unpack_from('<I', h, o)[0] for o in range(0, 0x80, 4)}
print('crc32 found in header:', crc in words or (crc ^ 0xffffffff) in words)
vec_sp, vec_reset = struct.unpack_from('<II', body, 0)
print(f'vector table: SP={vec_sp:#010x} reset={vec_reset:#010x}')
