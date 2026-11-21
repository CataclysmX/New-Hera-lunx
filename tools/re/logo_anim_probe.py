#!/usr/bin/env python3
"""Probe the boot logo (firmware RLE, RGB565) for the stroke animation: brightness histogram,
stroke ('core') masks at a few thresholds, connected components; writes core mask PNGs.
Usage: python3 -I tools/re/logo_anim_probe.py <bootlogo.rle> <out_dir>
"""
import os
import struct
import sys
import zlib

W, H = 480, 272


def load(path):
    data = open(path, 'rb').read()
    px = []
    for i in range(0, len(data) - 2, 3):
        px.extend([struct.unpack_from('<H', data, i + 1)[0]] * data[i])
    return px[:W * H]


def lum(c):
    r, g, b = ((c >> 11) & 31) * 255 // 31, ((c >> 5) & 63) * 255 // 63, (c & 31) * 255 // 31
    return max(r, g, b)


def png(path, rows):
    raw = b''.join(b'\0' + bytes(r) for r in rows)

    def chunk(k, b):
        return struct.pack('>I', len(b)) + k + b + struct.pack('>I', zlib.crc32(k + b))

    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 0, 0, 0, 0))
                           + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b''))


def components(mask):
    seen, comps = [False] * (W * H), []
    for s in range(W * H):
        if mask[s] and not seen[s]:
            stack, comp = [s], []
            seen[s] = True
            while stack:
                p = stack.pop()
                comp.append(p)
                x, y = p % W, p // W
                for dx in (-1, 0, 1):
                    for dy in (-1, 0, 1):
                        nx, ny = x + dx, y + dy
                        if 0 <= nx < W and 0 <= ny < H:
                            q = ny * W + nx
                            if mask[q] and not seen[q]:
                                seen[q] = True
                                stack.append(q)
            comps.append(comp)
    return comps


def main():
    px = load(sys.argv[1])
    out = sys.argv[2]
    os.makedirs(out, exist_ok=True)
    L = [lum(c) for c in px]
    hist = [0] * 9
    for v in L:
        hist[min(v // 32, 8)] += 1
    print('brightness histogram (/32):', hist)
    for thr in (160, 190, 220):
        mask = [v >= thr for v in L]
        comps = components(mask)
        big = sorted((len(c) for c in comps), reverse=True)
        print(f'thr {thr}: {sum(mask)} px, {len(comps)} components, largest {big[:12]}')
        png(os.path.join(out, f'core_{thr}.png'), [[255 if mask[y * W + x] else 0 for x in range(W)] for y in range(H)])


if __name__ == '__main__':
    main()
