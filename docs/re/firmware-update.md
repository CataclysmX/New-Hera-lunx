# Firmware : mise à jour & firmware custom

## Versions (2026-11-19)
- Device de l'utilisateur : **1.40** (`req 0x02` → `28 01`, message device-info `01 28 01`, idem dans
  les captures Windows d'avril 2026). Codage décimal : 0x28 = 40, 0x2a = 42.
- **Check de MAJ de l'app** : `GET https://ts.hercules.com/download/streaming/update/update.xml`
  (canal bêta : `…/updateBeta/update.xml`). Format : `<software>` (version, downloadURL, size, MD5,
  changelogUrl, FW minimal compatible par device) + `<firmwares><controller name="HSM01">` (version,
  **MD5 du `.hsm`**, downloadURL, app minimale). Copies : `re_work/net/`.
- Contenu au 17/11/2026 (stable **et** bêta) : app **2.5.8** ; firmware HSM01 = **1.40** (`mandatory`),
  lien direct `…/update/HSM01_S32L4R7_v1_40.hsm` (MD5 `1e7eb785…` vérifié) ; HSM02 (200 XLR) = 2.39.
- `HSM01_S32L4R7_v1_42.hsm` (en-tête @0x68 = 0x012a, date 2025-08-19 ; 1.40 = 0x0128, 2024-01-12) n'est
  **pas publiée** dans le manifeste : elle n'existe que dans le paquet driver (`HSM.exe`) des installeurs
  v2.5.4 et v2.5.8 (fichier identique, MD5 `7dd31bb3…`). L'updater en ligne ne la propose donc jamais.
  Statut (bêta / en attente de validation ?) inconnu → prudence.
- 1.40 → 1.42 : 563 091 octets différents sur 1 953 792.
- **Image 1.40 officielle récupérée** = image de secours du firmware actuel (backup DFU sans relire la puce).
- Forum Hercules : gels aléatoires rapportés sur 1.40 ; correctifs de 1.42 inconnus (pas de changelog).

## Fichier `.hsm`
En-tête `FWFG` 0x80 o puis image brute flash @0x08000000 (1 953 792 o ≈ toute la flash 2 Mo ;
vecteurs SP=0x2008f5f0, reset=0x08000479). Champs : `"Hercules"`, `"HSM01"`, `"SL4R"` (STM32L4R),
version 0x012a, date, @0x50 0x16db77ff et @0x7c 0x3591a3f6 = contrôles (≠ CRC32 zlib / STM32 sur
body, body[:0x91ef0] ou header : `tools/re/fw_crc_probe.py`), @0x54 = 0x91ef0 (taille code ?).
Algo de contrôle : `GuiSTDFUDevUpdate64.dll` (`DfuImage::CheckFileCRC`, `CANSPIcrc32`, CRC par secteur) – à reverser.

## Chaîne de MAJ officielle (Windows)
Stream Control → `hercHsmInstall.exe` → `GuiSTDFUDevUpdate64.dll` + driver `GuiSTDFUDev.sys` :
1. requête vendor `0x40 0xF2 wValue=0 wIndex=0xE053` → reboot en **bootloader ROM ST DFU** `0483:df11`
   (alternative utilisateur : maintenir clic molette 1 + 4 au branchement – code dans le firmware applicatif) ;
2. DfuSe : effacement, téléchargement, vérification (errERASE / errCHECK_ERASED / errVERIFY),
   gestion de la protection en lecture (`GuiSTDFU_IsDeviceReadProtected`, `DisableDeviceReadProtection`
   ⇒ mass erase).

## Options sous Linux
| Option | Faisabilité | Risque |
|--------|-------------|--------|
| App Windows sous Wine/Proton | ✘ : SDK = `SetupDi` + `DeviceIoControl` vers drivers **noyau** KMDF (`HercStreamMixDevice.sys`, `GuiSTDFUDev.sys`) que Wine ne charge pas | – |
| VM Windows + passthrough USB (06f8:e053 **et** 0483:df11) | ✔ outil officiel | faible (= Windows natif) |
| `dfu-util` natif | ◐ après RE de `GuiSTDFUDevUpdate64.dll` (contrôles, RDP, séquence exacte) | moyen |

## Firmware custom – évaluation
- Faisable en théorie : STM32L4R7 (Cortex-M4F, 2 Mo flash, 640 Ko RAM), bootloader ROM non effaçable,
  firmware officiel décompilé (`re_work/decomp/fw_hsm01_v1_42.c`) comme référence matérielle
  (GPIO boutons, ADC molettes, rétroéclairage, USB ISO/INT).
- Risques : si le firmware custom casse l'USB **et** le combo molettes 1+4, retour au DFU uniquement via
  BOOT0/SWD (ouverture du boîtier). **Ne jamais écrire les option bytes / RDP niveau 2** (verrouillage
  définitif du DFU et du SWD). Backup 1.40 probablement impossible si RDP1 (upload DFU refusé).
- Prérequis avant d'essayer : sonde SWD (ST-Link), repérage des pads SWD/BOOT0, maîtrise de la MAJ officielle.
- Recommandation : reporter ; d'abord finir la partie hôte (drivers + app) qui couvre l'existant.

### Étape 1 réalisée (2026-11-19, lecture seule) – `backups/stream100-fw1.40-2026-11-19/` (hors git)
- Entrée DFU depuis Linux ✔ (requête `40 F2 0000 E053 0000`) ; sortie = débrancher/rebrancher ✔.
- Bootloader ROM : bcdDevice 0x2200, alt 0 `Internal Flash 0x08000000 512×4K`, alt 1
  `Option Bytes 0x1FF00000 / 0x1FF01000 (2×64 o)`, alt 2 `OTP 0x1FFF7000 1K`.
- **Option bytes** (`tools/re/stm32l4_ob.py`) : OPTR = 0xFFEFF8AA → **RDP niveau 0** (pas de protection),
  WRP/PCROP désactivés, DBANK=1, BFB2=0, nBOOT0=1, **nSWBOOT0=1 (BOOT0 = broche physique)**.
- **Sauvegarde complète** : `flash_full_backup.bin` (2 Mo) + `otp.bin` (SHA-256 dans `backup.sha256`).
  Plage 0x08000000–0x081DD000 = image officielle 1.40 à l'octet près ; 0x081DD000–0x081FF000 vide ;
  **0x081FF000 (4 Ko) = mémoire info propre à l'appareil** (n° série, code usine, réglages – à préserver) ;
  OTP = `53 e0 f8 06` (VID/PID).
- Conséquences : on peut flasher sans mass-erase et restaurer **l'état exact** de l'appareil ;
  récupération matérielle possible via la broche BOOT0 (à localiser) en plus du SWD.

### Matériel utilisé par le firmware officiel (`tools/re/fw_peripherals.py`, v1.40)
LTDC (écran RGB parallèle, framebuffer en SRAM3 0x20040000 – le fond 8 bits + palette correspond à
un calque L8/CLUT), OCTOSPI1 mappée @0x90000000 (+ OCTOSPI2) = flash externe (logo, graphismes),
ADC (molettes à double curseur), TIM1/2/3/4/5/8/15/16/17 (PWM rétroéclairage/LEDs, base de temps),
DMA1/2 + DMAMUX, USB OTG FS, GPIOB..H. Pas de DMA2D détecté.

### Ce qu'un firmware custom permettrait
- Écran libre : framebuffer RGB565 plein écran (261 Ko, tient en SRAM3 384 Ko), texte/polices, mise en
  page quelconque, animations ; rendu de widgets côté device ou mise à jour par rectangles.
  Limite : USB Full-Speed ≈ 0,95 Mo/s → ~3,6 img/s en plein écran brut, fluide en mises à jour partielles.
- Classes USB standard (composite) : **HID** (raccourcis, touches média, volume système) et/ou **MIDI**
  (molettes en CC relatifs, boutons en notes) → fonctionne sans driver sur Linux/Windows/macOS.
- Configuration stockée dans le device (flash interne libre 136 Ko + flash externe) : utilisable sans app.
- Molettes : position absolue / résolution fine / accélération ; LEDs : intensité par LED si câblées en PWM.
- Correction des gels, logo animé, compatibilité optionnelle avec le protocole officiel (app Windows OK).

### Plan si on se lance (du moins au plus risqué)
1. `dfu-util` + entrée en DFU (`0xF2` ou molettes 1+4) : lister les alt settings, lire les **option bytes**
   (niveau RDP) – non destructif ; un débranchement ramène au firmware officiel.
2. Cartographier le matériel depuis `fw_hsm01_v1_42.c` (GPIO boutons, ADC molettes, timers PWM
   rétroéclairage/LEDs, contrôleur LCD, flash externe, USB) + photos du PCB (pads SWD/BOOT0).
3. Achat ST-Link, test d'accès SWD → récupération garantie quoi qu'il arrive (sauf RDP2).
4. Firmware minimal (Rust `embassy-stm32` ou C/CubeL4) qui **garde la trappe DFU en tout premier**
   (mot magique RAM + combo molettes 1+4 avant toute autre init), clignote une LED.
5. Puis écran, entrées, USB (classe vendor compatible protocole actuel, ou HID/MIDI class-compliant).
Retour arrière à tout moment : `dfu-util -a 0 -s 0x08000000:leave -D hsm01_v1_40_body.bin` (image officielle).
