#!/usr/bin/env python3
"""Render frames of the firmware boot stroke animation (build.rs bootanim.bin + bootlogo.rle) to
PNG, replaying what gfx does: strokes traced in DEVUS green with a brighter pen tip, then light-up
(strokes green → real colour, halo black → real colour).
Usage: python3 -I tools/re/anim_preview.py <out_dir> <frame> [frame...]   ('glowK' = light-up K/45)
Reads the newest target/thumbv7em-none-eabihf/release/build/hsm01-fw-*/out/.
"""
import glob
import os
import struct
import sys
import zlib

W, H, STROKE_LUM, GLOW_STEPS = 480, 272, 190, 45
TRACE, PEN_TIP = 0x036D, 0x03EF  # keep in sync with firmware gfx.rs
OUT = os.path.dirname(max(glob.glob(
    'firmware/rust/target/thumbv7em-none-eabihf/release/build/hsm01-fw-*/out/bootanim.bin'), key=os.path.getmtime))


def split(c):
    return (c >> 11) & 31, (c >> 5) & 63, c & 31


def rgb(c):
    r, g, b = split(c)
    return bytes((r * 255 // 31, g * 255 // 63, b * 255 // 31))


def lum(c):
    r, g, b = split(c)
    return max(r * 255 // 31, g * 255 // 63, b * 255 // 31)


def lerp(a, b, k):
    mix = [int(x + (y - x) * k / GLOW_STEPS) for x, y in zip(split(a), split(b))]
    return mix[0] << 11 | mix[1] << 5 | mix[2]


def frames(anim):
    n, pos, out = struct.unpack_from('<H', anim, 0)[0], 2, []
    for _ in range(n):
        runs, pos = struct.unpack_from('<H', anim, pos)[0], pos + 2
        fr = []
        for _ in range(runs):
            fr.append((anim[pos] | anim[pos + 1] << 8 | anim[pos + 2] << 16, anim[pos + 3]))
            pos += 4
        out.append(fr)
    return out


def png(path, fb):
    raw = b''.join(b'\0' + b''.join(rgb(c) for c in fb[y * W:(y + 1) * W]) for y in range(H))

    def chunk(k, b):
        return struct.pack('>I', len(b)) + k + b + struct.pack('>I', zlib.crc32(k + b))

    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0))
                           + chunk(b'IDAT', zlib.compress(raw, 6)) + chunk(b'IEND', b''))


def main():
    out_dir, wanted = sys.argv[1], sys.argv[2:]
    os.makedirs(out_dir, exist_ok=True)
    fr = frames(open(os.path.join(OUT, 'bootanim.bin'), 'rb').read())
    data = open(os.path.join(OUT, 'bootlogo.rle'), 'rb').read()
    logo = []
    for i in range(0, len(data) - 2, 3):
        logo.extend([struct.unpack_from('<H', data, i + 1)[0]] * data[i])
    print(f'{len(fr)} stroke frames + {GLOW_STEPS} light-up frames = {(len(fr) + GLOW_STEPS) / 60:.2f} s at 60 fps')
    for want in wanted:
        fb = [0] * (W * H)
        k, upto = (int(want[4:]), len(fr)) if want.startswith('glow') else (0, int(want) + 1)
        for i, f in enumerate(fr[:upto]):
            col = PEN_TIP if i == upto - 1 and k == 0 else TRACE
            for off, ln in f:
                fb[off:off + ln] = [col] * ln
        if k:
            fb = [lerp(TRACE if lum(c) >= STROKE_LUM else 0, c, k) for c in logo]
        png(os.path.join(out_dir, f'frame_{want}.png'), fb)


if __name__ == '__main__':
    main()
