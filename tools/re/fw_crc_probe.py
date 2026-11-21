#!/usr/bin/env python3
"""Try common CRC32 variants over a raw firmware body to identify FWFG header checksums.
Usage: python3 -I fw_crc_probe.py <file.hsm>
"""
import struct
import sys
import zlib


def crc32_stm32(data, init=0xffffffff):
    """STM32 hardware CRC: poly 0x04C11DB7, MSB first, 32-bit LE words, no final xor."""
    crc = init
    for (w,) in struct.iter_unpack('<I', data[:len(data) & ~3]):
        crc ^= w
        for _ in range(32):
            crc = ((crc << 1) ^ 0x04C11DB7) & 0xffffffff if crc & 0x80000000 else (crc << 1) & 0xffffffff
    return crc


d = open(sys.argv[1], 'rb').read()
h, body = d[:0x80], d[0x80:]
targets = {struct.unpack_from('<I', h, o)[0]: o for o in range(0, 0x80, 4)}
size54 = struct.unpack_from('<I', h, 0x54)[0]
for name, blob in (('body', body), ('body[:@0x54]', body[:size54]), ('header[:0x7c]', h[:0x7c]),
                   ('header[:0x50]', h[:0x50])):
    for label, val in (('zlib', zlib.crc32(blob)), ('zlib^~', zlib.crc32(blob) ^ 0xffffffff),
                       ('stm32', crc32_stm32(blob)), ('stm32^~', crc32_stm32(blob) ^ 0xffffffff)):
        hit = f'  <== header @{targets[val]:#04x}' if val in targets else ''
        print(f'{name:14} {label:8} {val:#010x}{hit}')
