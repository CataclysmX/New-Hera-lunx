#!/usr/bin/env python3
"""Minimal InstallShield cabinet (v6+) extractor (subset of unshield's logic).

Usage: python3 -I iscab_unpack.py <dir_with_data1.hdr> <empty_out_dir> [--list]
"""
import os
import struct
import sys
import zlib

SIG = 0x28635349
FILE_SPLIT, FILE_OBFUSCATED, FILE_COMPRESSED, FILE_INVALID = 1, 2, 4, 8


def u16(b, o): return struct.unpack_from('<H', b, o)[0]
def u32(b, o): return struct.unpack_from('<I', b, o)[0]
def u64(b, o): return struct.unpack_from('<Q', b, o)[0]


WIDE = False  # IS >= 2010 stores UTF-16LE names


def cstr(b, o):
    if WIDE:
        e = o
        while b[e:e + 2] != b'\0\0':
            e += 2
        return b[o:e].decode('utf-16-le', 'replace')
    e = b.index(b'\0', o)
    return b[o:e].decode('utf-8', 'replace')


class Volume:
    def __init__(self, path):
        self.f = open(path, 'rb')
        h = self.f.read(0x60)
        assert u32(h, 0) == SIG, path
        p = 20
        self.data_offset = u32(h, p)
        self.first_file_index = u32(h, p + 8)
        self.last_file_index = u32(h, p + 12)
        self.first_file_offset = u64(h, p + 16)
        self.first_file_size_expanded = u64(h, p + 24)
        self.first_file_size_compressed = u64(h, p + 32)
        self.last_file_offset = u64(h, p + 40)
        self.last_file_size_expanded = u64(h, p + 48)
        self.last_file_size_compressed = u64(h, p + 56)


def deobfuscate(buf, seed):
    out = bytearray(len(buf))
    for i, x in enumerate(buf):
        x ^= 0xd5
        x = ((x >> 2) | (x << 6)) & 0xff
        out[i] = (x - ((seed + i) % 0x47)) & 0xff
    return bytes(out), seed + len(buf)


def main():
    src, out = sys.argv[1], sys.argv[2]
    list_only = '--list' in sys.argv
    hdr = open(os.path.join(src, 'data1.hdr'), 'rb').read()
    assert u32(hdr, 0) == SIG
    version = u32(hdr, 4)
    global WIDE
    major = (version & 0xffff) // 100 if version >> 24 in (2, 4) else (version >> 12) & 0xf
    WIDE = major >= 17
    cdo = u32(hdr, 12)
    p = cdo + 0xc
    ft_off = u32(hdr, p)
    dir_count = u32(hdr, p + 0x10)
    file_count = u32(hdr, p + 0x1c)
    ft_off2 = u32(hdr, p + 0x20)
    base = cdo + ft_off
    file_table = [u32(hdr, base + 4 * i) for i in range(dir_count + file_count)]
    dirs = [cstr(hdr, base + file_table[i]) for i in range(dir_count)]
    print(f'version={version:#x} dirs={dir_count} files={file_count}', file=sys.stderr)

    vols = {}

    def vol(n):
        if n not in vols:
            vols[n] = Volume(os.path.join(src, f'data{n}.cab'))
        return vols[n]

    for idx in range(file_count):
        q = base + ft_off2 + idx * 0x57
        flags = u16(hdr, q)
        exp = u64(hdr, q + 2)
        comp = u64(hdr, q + 10)
        doff = u64(hdr, q + 18)
        name_off = u32(hdr, q + 0x3a)
        dir_idx = u16(hdr, q + 0x3e)
        volume = u16(hdr, q + 0x55)
        if flags & FILE_INVALID or name_off == 0:
            continue
        name = cstr(hdr, base + name_off)
        d = dirs[dir_idx] if dir_idx < len(dirs) else '?'
        rel = (d.replace('\\', '/').strip('/') + '/' + name).lstrip('/')
        print(f'{idx:4} fl={flags:#x} vol={volume} {exp:>10} {rel}')
        if list_only:
            continue
        if rel.startswith('/') or '..' in rel.split('/'):
            raise SystemExit(f'unsafe path {rel!r}')

        # read compressed stream, following volume splits
        chunks = []
        v = vol(volume)
        if idx == v.last_file_index and v.last_file_offset != 0x7fffffff and flags & FILE_SPLIT:
            v.f.seek(v.last_file_offset)
            part = v.last_file_size_compressed
        elif idx == v.first_file_index and flags & FILE_SPLIT and volume > 1:
            v.f.seek(v.first_file_offset)
            part = v.first_file_size_compressed
        else:
            v.f.seek(doff)
            part = comp
        remaining = comp if flags & FILE_COMPRESSED else exp
        cur = volume
        while remaining > 0:
            take = min(part, remaining)
            chunks.append(v.f.read(take))
            remaining -= take
            if remaining > 0:
                cur += 1
                v = vol(cur)
                v.f.seek(v.first_file_offset)
                part = v.first_file_size_compressed if flags & FILE_COMPRESSED else v.first_file_size_expanded
        raw = b''.join(chunks)
        if flags & FILE_OBFUSCATED:
            raw, _ = deobfuscate(raw, 0)
        if flags & FILE_COMPRESSED:
            data, o = bytearray(), 0
            while o < len(raw):
                n = u16(raw, o)
                o += 2
                data += zlib.decompressobj(-15).decompress(raw[o:o + n])
                o += n
            raw = bytes(data)
        if len(raw) != exp:
            print(f'   !! size mismatch {len(raw)} != {exp}', file=sys.stderr)
        dst = os.path.join(out, rel)
        os.makedirs(os.path.dirname(dst) or out, exist_ok=True)
        if os.path.exists(dst):
            dst += f'.dup{idx}'
        with open(dst, 'xb') as f:
            f.write(raw)


if __name__ == '__main__':
    main()
