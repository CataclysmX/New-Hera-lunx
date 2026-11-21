#!/usr/bin/env python3
"""List which STM32L4+ peripheral base addresses appear as literals in a firmware image.
Usage: python3 -I fw_peripherals.py <fw_body.bin>
"""
import struct
import sys

PERIPH = {  # RM0432 memory map (STM32L4Rx/L4Sx)
    'TIM2': 0x40000000, 'TIM3': 0x40000400, 'TIM4': 0x40000800, 'TIM5': 0x40000C00,
    'TIM6': 0x40001000, 'TIM7': 0x40001400, 'SPI2': 0x40003800, 'SPI3': 0x40003C00,
    'USART2': 0x40004400, 'I2C1': 0x40005400, 'I2C2': 0x40005800, 'LPTIM1': 0x40007C00,
    'TIM1': 0x40012C00, 'SPI1': 0x40013000, 'TIM8': 0x40013400, 'USART1': 0x40013800,
    'TIM15': 0x40014000, 'TIM16': 0x40014400, 'TIM17': 0x40014800, 'LTDC': 0x40016800,
    'DSI': 0x40016C00, 'DMA1': 0x40020000, 'DMA2': 0x40020400, 'DMAMUX1': 0x40020800,
    'RCC': 0x40021000, 'FLASH_R': 0x40022000, 'CRC': 0x40023000, 'DMA2D': 0x4002B000,
    'GPIOA': 0x48000000, 'GPIOB': 0x48000400, 'GPIOC': 0x48000800, 'GPIOD': 0x48000C00,
    'GPIOE': 0x48001000, 'GPIOF': 0x48001400, 'GPIOG': 0x48001800, 'GPIOH': 0x48001C00,
    'GPIOI': 0x48002000, 'USB_OTG_FS': 0x50000000, 'ADC': 0x50040000, 'GFXMMU': 0x4002C000,
    'OCTOSPI1_R': 0xA0001000, 'OCTOSPI2_R': 0xA0001400, 'FMC_R': 0xA0000000,
    'OCTOSPI1_MEM': 0x90000000, 'OCTOSPI2_MEM': 0x70000000, 'FMC_BANK1': 0x60000000,
    'SRAM3': 0x20040000,
}

data = open(sys.argv[1], 'rb').read()
words = {}
for off in range(0, len(data) - 3, 4):
    w = struct.unpack_from('<I', data, off)[0]
    words.setdefault(w, 0)
    words[w] += 1
for name, base in sorted(PERIPH.items(), key=lambda kv: kv[1]):
    # count literals pointing into the first 1 KiB of the peripheral block
    n = sum(c for w, c in words.items() if base <= w < base + 0x400)
    if n:
        print(f'{name:13} {base:#010x}  {n} literal(s)')
