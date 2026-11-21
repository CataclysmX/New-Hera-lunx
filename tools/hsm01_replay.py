#!/usr/bin/env python3
"""Re-frame and replay the *commands* of OUT frames from a capture TSV onto a live Stream 100.

Each captured SM message is decoded into commands; flash/factory opcodes are refused; the
commands are re-framed with our own sequence number + CRC (tools/hsm01_live.py Frame).
Usage: python3 -I tools/hsm01_replay.py <capture.tsv> [--speed=1.0] [--hold=10]
"""
import os
import struct
import sys
import time

import usb.core

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), 're'))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hsm01_live as live  # noqa: E402
import hsm01_proto as proto  # noqa: E402

FORBIDDEN = {0x20, 0x21, 0x22, 0x33, 0x37, 0x50}


def main():
    path = sys.argv[1]
    speed = float(next((a.split('=')[1] for a in sys.argv if a.startswith('--speed=')), 1.0))
    hold = float(next((a.split('=')[1] for a in sys.argv if a.startswith('--hold=')), 10))
    frames = []
    for frame, t, m in proto.iter_out_frames(path):
        cmds, _, _ = proto.decode_sm(m)
        ops = {op for op, _ in cmds}
        if ops & FORBIDDEN:
            sys.exit(f'frame {frame} contains forbidden opcodes {ops & FORBIDDEN}')
        frames.append((float(t), b''.join(b for _, b in cmds)))
    print(f'{len(frames)} frames, {frames[-1][0] - frames[0][0]:.1f} s')
    dev = live.open_dev()
    t0, c0 = time.monotonic(), frames[0][0]
    errs = set()
    for t, cmds in frames:
        while time.monotonic() - t0 < (t - c0) / speed:
            try:
                d = bytes(dev.read(0x81, 64, timeout=5))
                sm = d[d[0]:]
                if sm[:2] == b'sm' and sm[6] == 2:
                    errs.add((struct.unpack_from('<H', sm, 7)[0], sm[28]))
                elif sm[:2] == b'sm' and sm[6] == 1:
                    dev.write(0x01, live.Frame.build(b'\x14\x00'), timeout=500)
            except usb.core.USBTimeoutError:
                pass
        dev.write(0x01, live.Frame.build(cmds), timeout=500)
    print('replay done, (err, lastInv) seen:', errs, f'- holding {hold}s with keep-alive')
    end = time.monotonic() + hold
    while time.monotonic() < end:
        dev.write(0x01, live.Frame.build(live.KEEPALIVE), timeout=500)
        time.sleep(0.5)


if __name__ == '__main__':
    main()
