# USB – descripteurs & init (Stream 100 = HSM01)

Source : `lsusb -v`, capture `testzireshqrk3.txt` (frames 13-46), DLL `hsm_api_core` (thread device `FUN_180005e60`).

## Identité
- VID:PID `06f8:e053` (Guillemot / "Hercules" / "Stream 100"), USB 1.0 Full-Speed, bcdDevice 0.01, 500 mA.
- Mode boot/DFU : string "Hercules Stream 100 (Boot)" → ST DFU (driver `GuiSTDFUDev`). MCU **STM32L4R7**.
- `06f8:e054/e055` = Stream 200 XLR (driver Thesycon `tlusbapi`, hors périmètre).

## Interfaces (config 1)
| If | Alt | EP | Type | Taille | Usage |
|----|-----|----|------|--------|-------|
| 0 | 0 | 0x02 OUT | Bulk | 64 | non utilisé en fonctionnement normal (MAJ ?) |
| 0 | 0 | 0x81 IN | Interrupt (bInterval 16) | 64 | contrôles + paquets `sm` → [framing](framing.md) |
| 1 | 0 | – | – | – | idle |
| 1 | 1 | 0x01 OUT | **Isochrone** (1 ms) | 952 | trames écran `HERCULES`+`SM` |

## Séquence d'init observée (driver Windows)
1. `SET_CONFIGURATION 1`
2. `SET_INTERFACE if=1 alt=1`
3. `c0 52 0000 0000 len 2` → `01 02` (banque 0, 1 octet = 0x02)
4. `c0 52 0001 0000 len 9` → `08 6e c2 a7 07 07 9b 00 00` = [len=8][**n° de série** 8 o]
5. GET_DESCRIPTOR string 2 & 1
6. `c0 02 0000 0000 len 2` → `28 01` = version firmware 1.40 (relue périodiquement)
7. Lectures interrupt IN 0x81 en continu (16 URBs en vol), écriture ISO à ≥ 2 Hz (keep-alive 500 ms).
Arrêt : `SET_INTERFACE if=0 alt=0`, `SET_CONFIGURATION 0`.

## IOCTL driver Windows (HercStreamMixDevice.sys, KMDF) – pour mémoire
| IOCTL | Sens | Rôle déduit |
|-------|------|-------------|
| 0x1e2000 | out 64 | lecture report interrupt IN |
| 0x1e2005 | in N | écriture ISO (N multiple de 952) |
| 0x1e2008 | out 4 | taille paquet ISO (952) |
| 0x1e200c | out 4 | taille paquet INT IN (64) |
| 0x1e2010 | out 4 | ? (lu à l'ouverture) |
| 0x1e2024 / 0x1e2034 | in 4 (=0) | réglages internes (reset/mode) |
| 0x1e2014 / 0x1e202c | in 4 | params registre (MaxNumberOfWriteReadIterations…) |

## Requêtes vendor (firmware `FUN_08008d00`, bmRequestType 0xC0)
| bRequest | wValue | wIndex | Réponse / effet |
|----------|--------|--------|-----------------|
| 0x02 | – | – | **version firmware** `[minor][major]` (`2a 01` = v1.42 ; capture : `28 01` = v1.40) |
| 0x52 | offset | banque 0/1 | lecture mémoire info (dernière page flash 0x081FF000, émul. EEPROM) : `[n][n octets]`. Banque 0 = `02` + **n° de série** 8 o (offset 1..8) |
| 0x0e | 1..0x1000 | ≤0x3ff | wLength 4 → espace libre du ring ISO (`0xfeff - occupé`) + réglage interne |
| 0x0f | 1..0x1000 | ≤0x3ff | réglage interne (2 × u16), non utilisé par la DLL normale |
| 0xF2 | **0** | **0xE053** | ⚠ **reboot en bootloader ST-DFU** (`0483:df11`), bmRequestType **0x40**, sans données (driver Windows `RebootToDfuMode`). Testé ✔ : `tools/hsm01_dfu_enter.py --yes` ; débrancher/rebrancher = retour au firmware |

Mode boot : DFU ST standard `0483:df11` (driver Windows `GuiSTDFUDev`) → sous Linux `dfu-util`.

Sous Linux : pas besoin de driver noyau, **libusb** suffit (classe vendor). Accès non-root :
`packaging/udev/70-hercules-stream100.rules` (uaccess).
