#!/usr/bin/env python3
"""Hercules Stream 100 (HSM01) wire-format helpers, ported from hsm_api_core_x64.dll.

- CRC16 table: FUN_180016490, CRC loop: FUN_180016570 (see docs/protocol/framing.md)
- OUT frame (ISO EP 0x01): 952-byte "HERCULES" packet + SM message padded to 952*n
- IN report (INT EP 0x81, 64 B): [n][controls n-1 bytes][sm message]

CLI:  python3 -I hsm01_proto.py check <capture.tsv>   # verify CRC/checksum on captures
"""
import struct
import sys

ISO = 952


def _make_table():
    tbl = []
    for i in range(256):
        b = i & 0xff
        lo = i & 1
        # first step (unrolled in the DLL)
        if (((-lo) & 5) ^ (b >> 1)) & 1 == 0:
            v = (-lo) & 0x4002
        else:
            v = ((-lo) & 0x4002) ^ 0x8005
        v &= 0xffff
        for k in range(2, 8):
            nv = v >> 1
            if ((b >> k) ^ v) & 1:
                nv ^= 0x8005
            v = nv
        tbl.append(v & 0xffff)
    return tbl


TABLE = _make_table()


def crc16(data, crc=0):
    for x in data:
        crc = (crc >> 8) ^ TABLE[(crc ^ x) & 0xff]
    return crc & 0xffff


def sm_crc(msg):
    """CRC over SM message bytes [0:4] + [6:len] (bytes 4-5 hold the CRC)."""
    n = struct.unpack_from('<H', msg, 2)[0]
    return crc16(msg[6:n], crc16(msg[0:4]))


def build_frame(seq, commands):
    """Return the full ISO OUT buffer for an SM message containing `commands` bytes."""
    body = bytearray(b'SM\0\0\0\0' + struct.pack('<H', seq & 0xffff) + commands + b'\x11\x00')
    struct.pack_into('<H', body, 2, len(body))
    struct.pack_into('<H', body, 4, sm_crc(body))
    head = b'HERCULES'.ljust(ISO, b'\0')
    pad = (-len(body)) % ISO
    return head + bytes(body) + b'\0' * pad


def host_info(os_ver=(10, 0, 26100, 712), sdk=(0x53, 0x01)):
    """Command 0x10 (24 bytes) sent in every keep-alive frame (FUN_180015820)."""
    return bytes([0x10, 0x01]) + struct.pack('<4H', *os_ver) + bytes(sdk) + bytes(12)


def in_checksum_ok(sm):
    n = sm[2]
    return sum(sm[:n]) & 0xff == 0


# opcode -> length (firmware dispatcher FUN_080099d0, cross-checked with DLL encoders)
CMD_LEN = {
    0x10: lambda b, i: 24,
    0x11: lambda b, i: 2,
    0x12: lambda b, i: 5,
    0x13: lambda b, i: 5,
    0x14: lambda b, i: 1,
    0x15: lambda b, i: 1,
    0x16: lambda b, i: 1,
    0x17: lambda b, i: 1,
    0x20: lambda b, i: 2,
    0x21: lambda b, i: 4 + b[i + 1],
    0x22: lambda b, i: 3,
    0x30: lambda b, i: 3,
    0x31: lambda b, i: 3 + (b[i + 1] & 1),
    0x32: lambda b, i: 26 if b[i + 1] == 0 else 18,
    0x33: lambda b, i: 7 + 2 * struct.unpack_from('<H', b, i + 4)[0],
    0x34: lambda b, i: 9 + 2 * b[i + 8],
    0x35: lambda b, i: 5 + struct.unpack_from('<H', b, i + 3)[0],
    0x36: lambda b, i: 5 + struct.unpack_from('<H', b, i + 3)[0],
    0x37: lambda b, i: 9 + 4080,
    0x38: lambda b, i: 16,
    0x39: lambda b, i: 3 + (b[i + 1] & 1),
    0x40: lambda b, i: 10 if b[i + 1] & 0x40 else 6,
    0x41: lambda b, i: 6,
    0x50: lambda b, i: 2,
}


def decode_sm(m):
    """Split an OUT SM message into (opcode, bytes) list. Raises on unknown opcode."""
    n = struct.unpack_from('<H', m, 2)[0]
    i, out = 8, []
    while i < n:
        op = m[i]
        if op == 0x00:
            out.append((0, m[i:i + 1]))
            i += 1
            break
        if op not in CMD_LEN:
            raise ValueError(f'unknown opcode {op:#x} at {i}: {m[i:i + 12].hex()}')
        ln = CMD_LEN[op](m, i)
        out.append((op, m[i:i + ln]))
        i += ln
    return out, i, n


def iter_out_frames(path):
    for line in open(path):
        c = line.rstrip('\n').split('\t')
        if len(c) < 7 or c[4] != 'ISOCH_TRANSFER' or c[2] != 'host':
            continue
        data = bytes.fromhex(c[6])
        if data[:8] == b'HERCULES' and data[ISO:ISO + 2] == b'SM':
            yield c[0], c[1], data[ISO:]


def cmd_decode(path, verbose):
    from collections import Counter
    cnt, errs, tail = Counter(), 0, Counter()
    for frame, t, m in iter_out_frames(path):
        try:
            cmds, end, n = decode_sm(m)
        except ValueError as e:
            errs += 1
            if errs < 6:
                print('frame', frame, e)
            continue
        tail[n - end] += 1
        for op, b in cmds:
            cnt[op] += 1
        if verbose:
            print(frame, t, ' '.join(f'{op:02x}:{b[1:24].hex()}' + ('…' if len(b) > 24 else '')
                                      for op, b in cmds))
    print('opcodes:', ', '.join(f'{k:#04x}×{v}' for k, v in sorted(cnt.items())))
    print('bytes after END (should be 0):', dict(tail), '| decode errors:', errs)


def main():
    if len(sys.argv) >= 3 and sys.argv[1] in ('decode', 'decode-v'):
        cmd_decode(sys.argv[2], sys.argv[1] == 'decode-v')
        return
    if len(sys.argv) < 3 or sys.argv[1] != 'check':
        print(__doc__)
        return
    ok = bad = iok = ibad = 0
    for line in open(sys.argv[2]):
        c = line.rstrip('\n').split('\t')
        if len(c) < 7 or not c[6]:
            continue
        data = bytes.fromhex(c[6])
        if c[4] == 'ISOCH_TRANSFER' and c[2] == 'host' and data[:8] == b'HERCULES':
            m = data[ISO:]
            if m[:2] != b'SM':
                continue
            if sm_crc(m) == struct.unpack_from('<H', m, 4)[0]:
                ok += 1
            else:
                bad += 1
                if bad < 4:
                    print('CRC mismatch frame', c[0], m[:16].hex())
        elif c[4] == 'BULK_OR_INTERRUPT_TRANSFER' and c[3] == 'host' and len(data) > 8:
            n = data[0]
            sm = data[n:]
            if sm[:2] == b'sm':
                if in_checksum_ok(sm):
                    iok += 1
                else:
                    ibad += 1
    print(f'OUT SM crc ok={ok} bad={bad} | IN sm checksum ok={iok} bad={ibad}')


if __name__ == '__main__':
    main()
