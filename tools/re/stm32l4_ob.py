#!/usr/bin/env python3
"""Decode STM32L4+ (L4Rx/L4Sx) option bytes read through DFU (alt 1, 0x1FF00000, 64 bytes).
Layout: pairs of (value u32, complement u32). Ref: RM0432 "Option bytes".
Usage: python3 -I stm32l4_ob.py ob_bank1.bin [ob_bank2.bin]
"""
import struct
import sys

OPTR_BITS = [
    (8, 3, 'BOR_LEV'), (12, 1, 'nRST_STOP'), (13, 1, 'nRST_STDBY'), (14, 1, 'nRST_SHDW'),
    (16, 1, 'IWDG_SW'), (17, 1, 'IWDG_STOP'), (18, 1, 'IWDG_STDBY'), (19, 1, 'WWDG_SW'),
    (20, 1, 'BFB2'), (21, 1, 'DB1M'), (22, 1, 'DBANK'), (23, 1, 'nBOOT1'), (24, 1, 'SRAM2_PE'),
    (25, 1, 'SRAM2_RST'), (26, 1, 'nSWBOOT0'), (27, 1, 'nBOOT0'),
]


def words(path):
    d = open(path, 'rb').read()
    return [struct.unpack_from('<II', d, o) for o in range(0, len(d) - 7, 8)]


b1 = words(sys.argv[1])
optr = b1[0][0]
rdp = optr & 0xff
level = {0xaa: '0 (no protection)', 0xcc: '2 (PERMANENT lock)'}.get(rdp, '1 (read protection)')
print(f'OPTR {optr:#010x} (complement ok: {b1[0][0] ^ b1[0][1] == 0xffffffff})')
print(f'  RDP = {rdp:#04x} -> level {level}')
for shift, width, name in OPTR_BITS:
    print(f'  {name:10} = {(optr >> shift) & ((1 << width) - 1)}')
names = ['PCROP1_STR', 'PCROP1_END', 'WRP1A', 'WRP1B']
for name, (v, c) in zip(names, b1[1:5]):
    print(f'{name:11} {v:#010x} (complement ok: {v ^ c == 0xffffffff})')
if len(sys.argv) > 2:
    for name, (v, c) in zip(['PCROP2_STR', 'PCROP2_END', 'WRP2A', 'WRP2B'], words(sys.argv[2])[1:5]):
        print(f'{name:11} {v:#010x} (complement ok: {v ^ c == 0xffffffff})')
