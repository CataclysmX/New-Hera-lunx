#!/usr/bin/env python3
"""Dump clock / LTDC / GPIO registers of the custom firmware (vendor request 0x03, 240.12+) and
derive the pixel clock and frame rate (assumes MSI is the PLL source, range from RCC_CR).
Usage: python3 -I tools/fw_regs.py
"""
import struct
import sys

import usb.core

NAMES = ['RCC_CR', 'RCC_CFGR', 'RCC_PLLCFGR', 'RCC_PLLSAI1CFGR', 'RCC_PLLSAI2CFGR', 'RCC_CCIPR',
         'RCC_CCIPR2', 'LTDC_SSCR', 'LTDC_BPCR', 'LTDC_AWCR', 'LTDC_TWCR', 'LTDC_GCR', 'LTDC_BCCR',
         'GPIOB_OSPEEDR', 'GPIOD_OSPEEDR', 'GPIOE_OSPEEDR',
         'LTDC_IER', 'LTDC_ISR', 'LTDC_CPSR', 'LTDC_CDSR', 'L1_CR', 'L1_WHPCR', 'L1_WVPCR',
         'L1_PFCR', 'L1_CACR', 'L1_DCCR', 'L1_BFCR', 'L1_CFBAR', 'L1_CFBLR', 'L1_CFBLNR',
         'TIM2_ARR', 'TIM2_CCR1']
MSI = [100e3, 200e3, 400e3, 800e3, 1e6, 2e6, 4e6, 8e6, 16e6, 24e6, 32e6, 48e6]


def bits(v, lo, n):
    return (v >> lo) & ((1 << n) - 1)


def main():
    dev = usb.core.find(idVendor=0x06f8, idProduct=0xe053)
    if dev is None:
        sys.exit('Stream 100 not found')
    r = {}
    for page in (0, 1):
        data = bytes(dev.ctrl_transfer(0xC0, 0x03, page, 0, 64, 1000))
        for i in range(len(data) // 4):
            r[NAMES[page * 16 + i]] = struct.unpack_from('<I', data, 4 * i)[0]
    for k, v in r.items():
        print(f'{k:<16} 0x{v:08x}')

    cr = r['RCC_CR']
    msi = MSI[bits(cr, 4, 4)] if bits(cr, 3, 1) else MSI[6]  # MSIRGSEL ? MSIRANGE : (reset) 4 MHz
    pll = r['RCC_PLLCFGR']
    m_main = bits(pll, 4, 4) + 1
    sai2 = r['RCC_PLLSAI2CFGR']
    m2 = bits(sai2, 4, 4) + 1                      # L4+: PLLSAI2M
    n2 = bits(sai2, 8, 7)
    r2 = (bits(sai2, 25, 2) + 1) * 2
    div = [2, 4, 8, 16][bits(r['RCC_CCIPR2'], 16, 2)]
    vco = msi / m2 * n2
    pclk = vco / r2 / div
    totw = bits(r['LTDC_TWCR'], 16, 12) + 1
    toth = bits(r['LTDC_TWCR'], 0, 11) + 1
    print(f'\nMSI {msi / 1e6:g} MHz (main PLL M={m_main}), PLLSAI2 M={m2} N={n2} R=/{r2} '
          f'VCO {vco / 1e6:g} MHz, LTDC /{div}')
    print(f'pixel clock {pclk / 1e6:.3f} MHz, total {totw}x{toth} -> {pclk / totw / toth:.1f} Hz')
    sp = r['GPIOE_OSPEEDR']
    print(f'GPIOE speed pin0 = {["LOW", "MEDIUM", "HIGH", "VERY_HIGH"][sp & 3]}; '
          f'GCR 0x{r["LTDC_GCR"]:08x} (HSPOL {bits(r["LTDC_GCR"], 31, 1)} VSPOL '
          f'{bits(r["LTDC_GCR"], 30, 1)} DEPOL {bits(r["LTDC_GCR"], 29, 1)} PCPOL '
          f'{bits(r["LTDC_GCR"], 28, 1)} DEN {bits(r["LTDC_GCR"], 16, 1)})')


if __name__ == '__main__':
    main()
