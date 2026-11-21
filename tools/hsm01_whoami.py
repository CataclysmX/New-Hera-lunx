#!/usr/bin/env python3
"""Tell which firmware a Stream 100 runs: USB strings, bcdDevice and vendor request 0x02.
Official firmware answers e.g. 1.40; the custom firmware answers 240.xx (major 0xF0).
Usage: python3 -I tools/hsm01_whoami.py
"""
import sys

import usb.core
import usb.util

dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
if dev is None:
    dfu = usb.core.find(idVendor=0x0483, idProduct=0xdf11)
    sys.exit('device in ST DFU mode (0483:df11)' if dfu else 'Stream 100 not found')
print(f'bcdDevice {dev.bcdDevice:#06x}  manufacturer {usb.util.get_string(dev, dev.iManufacturer)!r}'
      f'  product {usb.util.get_string(dev, dev.iProduct)!r}')
v = bytes(dev.ctrl_transfer(0xC0, 0x02, 0, 0, 2, 1000))
kind = 'custom firmware (hsm01-fw)' if v[1] == 0xF0 else 'official firmware'
print(f'version {v[1]}.{v[0]:02d}  -> {kind}')
