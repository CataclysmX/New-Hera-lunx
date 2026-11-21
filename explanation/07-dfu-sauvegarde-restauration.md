# 07 – Mode DFU, sauvegarde complète, restauration

Le microcontrôleur (STM32L4R7) contient un **bootloader ROM de ST**, impossible à effacer, qui parle le
protocole standard **DFU** (`0483:df11`). Le firmware officiel sait y redémarrer sur demande USB.
C'est la porte d'entrée sans démontage.

## 1. Entrer en DFU

```sh
lsusb | grep -i -E "0483:df11|06f8:e053"
python3 -I tools/hsm01_dfu_enter.py --yes      # requête 0x40 / 0xF2 / wValue 0 / wIndex 0xE053
dfu-util -l                                     # liste les « alt settings » du bootloader
```

Autre méthode, sans PC : **maintenir les clics des molettes 1 et 4 en branchant** (prévu dans le
firmware officiel, et repris dans le firmware custom). Pour revenir au firmware : débrancher /
rebrancher, ou `dfu-util … -s 0x08000000:leave`.

`dfu-util -l` montre trois zones :

| alt | zone | adresse |
|-----|------|---------|
| 0 | flash interne | `0x08000000`, 512 pages de 4 Ko |
| 1 | option bytes | `0x1FF00000` et `0x1FF01000` (2 × 64 o) |
| 2 | OTP | `0x1FFF7000` (1 Ko) |

## 2. Lire les option bytes (protection) – lecture seule

```sh
mkdir -p re_work/dfu && cd re_work/dfu
dfu-util -d 0483:df11 -a 1 -s 0x1FF00000:64 -U ob_bank1.bin
dfu-util -d 0483:df11 -a 1 -s 0x1FF01000:64 -U ob_bank2.bin
python3 -I ../../tools/re/stm32l4_ob.py ob_bank1.bin ob_bank2.bin
```

Résultat : `OPTR = 0xFFEFF8AA` → **RDP niveau 0** (aucune protection : on peut tout relire), pas de
zones protégées, double banque (pages 4 Ko), BOOT0 = broche physique. Si RDP avait été au niveau 1,
la lecture aurait été refusée et la sauvegarde impossible : c'est le **premier** point à vérifier.

## 3. Sauvegarde complète

```sh
dfu-util -d 0483:df11 -a 0 -s 0x08000000:256 -U flash_head.bin             # test rapide
cmp <(head -c 256 ../fw/hsm01_v1_40_body.bin) flash_head.bin && echo "== image officielle 1.40"
dfu-util -d 0483:df11 -a 0 -s 0x08000000:0x200000 -U flash_full_backup.bin   # 2 Mo
dfu-util -d 0483:df11 -a 2 -s 0x1FFF7000:1024 -U otp.bin
sha256sum flash_full_backup.bin otp.bin | tee backup.sha256
python3 -I ../../tools/re/flash_map.py flash_full_backup.bin ../fw/hsm01_v1_40_body.bin   # carte page par page
python3 -I ../../tools/re/find_bytes.py flash_full_backup.bin 6ec2a707 4                 # où est le n° de série
cd ../..
mkdir -p backups/stream100-fw1.40-2026-11-19
cp re_work/dfu/{flash_full_backup.bin,otp.bin,ob_bank1.bin,ob_bank2.bin,backup.sha256} backups/stream100-fw1.40-2026-11-19/
(cd backups/stream100-fw1.40-2026-11-19 && sha256sum -c backup.sha256)
```

Ce que dit `flash_map.py` : `0x08000000–0x081DD000` = image officielle 1.40 à l'octet près ; ensuite
vide ; **dernière page `0x081FF000` = données propres à l'appareil** (n° de série, réglages). Cette page
ne doit **jamais** être effacée : le firmware custom l'exclut de son plan mémoire (fichier 09).

## 4. Restaurer le firmware d'origine

Le firmware custom n'écrit que le début de la flash. Il suffit de réécrire la même longueur depuis la
sauvegarde ; on prépare des fichiers « début de flash » :

```sh
cd backups/stream100-fw1.40-2026-11-19
head -c 262144 flash_full_backup.bin > restore_first_256k.bin     # couvre tout firmware custom < 256 Ko
sha256sum restore_first_256k.bin | tee -a backup.sha256
cd ../..
python3 -I tools/hsm01_dfu_enter.py --yes        # (ou molettes 1+4 au branchement)
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D backups/stream100-fw1.40-2026-11-19/restore_first_256k.bin
```

## 5. Flasher / vérifier / démarrer (procédure utilisée pour chaque version)

```sh
python3 -I tools/hsm01_dfu_enter.py --yes                       # depuis le firmware en cours
dfu-util -d 0483:df11 -a 0 -s 0x08000000 -D firmware/rust/hsm01-fw.bin
SIZE=$(stat -c %s firmware/rust/hsm01-fw.bin)
rm -f re_work/dfu/verify/readback.bin; mkdir -p re_work/dfu/verify
dfu-util -d 0483:df11 -a 0 -s 0x08000000:$SIZE -U re_work/dfu/verify/readback.bin   # relecture
cmp firmware/rust/hsm01-fw.bin re_work/dfu/verify/readback.bin && echo READBACK OK
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave                   # démarre le firmware
python3 -I tools/hsm01_whoami.py                                 # quel firmware tourne (1.40 ou 240.xx)
```

Notes :
- `:leave` démarre le firmware sans débrancher ; la ré-énumération prend ~1 s (attendre avant de
  conclure qu'il ne démarre pas).
- **Ne jamais écrire l'alt 1 (option bytes)** : un mauvais octet peut passer la puce en RDP niveau 2,
  irréversible (plus de DFU, plus de debug).
- Dernier recours matériel s'il n'y avait plus aucune trappe : broche BOOT0 ou sonde SWD (ST-Link),
  ce qui demande d'ouvrir le boîtier. Ça n'a jamais été nécessaire.
