#!/usr/bin/env python3
"""Read bytes at a virtual address of a PE file and print them (hex + common decodings).
Usage: python3 -I pe_read.py <file> <va_hex> [len=16] [struct_fmt]
  e.g. pe_read.py hsm_api_core_x64.dll 0x180108dc0 8 <d
"""
import struct
import sys

import pefile

pe = pefile.PE(sys.argv[1], fast_load=True)
va = int(sys.argv[2], 16)
n = int(sys.argv[3]) if len(sys.argv) > 3 else 16
data = pe.get_data(va - pe.OPTIONAL_HEADER.ImageBase, n)
print(f'{va:#x}: {data.hex(" ")}')
if len(sys.argv) > 4:
    print(struct.unpack_from(sys.argv[4], data))
