# Matériel du Stream 100 (déduit du firmware officiel 1.40, `re_work/decomp/fw_hsm01_v1_40.c`)

MCU **STM32L4R7** (Cortex-M4F, 2 Mo flash double banque, 640 Ko SRAM), firmware HAL STM32Cube.
`main` = `FUN_0800dc8c`. Récupération : DFU ROM (`0483:df11`), BOOT0 physique, SWD.

## Horloges (`FUN_0800616c` = SystemClock_Config)
| Domaine | Source | Fréquence |
|---------|--------|-----------|
| SYSCLK | MSI 4 MHz (range 6), PLL M=1 N=60 R=2 | **120 MHz** (Scale1 boost, latence flash 5) |
| LSE | **jamais démarré** (`OscillatorType` = MSI seul) : l'appel `HAL_RCCEx_EnableMSIPLLMode` est sans effet ; attendre `LSERDY` bloque la carte (quartz absent ou muet, test 240.05) | – |
| USB | PLLSAI1 M=1 N=24 Q=2 (MSI non asservi) | 48 MHz (firmware custom : HSI48 + CRS) |
| LTDC | PLLSAI2 M=1 N=40 R=2 = 80 MHz, diviseur LTDC /8 (`CCIPR2.PLLSAI2DIVR`, **modifiable seulement PLLSAI2 arrêtée**) | **10 MHz** pixel clock (~63 Hz) ; 40 MHz = dalle sur-cadencée → rémanence |
Pas de HSE.

## Écran (LTDC, `FUN_0800d000` = MX_LTDC_Init, `FUN_08002500` = MspInit)
- 480×272, polarités HS/VS/DE/PC actives basses.
- H : HSW 1, HBP 43, actif 480, HFP 8 (total 531) ; V : VSW 10, VBP 12, actif 272, VFP 4 (total 297).
  Registres HAL : Hsync 0, Vsync 9, AccHBP 43, AccVBP 21, AccActW 523, AccActH 293, TotW 531, TotH 297.
- Calque 0 : **RGB565**, fenêtre 0..480 × 0..272, alpha 255, blending CA/CA, framebuffer en SRAM (variable).
- Broches AF11 : **PE0–PE15**, **PB2**, **PD0, PD1, PD3, PD6, PD8, PD9, PD10, PD11, PD12, PD14, PD15**.
- **PA8** sortie push-pull mise à 1 (DISP / enable dalle).
- Interruption ligne 293 (IRQ 91 LTDC, 92 LTDC_ER, priorité 0x90) – swap de framebuffer.
- Rétroéclairage : **TIM2 CH1 sur PA5 (AF1)**, PSC 0, ARR 499 (240 kHz), duty = % × 499 / 100 ;
  **PB10** = enable (1 = allumé). `FUN_08010a70(level, mode)`.

## LEDs boutons
**TIM8 CH1–CH4 sur PC6–PC9 (AF3)**, une LED par canal : CCR = intensité × 499 / 100 si allumée, 0 sinon
(`FUN_0800cf78`, `FUN_0800cf20`) → intensité réglable par LED possible.
TIM2 CH2 sur **PB3 (AF1)** : usage inconnu ; officiel : PWM1 impulsion 0, canal jamais démarré (aucune
écriture directe TIM2). Seule broche que le firmware custom laissait différente (JTDO flottante) ;
reproduite à l'identique depuis 240.08. Polarités/timings LTDC officiels : HS/VS/DE/PC = 0, Hsync 0,
Vsync 9, AccHBP 43, AccVBP 21, AccActW 523, AccActH 293, TotW 531, TotH 297 (= firmware custom).

## Entrées (actives basses)
| Contrôle | Broche |
|----------|--------|
| Bouton 1 / 2 / 3 / 4 | **PB15 / PB14 / PB13 / PB12** |
| Power | **PB11** |
| Clic molette 1 / 2 / 3 | **PC12 / PC11 / PC10** |
| Clic molette 4 | **PA15** |
| Molettes (potentiomètres double curseur) | ADC1 (`FUN_0800e128`), canaux à cartographier |
Trappe DFU du firmware custom : clic molette 1 (PC12) + clic molette 4 (PA15) maintenus au boot.

## Autres
- Flash externe : **OCTOSPI1** mappée @0x90000000 (+ OCTOSPI2) – logo, graphismes. Broches à cartographier.
- USB OTG FS (PA11/PA12), device full-speed.
- Mémoire info (n° série, réglages) : dernière page flash **0x081FF000** – ne jamais l'effacer.
- TIM1/3/4/5/15/16/17, DMA1/2, DMAMUX utilisés : rôles à préciser (ADC DMA, base de temps…).
