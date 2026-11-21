/* STM32L4R7 – Hercules Stream 100.
 * The last 4 KiB flash page (0x081FF000) stores device-specific data (serial number, factory code,
 * settings). It is deliberately excluded so the linker can never place anything there.
 * SRAM1 (192K) + SRAM2 (64K) + SRAM3 (384K) are contiguous from 0x20000000. */
MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = 2048K - 4K
  RAM   : ORIGIN = 0x20000000, LENGTH = 640K
}
