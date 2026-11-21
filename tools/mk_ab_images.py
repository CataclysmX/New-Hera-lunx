#!/usr/bin/env python3
"""Images for the panel image-retention A/B test (docs/firmware/stage2.md):
checker.png = black/white checkerboard, 80x68 cells (edges away from the fw_geometry.py grid),
grey.png = uniform 50 % grey, both 480x272.
Usage: python3 -I tools/mk_ab_images.py [out_dir=re_work/ab]
"""
import os
import struct
import sys
import zlib

W, H = 480, 272


def png(path, pixel):
    raw = b''.join(b'\0' + b''.join(pixel(x, y) for x in range(W)) for y in range(H))

    def chunk(kind, body):
        return struct.pack('>I', len(body)) + kind + body + struct.pack('>I', zlib.crc32(kind + body))

    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0))
                + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join('re_work', 'ab')
    os.makedirs(out, exist_ok=True)
    png(os.path.join(out, 'checker.png'),
        lambda x, y: b'\xff\xff\xff' if (x // 80 + y // 68) % 2 else b'\0\0\0')
    png(os.path.join(out, 'grey.png'), lambda x, y: b'\x80\x80\x80')
    print(f'{out}/checker.png, {out}/grey.png')


if __name__ == '__main__':
    main()
