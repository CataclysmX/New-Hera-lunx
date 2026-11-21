#!/usr/bin/env python3
"""Find literal-pool words equal to a value in the raw firmware image (base 0x08000000).
Usage: python3 -I fw_find_word.py <fw_body.bin> <value_hex>
"""
import struct
import sys

data = open(sys.argv[1], 'rb').read()
val = int(sys.argv[2], 16)
for off in range(0, len(data) - 3, 4):
    if struct.unpack_from('<I', data, off)[0] == val:
        print(f'{0x08000000 + off:#010x}')
