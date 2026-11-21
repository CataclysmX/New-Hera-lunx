#!/usr/bin/env python3
"""GIF of the custom firmware's screen saver (« SIGNAL NOT FOUND », firmware 240.22 and later),
replayed on the computer exactly as the firmware draws it (firmware/rust/src/main.rs `Saver`,
src/gfx.rs `saver_draw`): the badge sprite of the build (`$OUT_DIR/saver.bin`: [w u16][h u16]
then rows of 4-bit alpha), starting at (max_x / 2, max_y / 3), one pixel a frame on each axis at
30 frames a second, a new colour at each wall, tinted by its alpha onto black. Shown at full
brightness (on the device the backlight is at 1 %). Each GIF frame holds 2 firmware frames.

Usage: python3 -I tools/saver_gif.py SAVER.BIN OUT.gif [seconds]
(SAVER.BIN: firmware/rust/target/thumbv7em-none-eabihf/release/build/hsm01-fw-*/out/saver.bin)
"""
import struct
import sys

from PIL import Image

W, H, FW_FPS, STEP = 480, 272, 30, 2
COLOURS = [(0, 224, 200), (255, 64, 160), (255, 208, 0), (64, 160, 255), (176, 96, 255),
           (255, 128, 32), (96, 255, 96)]


def rgb565(r, g, b):
    return (r & 0xf8) << 8 | (g & 0xfc) << 3 | b >> 3


def to_rgb888(c):
    r, g, b = c >> 11 & 31, c >> 5 & 63, c & 31
    return (r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2)


def ramp(colour):
    """The 16 shades of `colour` as the firmware computes them (RGB565 channels × alpha / 15)."""
    c = rgb565(*colour)
    r, g, b = c >> 11 & 31, c >> 5 & 63, c & 31
    return [to_rgb888((r * k // 15) << 11 | (g * k // 15) << 5 | (b * k // 15)) for k in range(16)]


def main():
    data = open(sys.argv[1], 'rb').read()
    out, secs = sys.argv[2], float(sys.argv[3]) if len(sys.argv) > 3 else 16.0
    w, h = struct.unpack_from('<HH', data, 0)
    row_bytes = (w + 1) // 2
    alpha = [[(data[4 + y * row_bytes + x // 2] >> (0 if x % 2 else 4)) & 15 for x in range(w)]
             for y in range(h)]
    max_x, max_y = max(W - w, 0), max(H - h, 0)
    x, y, dx, dy, colour = max_x // 2, max_y // 3, 1, 1, 0
    frames = []
    for n in range(int(secs * FW_FPS)):
        if n % STEP == 0:
            img = Image.new('RGB', (W, H))
            shades = ramp(COLOURS[colour])
            px = img.load()
            for yy in range(min(h, H - y)):
                for xx in range(min(w, W - x)):
                    if alpha[yy][xx]:
                        px[x + xx, y + yy] = shades[alpha[yy][xx]]
            frames.append(img)
        nx, ny = x + dx, y + dy  # Saver::step
        hit_x, hit_y = not 0 <= nx <= max_x, not 0 <= ny <= max_y
        if hit_x:
            dx = -dx
        if hit_y:
            dy = -dy
        if hit_x or hit_y:
            colour = (colour + 1) % len(COLOURS)
        x, y = min(max(nx, 0), max_x), min(max(ny, 0), max_y)
    frames[0].save(out, save_all=True, append_images=frames[1:], loop=0, optimize=True,
                   duration=round(1000 * STEP / FW_FPS / 10) * 10)
    print(f'{out}: {len(frames)} images, badge {w}x{h}')


if __name__ == '__main__':
    main()
