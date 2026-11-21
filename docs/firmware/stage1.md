# Firmware custom – palier 1 (trappes DFU + mire écran)

Sources : `firmware/rust/` (Rust, embassy-stm32 0.6 `stm32l4r7zi` + `dual-bank` + `time-driver-tim5`,
embassy-usb 0.6, cortex-m-rt `set-vtor`). Binaire ≈ 21 Ko.
USB : `06f8:e053`, produit « Stream 100 (hsm01-fw stage 1) », bcdDevice 0xF001, interface vendor,
requêtes `0x02` (version → 240.01) et `0x40 F2 0000 E053` (→ DFU). HSI48 + CRS (sans quartz).
`tools/hsm01_whoami.py` indique quel firmware tourne.

## Résultats (2026-11-19)
| Test | Résultat |
|------|----------|
| Flash DFU + relecture | ✔ identique à `hsm01-fw.bin` (6 pages, 0x08000000–0x08005FFF) |
| Mire écran + chenillard LEDs | ✔ validé visuellement |
| Énumération USB / requête 0x02 | ✔ 240.01 |
| Porte USB (`tools/hsm01_dfu_enter.py --yes`) | ✔ → `0483:df11` |
| Porte molettes 1+4 au branchement | ✔ → `0483:df11` (écran éteint, bootloader ROM) |
| Porte molettes 1+4 maintenues ~1 s en fonctionnement | non testé |

**Palier 1 validé** : deux voies logicielles indépendantes vers le DFU, sans ouvrir le boîtier.

## Comportement
1. **Trappe DFU en premier** (état reset, MSI 4 MHz) : clic molette 1 (PC12) + clic molette 4 (PA15)
   maintenus au branchement, ou mot magique `0xDF00B007` en SRAM2 (0x2003FC04, non effacée au reset
   car `SRAM2_RST=1`) → `SYSCFG.MEMRMP=1`, MSP/PC depuis 0x1FFF0000 → bootloader ROM `0483:df11`.
2. Horloges : MSI 4 MHz → PLL 80 MHz (pas de mode boost), PLLSAI2 80 MHz → LTDC ÷8 = 10 MHz.
3. LTDC 480×272 RGB565 (timings officiels, VBP 12), mire : barres couleur, dégradés, bordure,
   carré animé ; DISP (PA8), rétroéclairage PA5 + PB10 à 100 % ; chenillard LEDs PC6..PC9.
4. En fonctionnement : clics molettes 1 + 4 maintenus ~1 s → reboot DFU (via le mot magique).

## Build / vérifs
```sh
cd firmware/rust && cargo build --release
llvm-objcopy -O binary target/thumbv7em-none-eabihf/release/hsm01-fw hsm01-fw.bin
```
`memory.x` exclut la dernière page (FLASH = 2044 Ko) ; image ≈ 5,4 Ko → pages 0x08000000–0x08001FFF.
Vérifié au désassemblage : la trappe s'exécute avant toute init d'horloge.

## Flash / test / retour arrière
```sh
python3 -I tools/hsm01_dfu_enter.py --yes                 # firmware officiel → DFU
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D firmware/rust/hsm01-fw.bin
# test trappe : débrancher, maintenir clics molettes 1+4, rebrancher → lsusb montre 0483:df11
# retour au firmware officiel (palier 1 : 6 pages ; palier 2 : 7 pages → fichier 32 Ko) :
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D backups/stream100-fw1.40-2026-11-19/restore_first_32k.bin
```
Secours ultime si la trappe ne répond pas : broche BOOT0 ou SWD (ouverture du boîtier).
