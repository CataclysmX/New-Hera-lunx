#!/usr/bin/env python3
"""Unpack the overlay of an InstallShield "InstallScript Setup Launcher Unicode" exe.

Overlay layout (observed on Hercules_Stream_Control_v2.5.4.exe, IS 31):
    u32 file_count
    repeat file_count:
        utf16z name, utf16z relpath, utf16z version, utf16z size_decimal
        u8[size] data  (stored raw, no XOR in this build)

Usage: python3 -I is_unpack.py <setup.exe> <empty_out_dir>
"""
import os
import struct
import sys


def overlay_offset(d):
    pe = struct.unpack_from('<I', d, 0x3c)[0]
    ns = struct.unpack_from('<H', d, pe + 6)[0]
    oh = struct.unpack_from('<H', d, pe + 20)[0]
    st = pe + 24 + oh
    end = 0
    for i in range(ns):
        _, _, _, rs, ro = struct.unpack_from('<8sIIII', d, st + i * 40)
        end = max(end, ro + rs)
    return end


def read_wstr(d, off):
    end = off
    while d[end:end + 2] != b'\x00\x00':
        end += 2
    return d[off:end].decode('utf-16-le'), end + 2


def main():
    src, out = sys.argv[1], sys.argv[2]
    d = open(src, 'rb').read()
    off = overlay_offset(d)
    count = struct.unpack_from('<I', d, off)[0]
    off += 4
    os.makedirs(out, exist_ok=True)
    for _ in range(count):
        name, off = read_wstr(d, off)
        rel, off = read_wstr(d, off)
        ver, off = read_wstr(d, off)
        size_s, off = read_wstr(d, off)
        size = int(size_s)
        rel = rel.replace('\\', '/')
        if rel.startswith('/') or '..' in rel.split('/'):
            raise SystemExit(f'unsafe path {rel!r}')
        dst = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        with open(dst, 'xb') as f:
            f.write(d[off:off + size])
        print(f'{off:#010x} {size:>10} {ver:>12} {rel}')
        off += size
    print('end', hex(off), 'file size', hex(len(d)))


if __name__ == '__main__':
    main()
