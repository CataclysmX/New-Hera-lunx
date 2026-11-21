#!/usr/bin/env python3
"""USB port reset of the Stream 100 (custom firmware): like unplugging / replugging, nothing is
written. Used when the input reports stopped (stuck interrupt IN endpoint).
The companion app must be stopped. Usage: python3 -I tools/fw_usb_reset.py
"""
import sys
import time

import usb.core


def main():
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    dev.reset()
    time.sleep(1.5)
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    print('back' if dev is not None else 'not back yet (replug if needed)')


if __name__ == '__main__':
    main()
