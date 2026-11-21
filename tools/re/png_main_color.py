#!/usr/bin/env python3
"""Most frequent non-white/non-transparent colour of a PNG (8-bit RGB/RGBA, non-interlaced),
stdlib only. Usage: python3 -I tools/re/png_main_color.py <file.png>
"""
import collections
import struct
import sys
import zlib


def pixels(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    pos, idat, w = 8, b'', 0
    while pos < len(data):
        ln, kind = struct.unpack_from('>I4s', data, pos)
        body = data[pos + 8:pos + 8 + ln]
        if kind == b'IHDR':
            w, h, depth, ctype, _, _, inter = struct.unpack('>IIBBBBB', body)
            assert depth == 8 and ctype in (2, 6) and inter == 0, (depth, ctype, inter)
            bpp = 3 if ctype == 2 else 4
        elif kind == b'IDAT':
            idat += body
        pos += 12 + ln
    raw, stride, prev, out = zlib.decompress(idat), w * bpp, bytearray(w * bpp), []
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], prev[i - bpp] if i >= bpp else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        out.extend(tuple(line[i:i + bpp]) for i in range(0, stride, bpp))
        prev = line
    return out


def main():
    counts = collections.Counter(p[:3] for p in pixels(sys.argv[1])
                                 if (len(p) == 3 or p[3] > 200) and min(p[:3]) < 200)
    for (r, g, b), n in counts.most_common(3):
        c565 = ((r & 0xf8) << 8) | ((g & 0xfc) << 3) | (b >> 3)
        print(f'#{r:02x}{g:02x}{b:02x}  rgb({r},{g},{b})  RGB565 0x{c565:04x}  ({n} px)')


if __name__ == '__main__':
    main()
