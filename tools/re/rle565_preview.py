#!/usr/bin/env python3
"""Render an RGB565 RLE stream (runs [count u8][color u16 LE], as the custom firmware's boot logo
or BLIT_RLE payload) to a PNG, to check it before flashing.
Usage: python3 -I tools/re/rle565_preview.py <in.rle> <out.png> [width=480] [height=272]
"""
import struct
import sys
import zlib


def main():
    src, dst = sys.argv[1], sys.argv[2]
    w = int(sys.argv[3]) if len(sys.argv) > 3 else 480
    h = int(sys.argv[4]) if len(sys.argv) > 4 else 272
    data = open(src, 'rb').read()
    px = []
    for i in range(0, len(data) - 2, 3):
        c = struct.unpack_from('<H', data, i + 1)[0]
        rgb = bytes((((c >> 11) & 0x1f) * 255 // 31, ((c >> 5) & 0x3f) * 255 // 63, (c & 0x1f) * 255 // 31))
        px.extend([rgb] * data[i])
    print(f'{len(data)} bytes, {len(data) // 3} runs, {len(px)} pixels (expected {w * h})')
    px = (px + [b'\0\0\0'] * (w * h))[:w * h]
    raw = b''.join(b'\0' + b''.join(px[y * w:(y + 1) * w]) for y in range(h))

    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))

    png = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
           + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))
    open(dst, 'wb').write(png)


if __name__ == '__main__':
    main()
