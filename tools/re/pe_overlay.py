#!/usr/bin/env python3
"""Print PE sections, overlay offset and first bytes of the overlay.
Usage: python3 -I pe_overlay.py <file.exe>
"""
import struct
import sys

d = open(sys.argv[1], 'rb').read()
pe = struct.unpack_from('<I', d, 0x3c)[0]
ns = struct.unpack_from('<H', d, pe + 6)[0]
oh = struct.unpack_from('<H', d, pe + 20)[0]
st = pe + 24 + oh
end = 0
for i in range(ns):
    name, vs, va, rs, ro = struct.unpack_from('<8sIIII', d, st + i * 40)
    print(name, hex(va), hex(rs), hex(ro))
    end = max(end, ro + rs)
print('file size', hex(len(d)))
print('overlay at', hex(end))
print(d[end:end + 0x80].hex(' '))
print(d[end:end + 0x80])
i = d.find(b'InstallShield', end)
print('IS sig', hex(i))
