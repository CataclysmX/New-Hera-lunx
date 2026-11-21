#!/usr/bin/env python3
"""Pre-flash check of a custom firmware ELF (replaces reading the disassembly by hand):
  1. the DFU escape hatch (`escape::check_at_boot`, inlined in `__cortex_m_rt_main`) runs first:
     its constants (RAM magic 0xdf00b007 = MOVW 0xb007 + MOVT 0xdf00, delay MOVW 0x9c41) are loaded
     before the first call to `__aeabi_memclr` / `embassy_stm32::init`;
  2. the flash image (PT_LOAD segments) stays out of the last flash page 0x081FF000 (device data).
Usage: python3 -I tools/re/fw_hatch_check.py firmware/rust/target/thumbv7em-none-eabihf/release/hsm01-fw
"""
import struct
import sys

WINDOW = 1024  # bytes of main searched
FLASH, LAST_PAGE = 0x08000000, 0x081FF000


def sections(elf):
    shoff, = struct.unpack_from('<I', elf, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from('<HHH', elf, 0x2E)
    hdrs = [struct.unpack_from('<IIIIIIIIII', elf, shoff + i * shentsize) for i in range(shnum)]
    names = hdrs[shstrndx]
    name = lambda off: elf[names[4] + off:elf.index(b'\0', names[4] + off)].decode()
    return {name(h[0]): h for h in hdrs}


def symbol(elf, secs, wanted, required=True):
    """(address, size) of the function whose (possibly mangled) name contains `wanted`."""
    symtab, strtab = secs['.symtab'], secs['.strtab']
    for off in range(symtab[4], symtab[4] + symtab[5], 16):
        st_name, value, size, _, _, _ = struct.unpack_from('<IIIBBH', elf, off)
        start = strtab[4] + st_name
        name = elf[start:elf.index(b'\0', start)].decode(errors='replace')
        # exact, v0-mangled (`…18___cortex_m_rt_main`) or legacy-mangled (`…__cortex_m_rt_main17h…E`)
        if size > 0 and (name == wanted or name.endswith(wanted) or wanted + '17h' in name):
            return value & ~1, size
    if required:
        sys.exit(f'symbol {wanted} not found')
    return None


def read(elf, secs, addr, n):
    for h in secs.values():
        sh_type, sh_addr, sh_offset, sh_size = h[1], h[3], h[4], h[5]
        if sh_type == 1 and sh_addr <= addr < sh_addr + sh_size:  # PROGBITS
            return elf[sh_offset + addr - sh_addr:sh_offset + min(addr + n, sh_addr + sh_size) - sh_addr]
    sys.exit(f'address {addr:#x} not in the image')


def bl_targets(code, base):
    """Targets of the Thumb-2 BL / B.W instructions in `code` (loaded at `base`)."""
    out = []
    for i in range(0, len(code) - 3, 2):
        hi, lo = struct.unpack_from('<HH', code, i)
        if hi >> 11 == 0b11110 and lo >> 14 == 0b11 and (lo >> 12) & 1:
            s, imm10, j1, j2, imm11 = (hi >> 10) & 1, hi & 0x3FF, (lo >> 13) & 1, (lo >> 11) & 1, lo & 0x7FF
            i1, i2 = 1 - (j1 ^ s), 1 - (j2 ^ s)
            off = (s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1)
            if s:
                off -= 1 << 25
            out.append((base + i, base + i + 4 + off))
    return out


def movw_movt(code, base):
    """(address, 'movw'|'movt', imm16) of the Thumb-2 MOVW / MOVT instructions in `code`."""
    out = []
    for i in range(0, len(code) - 3, 2):
        hi, lo = struct.unpack_from('<HH', code, i)
        if hi & 0xFB70 in (0xF240, 0xF2C0) and not lo & 0x8000:
            imm16 = ((hi & 0xF) << 12) | (((hi >> 10) & 1) << 11) | (((lo >> 12) & 7) << 8) | (lo & 0xFF)
            out.append((base + i, 'movt' if hi & 0x0080 else 'movw', imm16))
    return out


def flash_end(elf):
    phoff, = struct.unpack_from('<I', elf, 0x1C)
    phentsize, phnum = struct.unpack_from('<HH', elf, 0x2A)
    end = FLASH
    for i in range(phnum):
        p_type, _, _, p_paddr, p_filesz = struct.unpack_from('<IIIII', elf, phoff + i * phentsize)
        if p_type == 1 and p_filesz and FLASH <= p_paddr < LAST_PAGE + 0x1000:
            end = max(end, p_paddr + p_filesz)
    return end


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    elf = open(sys.argv[1], 'rb').read()
    secs = sections(elf)
    ok = True

    def report(good, text):
        nonlocal ok
        print(('OK  ' if good else 'KO  ') + text)
        ok &= good

    main_addr, _ = symbol(elf, secs, '__cortex_m_rt_main')
    code = read(elf, secs, main_addr, WINDOW)
    names = ('__aeabi_memclr', '__aeabi_memclr4', '__aeabi_memclr8', '13embassy_stm324init')
    stops = [a for a in (symbol(elf, secs, n, False) for n in names) if a]
    calls = [at for at, target in bl_targets(code, main_addr) if any(target == a for a, _ in stops)]
    limit = calls[0] if calls else main_addr + WINDOW
    loads = [(at, kind, v) for at, kind, v in movw_movt(code, main_addr) if at < limit]
    for name, kind, value in (('magic low 0xb007', 'movw', 0xB007), ('magic high 0xdf00', 'movt', 0xDF00),
                              ('delay 0x9c41', 'movw', 0x9C41)):
        at = next((a for a, k, v in loads if k == kind and v == value), None)
        report(at is not None, f'{name} loaded at {at:#x}' if at is not None
               else f'{name} not loaded before the first memclr/init call')
    report(bool(calls), f'first memclr/init call at {limit:#x} (main at {main_addr:#x})')

    end = flash_end(elf)
    report(end <= LAST_PAGE, f'flash image ends at {end:#x} ({(end - FLASH) // 1024} KiB), '
                             f'last page {LAST_PAGE:#x} untouched')
    sys.exit(0 if ok else 1)


if __name__ == '__main__':
    main()
