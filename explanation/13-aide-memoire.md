# 13 – Aide-mémoire

## État de l'appareil

```sh
lsusb | grep -iE '06f8|0483'           # 06f8:e053 = firmware (officiel ou custom), 0483:df11 = DFU
python3 -I tools/hsm01_whoami.py       # 1.40 = officiel, 240.xx = custom
journalctl -k -f | grep "usb 1-1"      # suivre l'énumération
```

## DFU, flash, sauvegarde

```sh
python3 -I tools/hsm01_dfu_enter.py --yes                    # → DFU (ou clics molettes 1+4 au branchement)
dfu-util -l
dfu-util -d 0483:df11 -a 0 -s 0x08000000 -D firmware/rust/hsm01-fw.bin          # écrire
dfu-util -d 0483:df11 -a 0 -s 0x08000000:<taille> -U relu.bin && cmp relu.bin firmware/rust/hsm01-fw.bin
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave                                  # démarrer
dfu-util -d 0483:df11 -a 0 -s 0x08000000:0x200000 -U flash_full_backup.bin       # sauvegarde complète
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave \
         -D backups/stream100-fw1.40-2026-11-19/restore_first_256k.bin            # retour au firmware d'origine
# INTERDIT : écrire l'alt 1 (option bytes), effacer 0x081FF000 (dernière page)
```

## Analyse

```sh
python3 -I tools/re/is_unpack.py <setup.exe> <dossier>              # InstallShield
python3 -I tools/re/iscab_unpack.py <dossier>/Disk1 <sortie>        # cab InstallShield
python3 -I tools/re/pe_imports.py --exports <fichier.dll>
/opt/ghidra/support/analyzeHeadless re_work/ghidra_proj p -import <binaire> \
    -scriptPath tools/re -postScript DecompileAll.java "$PWD/re_work/decomp/<nom>.c" -deleteProject
#   firmware brut : -processor ARM:LE:32:Cortex -loader BinaryLoader -loader-baseAddr 0x08000000 -preScript FwSetup.java
python3 -I tools/re/fw_word.py <fw.bin> <adresse> [--f64]           # lire une constante
python3 -I tools/re/fw_find_word.py <fw.bin> <valeur>               # où est chargée une valeur
python3 -I tools/re/fw_xref_range.py <fw.bin> <decomp.c> <lo> <hi>  # quelles fonctions
python3 -I tools/re/wsdump.py capture.txt > capture.tsv             # export Wireshark → tableau
python3 -I tools/re/hsm01_proto.py check|decode|decode-v capture.tsv
```

## Compilation

```sh
(cd driver/rust && cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release)
(cd driver/c && cmake -S . -B build -DHSM01_WERROR=ON -DCMAKE_BUILD_TYPE=Release && cmake --build build && ctest --test-dir build)
(cd firmware/rust && cargo build --release && llvm-objcopy -O binary target/thumbv7em-none-eabihf/release/hsm01-fw hsm01-fw.bin)
(cd firmware/proto-tests && cargo test)
```

## Firmware custom : outils PC

```sh
driver/rust/target/release/hsm01 fw info | fill r g b | image f.png | backlight 0-100 | leds a b c d | monitor 20 | bench 30 | dfu
python3 -I tools/fw_regs.py          # pixel clock doit être 10.000 MHz / 63.1 Hz
python3 -I tools/fw_geometry.py      # mire
python3 -I tools/fw_vu_demo.py 60 15
python3 -I tools/fw_swatches.py #rrggbb …
python3 -I tools/fw_knobs.py 60
```

## Valeurs à connaître

| Quoi | Valeur |
|------|--------|
| appareil | `06f8:e053`, STM32L4R7, USB Full Speed |
| bootloader ROM | `0483:df11`, entrée par requête `0x40 F2 0000 E053` |
| flash | `0x08000000`, 2 Mo, pages 4 Ko ; dernière page `0x081FF000` = données de l'appareil |
| écran | 480×272 RGB565, horloge pixel 10 MHz, 63 Hz |
| firmware custom | version `240.xx`, bulk OUT `0x01`, rapports IN `0x81` / 10 ms, diag `0xC0 03` |
| molettes | ~512 pas par tour (unités du firmware officiel) |
