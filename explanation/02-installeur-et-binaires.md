# 02 – Ouvrir l'installeur Windows et trouver les bons fichiers

Point de départ : `Hercules_Stream_Control_v2.5.4.exe` (installeur du logiciel officiel).

## 1. Identifier le type d'installeur

```sh
file Hercules_Stream_Control_v2.5.4.exe
7z l Hercules_Stream_Control_v2.5.4.exe | head -80     # 7z ne sait pas tout ouvrir ici
python3 -I tools/re/pe_overlay.py Hercules_Stream_Control_v2.5.4.exe
```

`pe_overlay.py` lit les en-têtes PE, calcule la fin de la dernière section et affiche ce qui suit :
c'est l'**overlay**, où les installeurs rangent leurs données. On y trouve la signature
**InstallShield** (« Setup Launcher Unicode », non chiffré).

## 2. Déballer InstallShield (deux niveaux)

```sh
mkdir -p re_work/installer re_work/app
python3 -I tools/re/is_unpack.py Hercules_Stream_Control_v2.5.4.exe re_work/installer
python3 -I tools/re/iscab_unpack.py re_work/installer/Disk1 re_work/x --list | head -150   # lister
python3 -I tools/re/iscab_unpack.py re_work/installer/Disk1 re_work/app                    # extraire
```

- `is_unpack.py` découpe l'overlay en fichiers (`Disk1/data1.hdr`, `data1.cab`, `data2.cab`…).
- `iscab_unpack.py` lit le format cab InstallShield v31 (noms en **UTF-16**, données compressées en
  zlib brut par blocs) et extrait les fichiers.

Dans `re_work/app/` on trouve `Stream-control.exe` (l'application Qt) et **`HSM.exe`**, un second
installeur InstallShield (le « pack driver ») :

```sh
cd re_work/app && file HSM.exe Stream-control.exe tlusbdfuapi.dll && cd ../..
mkdir -p re_work/hsm re_work/hsm_files
python3 -I tools/re/is_unpack.py re_work/app/HSM.exe re_work/hsm
python3 -I tools/re/iscab_unpack.py re_work/hsm/Disk1 re_work/hsm_files
ls -R re_work/hsm_files | head -80
```

## 3. Ce qu'on regarde dans chaque fichier

```sh
cd re_work/hsm_files
iconv -f utf-16 -t utf-8 HercStreamMixDevice.inf | tr -d '\r'   # .inf = quel VID/PID, quel .sys
strings -n 6 amd64/HercStreamMixDevice.sys | head -100
strings -n 6 -el amd64/HercStreamMixDevice.sys | head -60      # -el : chaînes UTF-16
head -c 256 HSM01_S32L4R7_v1_42.hsm | xxd | head -16             # en-tête du firmware
cd ../..
```

Inventaire obtenu (détails : `docs/re/artifacts.md`) :

| Fichier | Rôle |
|---------|------|
| `Stream-control.exe` | appli Qt6 (interface, intégrations OBS/Twitch/audio…) |
| `hsm_api_core_x64.dll` | **toute la logique du protocole** (le plus important) |
| `HercStreamMixDevice.sys` | driver noyau Windows (KMDF) : simple pompe USB + IOCTL |
| `HSM01_S32L4R7_v1_42.hsm` | **le firmware** (en-tête 128 o + image STM32L4R7) |
| `GuiSTDFUDev*.inf/.sys/.dll` | mise à jour du firmware par le bootloader ST (DFU) |

## 4. Imports, exports, chaînes

```sh
cd re_work
python3 -I ../tools/re/pe_imports.py --exports app/Stream-control.exe hsm_files/hsm_api_core_x64.dll \
    hsm_files/amd64/HercStreamMixDevice.sys hsm_files/amd64/hercHsmInstall.exe
mkdir -p strings
for f in app/Stream-control.exe hsm_files/hsm_api_core_x64.dll hsm_files/amd64/HercStreamMixDevice.sys \
         hsm_files/amd64/hercHsmInstall.exe hsm_files/HSM01_S32L4R7_v1_42.hsm; do
  b=$(basename $f); strings -n 5 $f > strings/$b.a.txt; strings -n 4 -el $f > strings/$b.w.txt
done
grep -n -i -E "hsm_api|LoadLibrary|GetProcAddress|^HSM_" strings/Stream-control.exe.*.txt | head -60
grep -c "HERCULES" app/Stream-control.exe hsm_files/hsm_api_core_x64.dll hsm_files/HSM01_S32L4R7_v1_42.hsm
cd ..
```

Ce que ça apprend :
- l'appli charge la DLL avec `LoadLibraryW` et récupère 27 fonctions `HSM_*` → **la DLL est l'API** ;
- la chaîne `HERCULES` apparaît dans la DLL **et** dans le firmware → c'est un marqueur de trame ;
- les noms de classes C++ (`.?AV...@herculescreator@@`) donnent l'architecture de l'appli :

```sh
grep -E "^\.\?AV.*herculescreator" re_work/strings/Stream-control.exe.a.txt \
  | sed 's/^\.?AV//; s/@herculescreator@@$//' | sort -u | tr '\n' ' '
```

## 5. Bonus : traductions et ressources de l'application Qt

```sh
cd re_work && mkdir -p ts
lconvert6 -i app/translations/herculescreator_fr.qm -o ts/herculescreator_fr.ts   # textes de l'interface
grep -o -E "<translation>[^<]+" ts/herculescreator_fr.ts | sed 's/<translation>//' \
  | grep -i -E "bouton|molette|écran|page|LED|luminos" | sort -u | head -60
python3 -I ../tools/re/qrc_carve.py app/Stream-control.exe qrc_svg   # images SVG embarquées (ressources Qt)
cd ..
```

Les traductions donnent le vocabulaire fonctionnel (pages, actions, intégrations) sans lire de code.
