#!/usr/bin/env python3
"""Live probe for the Hercules Stream 100 (pyusb). Never sends flash/factory commands.

Usage: python3 -I tools/hsm01_live.py [seconds=10] [--content] [--leds=PATTERN]
  default   : init, keep-alive every 500 ms, print control-block changes + status changes
  --content : also send one content frame (screen on, brightness 100 %, LEDs)
  --leds=0120 : LED states for LED 0..n (0 off, 1 on, 2 blink)
Status 'err' != 0 or 'lastInv' != 0 means the firmware rejected something.
"""
import struct
import sys
import time

import usb.core
import usb.util

VID, PID = 0x06f8, 0xe053
ISO = 952
RAW = '--raw' in sys.argv
prev_raw = None


def _table():
    t = []
    for i in range(256):
        lo = i & 1
        v = ((-lo) & 0x4002) ^ (0x8005 if ((((-lo) & 5) ^ (i >> 1)) & 1) else 0)
        v &= 0xffff
        for k in range(2, 8):
            nv = v >> 1
            if ((i >> k) ^ v) & 1:
                nv ^= 0x8005
            v = nv
        t.append(v)
    return t


T = _table()


def crc16(data, crc=0):
    for x in data:
        crc = (crc >> 8) ^ T[(crc ^ x) & 0xff]
    return crc


class Frame:
    seq = 0

    @classmethod
    def build(cls, cmds):
        body = bytearray(b'SM\0\0\0\0' + struct.pack('<H', cls.seq) + cmds)
        cls.seq = (cls.seq + 1) & 0xffff
        struct.pack_into('<H', body, 2, len(body))
        struct.pack_into('<H', body, 4, crc16(body[6:], crc16(body[:4])))
        body += b'\0' * ((-len(body)) % ISO)
        return b'HERCULES'.ljust(ISO, b'\0') + bytes(body)


HOST_INFO = bytes([0x10, 0x01]) + struct.pack('<4H', 6, 1, 0, 0) + bytes([0x53, 0x01]) + bytes(12)
KEEPALIVE = HOST_INFO + b'\x11\x00'


def open_dev():
    dev = usb.core.find(idVendor=VID, idProduct=PID)
    if dev is None:
        sys.exit('Stream 100 not found')
    for i in (0, 1):
        try:
            if dev.is_kernel_driver_active(i):
                dev.detach_kernel_driver(i)
        except usb.core.USBError:
            pass
    try:
        if dev.get_active_configuration().bConfigurationValue != 1:
            dev.set_configuration(1)
    except usb.core.USBError:
        dev.set_configuration(1)
    usb.util.claim_interface(dev, 0)
    usb.util.claim_interface(dev, 1)
    dev.set_interface_altsetting(interface=1, alternate_setting=1)
    v = bytes(dev.ctrl_transfer(0xC0, 0x52, 0, 0, 2, 1000))
    sn = bytes(dev.ctrl_transfer(0xC0, 0x52, 1, 0, 9, 1000))
    st = bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))
    print(f'{dev.manufacturer} {dev.product}: req52/0={v.hex()} serial={sn.hex()} req02={st.hex()}')
    return dev


def status(sm):
    if sm[6] != 2:
        return None
    return dict(err=struct.unpack_from('<H', sm, 7)[0], ack=struct.unpack_from('<H', sm, 9)[0],
                leds=struct.unpack_from('<H', sm, 26)[0], lastInv=sm[28], render=sm[29],
                fw=sm[32])


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    secs = float(args[0]) if args else 10
    leds = next((a.split('=')[1] for a in sys.argv if a.startswith('--leds=')), None)
    dev = open_dev()
    t0 = last_ka = time.monotonic()
    prev_ctl = prev_st = seen_info = None
    content = '--content' in sys.argv or leds is not None
    while time.monotonic() - t0 < secs:
        now = time.monotonic()
        if content:
            cmds = b'\x12\0\0\0\0' + b'\x31\x01\x00\x64'
            for i, c in enumerate(leds or ''):
                cmds += bytes([0x30, i, int(c)])
            dev.write(0x01, Frame.build(cmds + b'\x00'), timeout=500)
            content = False
            print('content frame sent')
        if now - last_ka > 0.5:
            dev.write(0x01, Frame.build(KEEPALIVE), timeout=500)
            last_ka = now
        try:
            d = bytes(dev.read(0x81, 64, timeout=50))
        except usb.core.USBTimeoutError:
            continue
        n = d[0]
        ctl, sm = d[:n], d[n:]
        if ctl != prev_ctl:
            btn = struct.unpack_from('<H', ctl, 1)[0]
            enc = struct.unpack_from('<4h', ctl, 3)
            print(f'{now - t0:7.3f} ctl btn={btn:016b} enc={enc} state={ctl[11]:#04x}')
            prev_ctl = ctl
        if sm[:2] == b'sm' and sum(sm[:sm[2]]) & 0xff == 0:
            s = status(sm)
            if s:
                k = (s['err'], s['leds'], s['lastInv'], s['fw'])
                if k != prev_st or (RAW and sm[7:4] != prev_raw):
                    print(f'{now - t0:7.3f} status {s}' + (f' raw={sm[7:sm[2]].hex(" ")}' if RAW else ''))
                    prev_st = k
            elif sm[6] == 1:
                # device info: repeated until the host acks with opcode 0x14 (like the DLL)
                if sm[7:sm[2]] != seen_info:
                    print(f'{now - t0:7.3f} device-info {sm[7:sm[2]].hex()}')
                    seen_info = sm[7:sm[2]]
                dev.write(0x01, Frame.build(b'\x14\x00'), timeout=500)
            else:
                print(f'{now - t0:7.3f} sm type={sm[6]} {sm[7:sm[2]].hex()}')
    usb.util.release_interface(dev, 1)
    usb.util.release_interface(dev, 0)


if __name__ == '__main__':
    main()
