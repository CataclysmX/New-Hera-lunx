#!/usr/bin/env python3
"""Reboot a Hercules Stream 100 into the ST ROM DFU bootloader (0483:df11) – nothing is written.

Sends vendor request 0xF2 wValue=0 wIndex=0xE053 (firmware FUN_08008d00): the firmware stores a
RAM magic and resets; early boot (FUN_08006254) then jumps to the ROM bootloader.
Unplug/replug the device to go back to the normal firmware (the RAM magic is lost on power-off).

Usage: python3 -I tools/hsm01_dfu_enter.py --yes
"""
import sys
import time

import usb.core

VID, PID = 0x06f8, 0xe053
DFU_VID, DFU_PID = 0x0483, 0xdf11


def wait_dfu(timeout=6.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if usb.core.find(idVendor=DFU_VID, idProduct=DFU_PID) is not None:
            return True
        time.sleep(0.2)
    return False


def main():
    if '--yes' not in sys.argv:
        sys.exit(__doc__)
    if usb.core.find(idVendor=DFU_VID, idProduct=DFU_PID) is not None:
        print('already in DFU mode')
        return
    # Same request as the Windows driver (HercStreamMixDevice.sys `RebootToDfuMode`):
    # URB_FUNCTION_VENDOR_DEVICE, bRequest 0xF2, wValue 0, wIndex = PID (0xE053), no data.
    for bm, length in ((0x40, 0), (0xC0, 2)):
        dev = usb.core.find(idVendor=VID, idProduct=PID)
        if dev is None:
            sys.exit('Stream 100 not found')
        try:
            if bm & 0x80:
                dev.ctrl_transfer(bm, 0xF2, 0, PID, length, timeout=1000)
            else:
                dev.ctrl_transfer(bm, 0xF2, 0, PID, None, timeout=1000)
            print(f'request sent (bmRequestType {bm:#04x})')
        except usb.core.USBError as e:
            print(f'request {bm:#04x}: {e} (a reset/disconnect here is expected)')
        usb.util.dispose_resources(dev)
        if wait_dfu():
            print('device is now in ST DFU mode (0483:df11)')
            return
        print('still not in DFU mode, trying the other direction')
    sys.exit('device did not enter DFU mode')


if __name__ == '__main__':
    import usb.util  # noqa: E402
    main()
