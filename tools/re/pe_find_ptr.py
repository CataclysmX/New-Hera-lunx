#!/usr/bin/env python3
"""Find 8-byte pointers equal to a VA inside a PE64 image (e.g. to locate vtables).
Usage: python3 -I pe_find_ptr.py <file> <va_hex> [va_hex...]
"""
import struct
import sys

import pefile

pe = pefile.PE(sys.argv[1], fast_load=True)
base = pe.OPTIONAL_HEADER.ImageBase
targets = {int(a, 16) for a in sys.argv[2:]}
for s in pe.sections:
    data = s.get_data()
    for off in range(0, len(data) - 7, 8):
        v = struct.unpack_from('<Q', data, off)[0]
        if v in targets:
            print(f'{v:#x} referenced at {base + s.VirtualAddress + off:#x} ({s.Name.rstrip(b"\0").decode()})')
