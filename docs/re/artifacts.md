# Artefacts extraits de l'installeur (Hercules_Stream_Control_v2.5.4.exe)

Chaîne : InstallShield 31 "Setup Launcher Unicode" (overlay non chiffré) → `Disk1/data{1,2}.cab`
(cab InstallShield v31, noms UTF-16, zlib raw par blocs u16) → fichiers.
Extraction : voir [tools.md](tools.md). Résultat dans `re_work/` (non versionné, régénérable).

## App (`re_work/app/`)
- `Stream-control.exe` (x64, **Qt6**, namespace C++ `herculescreator`) – app companion.
  Charge `hsm_api_core_x64.dll` par `LoadLibraryW`+`GetProcAddress` (27 fonctions `HSM_*`).
  Intégrations : OBS (QWebSocket), Streamlabs, Twitch, audio Windows (sessions/apps), presets micro XML.
  Classes device : `IStreamMixDevice`, `StreamMix40` (Stream 100), `StreamMixPro` (200 XLR),
  `StreamMixSdk`, `Display`, `Buttons`.
- `tlusbdfuapi.dll` (Thesycon DFU, licence VID/PID 06f8:e054/e055 = Stream 200 XLR uniquement).
- ffmpeg/Qt runtime, vidéos tutoriel `StreamMix100/hercules100.mp4`.

## Pack driver `HSM.exe` (`re_work/hsm_files/`, 2ᵉ InstallShield imbriqué)
| Fichier | Rôle |
|---------|------|
| `HercStreamMixDevice.inf/.sys` (KMDF, v1.2.2.0) | driver **Stream 100** `USB\VID_06F8&PID_E053` (pompe ISO/INT + IOCTL) |
| `hsm_api_core_x64.dll` (v1.53, "HSM SDK") | **toute la logique protocole** (trames SM, CRC, parse IN) |
| `HSM01_S32L4R7_v1_42.hsm` | firmware Stream 100 (en-tête `FWFG` 0x80 o + image STM32L4R7 @0x08000000, non chiffré) |
| `GuiSTDFUDev.inf/.sys`, `GuiSTDFUDevUpdate*.dll`, `hercHsmInstall.exe` | mode boot ST-DFU + MAJ firmware |
| `HerculesStream200XLR*`, `tlusbapi_*`, `tlsetupfx` | Stream 200 XLR (Thesycon audio), hors périmètre |

## Fichiers de décompilation (`re_work/decomp/`, Ghidra 12.1 headless)
- `hsm_api_core_x64.c` (45 k lignes) – **source principale**. Fonctions clés : voir [dll-map.md](dll-map.md).
- `HercStreamMixDevice_sys.c` – dispatch IOCTL `FUN_140002a10`.
- `fw_hsm01_v1_42.c` – firmware (analyse partielle, 369 fonctions). Vérif SM : `FUN_08008f10`.
- `Stream-control.c` – app (en cours).

## Captures (`re_work/captures/*.tsv`, depuis les exports texte Wireshark du dossier racine)
`testzireshqrk3` = énumération + session complète ; `wireshark` = session app ; colonnes :
frame, time, src, dst, func, ep, payload_hex.
