# État de l'investigation

| Question | État | Preuve |
|----------|------|--------|
| Pile Windows (app Qt → SDK `hsm_api_core` → KMDF → USB) | ✔ | artifacts.md |
| Driver noyau nécessaire sous Linux ? | ✔ non : libusb + règle udev | usb.md |
| Init USB + requêtes vendor (0x02 version, 0x52 info/série, 0xf2 DFU) | ✔ capture + firmware | usb.md |
| Framing OUT (HERCULES/SM/CRC/seq/padding) | ✔ 3305/3305 + firmware | framing.md |
| Framing IN (`sm`, checksum, types 1/2) | ✔ 6483/6483 | framing.md, inputs.md |
| Opcodes : liste + longueurs | ✔ firmware (24 opcodes) ; 0 erreur sur captures | commands.md |
| Sémantique opcodes | ✔ écran on/off, luminosité, LEDs, icônes 32×32, labels 110×16, VU, fond, palette ; ◐ 0x32, 0x40 détails | commands.md |
| Bloc contrôles IN | ✔ 4 clics molette + 4 boutons + power + 4 molettes + état écran, mapping physique confirmé live | inputs.md |
| Écran | ✔ 480×272, rendu par le firmware (pas de framebuffer brut : éléments + fond indexé) | commands.md |
| MAJ firmware | ✔ ST-DFU `0483:df11` (`dfu-util`) ; image `.hsm` = en-tête FWFG 0x80 + binaire | usb.md |
| Modèle fonctionnel de l'app (canaux, pages, actions, intégrations) | ◐ chaînes/traductions ; décompilation `Stream-control.exe` en cours | – |
| Validation live sur le device | ✔ init, keep-alive, ack 0x14, écran/luminosité acceptés (err=0) ; ◐ LEDs/visuel à confirmer | live-tests.md |

| Driver Rust (`driver/rust`) | ✔ proto + usb + CLI ; `info` et `demo` validés en live (icônes, labels, LEDs, VU OK) | driver/rust.md |
| Driver C (`driver/c`) | ✔ à parité avec le Rust (2026-11-20) : proto + fond 256 couleurs (mêmes résultats que le Rust, empreintes communes) + usb + CLI `hsm01-c` (`background`, `splash`, images PNG/JPEG/GIF/BMP/PPM, `fw` avec COPY et économiseur 240.22), tests gcc/clang `-Werror` ; ✔ protocole libre `fw …` validé en live (240.04) ; ◐ `demo`, `background` et `splash` (firmware officiel) à tester | driver/c.md |
| CI release | ✔ `release-rust.yml` (tag `rust-v*`), `release-c.yml` (tag `c-v*`) | driver/*.md |
| Logo de démarrage custom | ◐ implémenté (Rust : `background`, `splash`) d'après DLL+fw ; test live à faire | commands.md |
| Firmware custom palier 1 | ✔ écran LTDC, LEDs, USB, portes DFU USB **et** molettes 1+4 validées | firmware/stage1.md |
| Firmware custom palier 2 (écran libre) | ✔ 240.13 : bulk 936 Kio/s (driver USB patché), FILL/BLIT/RLE, PWM rétro/LEDs, reports curseurs bruts, logo de démarrage (délai 10 s), horloge pixel 10 MHz corrigée (rémanence) ; ✔ décodage molettes ; ◐ démarrage à froid | firmware/stage2.md, firmware/knobs.md |
| Fonctions cachées du fw | ✔ overlay debug, mode autonome, DFU, usine | re/firmware-hidden.md |
| MAJ firmware 1.40 → 1.42 | ◐ dispo dans l'installeur ; Wine impossible ; VM Windows ou RE de l'updater | re/firmware-update.md |

Prochaines étapes
1. Test live `hsm01-c demo` (même rendu attendu que la version Rust).
2. MAJ firmware : VM Windows + passthrough, ou RE de `GuiSTDFUDevUpdate64.dll` pour un updater Linux.
3. Logique app (pages, actions, intégrations) – plus tard, une fois les primitives validées.
