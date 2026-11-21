# 09 – Écrire son propre firmware (Rust / embassy) et le flasher sans risque

Objectif : « libérer l'écran » – un firmware qui accepte des images quelconques par USB et renvoie
les entrées brutes, pour que tout le rendu se fasse sur le PC.

## 1. Outils et projet

```sh
rustup target add thumbv7em-none-eabihf
cd firmware/rust
cargo build --release
llvm-objcopy -O binary target/thumbv7em-none-eabihf/release/hsm01-fw hsm01-fw.bin
ls -l hsm01-fw.bin
cd ../..
```

Fichiers clés :
- `Cargo.toml` : `embassy-stm32` (features `stm32l4r7zi`, `dual-bank`, `time-driver-tim5`),
  `embassy-executor`, `embassy-usb`, `cortex-m-rt` (feature `set-vtor`).
- `.cargo/config.toml` : cible `thumbv7em-none-eabihf`.
- `memory.x` : `FLASH : ORIGIN = 0x08000000, LENGTH = 2048K - 4K` → la **dernière page** (données de
  l'appareil) est hors d'atteinte ; `RAM : ORIGIN = 0x20000000, LENGTH = 640K`.
- `build.rs` : place `memory.x` + convertit le logo (fichier 12).
- `src/escape.rs` : la **trappe DFU** ; `src/main.rs` : init + tâches ; `src/proto.rs` : protocole ;
  `src/gfx.rs` : framebuffer.

## 2. La trappe DFU d'abord (règle d'or)

Première chose exécutée par `main`, avant toute init d'horloge :
1. activer GPIOA/GPIOC, mettre PA15 et PC12 (clics molettes 4 et 1) en entrée pull-up, attendre ~10 ms ;
2. si les deux sont appuyés **ou** si un mot magique (`0xDF00B007`) est présent en SRAM2 (déposé avant
   un reset logiciel) → mettre la dalle en veille (PA8, PA5, PB10 à 0), remapper la mémoire système
   (`SYSCFG.MEMRMP = 1`) et **sauter dans le bootloader ROM** (`0x1FFF0000`).

Ainsi, même un firmware qui plante plus loin reste récupérable sans démontage. On le **vérifie au
désassemblage** avant chaque flash :

```sh
cd firmware/rust
llvm-objdump -d target/thumbv7em-none-eabihf/release/hsm01-fw --no-show-raw-insn \
  | awk '/__cortex_m_rt_main>:/{f=1} f{n++; print; if(n>120) exit}' \
  | grep -nE "main>:|0x104c|0x1000|0x9c41|0xb007|0xdf00|memclr|embassy_stm32.*init"
cd ../..
```

Il faut voir l'accès RCC (`0x1000`/`0x104c`), le délai (`0x9c41`) et le mot magique (`0xb007`/`0xdf00`)
**avant** `__aeabi_memclr` et `embassy_stm32::init`.

Tester les deux trappes au premier flash : `python3 -I tools/hsm01_dfu_enter.py --yes` (requête USB)
et « clics molettes 1+4 au branchement » → `lsusb` doit montrer `0483:df11`.

## 3. Progression par paliers

1. **Palier 1** : trappes DFU + mire de test (barres de couleur) + LEDs + USB minimal (requêtes `0x02`
   version → `240.01`, `0xF2` DFU). Le majeur **240** distingue le firmware custom de l'officiel.
2. **Palier 2** : protocole « écran libre » (`docs/firmware/protocol.md`) :
   - bulk OUT `0x01` : messages `[0xA5][op][0 0][longueur u32][données]` — FILL (rectangle uni),
     BLIT (pixels RGB565), BLIT_RLE (pixels compressés), BACKLIGHT, LEDS ;
   - interrupt IN `0x81` : toutes les 10 ms, boutons + 8 valeurs ADC brutes des molettes + rétro.
3. **Ensuite** : logo de démarrage animé (fichier 12), corrections (fichier 10).

## 4. Outils côté PC pour tester le firmware

```sh
python3 -I tools/hsm01_whoami.py              # version (240.xx = custom)
python3 -I tools/fw_smoke.py                  # scène de test + 3 rapports + débit
python3 -I tools/fw_bench.py 512              # débit par type de message
python3 -I tools/fw_geometry.py               # mire de géométrie (bordure, lignes, coins colorés)
python3 -I tools/fw_ab.py checker|grey        # test de rémanence
python3 -I tools/fw_regs.py                   # relit horloges / LTDC / GPIO (requête 0x03)
python3 -I tools/fw_knobs.py 60               # enregistre les curseurs des molettes (CSV)
python3 -I tools/fw_vu_demo.py 60 15          # VU-mètres dessinés côté PC à 60 fps
python3 -I tools/fw_swatches.py               # nuancier pour choisir une couleur sur la dalle
driver/rust/target/release/hsm01 fw info|fill 255 0 0|image photo.png|backlight 50|leds 100 50 20 0|monitor 20|bench 30|dfu
driver/c/build/hsm01-c fw info|fill …|image photo.ppm|…          # version C (images en PPM)
```

## 5. Tester le parseur sans l'appareil

Le parseur du firmware (`src/proto.rs`) est inclus tel quel dans un petit projet de tests sur PC :

```sh
cd firmware/proto-tests && cargo test && cd ../..
```

Il compare les chemins rapides (copie de lignes entières, remplissages) au traitement octet par octet
sur des flux aléatoires découpés aléatoirement : même framebuffer attendu.

## 6. Flasher

Procédure complète : fichier 07, section 5 (DFU → écriture → relecture → `:leave`). Restauration du
firmware d'origine : fichier 07, section 4.
