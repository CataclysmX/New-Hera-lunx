# hsm01 – driver C (Hercules Stream 100, Linux)

Port C11 de la version Rust ([rust.md](rust.md)) : même découpage, mêmes vecteurs de test, mêmes
commandes. À parité avec le Rust depuis le 2026-11-20 (fond plein écran, logo de démarrage,
images courantes, `copy` et économiseur du firmware custom). Sources : `driver/c/`.

| Cible CMake | Rôle | Équivalent Rust |
|-------------|------|-----------------|
| `hsm01_proto` (statique, sans libusb) | `include/hsm01/proto.h` : framing + CRC16, commandes (dont palette 0x33, tuile de fond 0x37, logo 0x50), RLE, parsing des reports | `hsm01-proto` (`frame`, `cmd`, `rle`, `input`) |
| `hsm01_proto` (suite) | `include/hsm01/background.h` : fond 480×272 indexé, 32 tuiles entrelacées, réduction à 256 couleurs par coupe médiane **identique au Rust** (mêmes palettes et pixels), reconstruction progressive | `hsm01-proto::background` |
| `hsm01_proto` (suite) | `include/hsm01/free.h` : protocole « écran libre » du firmware custom (FILL / BLIT / BLIT_RLE / COPY 240.21 / rétro / LEDs), reports bruts avec `screen_epoch` (240.22), décodeur molettes (512 pas/tour) | `hsm01-proto::free` |
| `hsm01` (statique) | `include/hsm01/usb.h` : init, vendor, isochrone (libusb async + `handle_events_completed`), interrupt, keep-alive pthread, ack auto | `hsm01-usb` |
| `hsm01` (suite) | `include/hsm01/free_usb.h` : firmware custom (240.xx) – bulk OUT, reports interrupt, reboot DFU (`HSM01_ERR_WRONG_FW` sur le firmware officiel) | `hsm01-usb::free` |
| `hsm01_image` (statique, non installée) | `tools/image.h` : images de l'outil — PNG, JPEG, GIF (1re image) par libpng / libjpeg / giflib, BMP (8, 24, 32 bits) et PPM lus à la main ; mises à l'échelle en gardant les proportions pour remplir 480×272 (Lanczos 3), centre recadré | `load_image` du CLI (crate `image`) |
| `hsm01-c` | CLI : `info`, `monitor`, `demo`, `brightness`, `led`, `background`, `splash` + `fw info\|fill\|image\|backlight\|leds\|monitor\|bench\|dfu` | `hsm01` |
| `test_proto`, `test_free`, `test_background`, `test_image` | tests CTest : trames officielles reconstruites à l'octet près, RLE, reports, protocole libre (dont COPY et `screen_epoch`), décodeur molettes ; fond (tuiles, reconstruction, palettes exactes, longueurs de commandes, **empreintes FNV-1a identiques à celles du test Rust** `same_result_as_the_c_port` sur deux images) ; images (chaque format écrit puis relu, mise à l'échelle et recadrage, erreurs) | tests des crates |

## Outil `hsm01-c`
```
hsm01-c info | monitor [secs] | demo [secs] | brightness <0-100> | led <0-3> <off|on|blink>
hsm01-c background <image> [secs]   # fond plein écran (RAM seulement), 15 s par défaut
hsm01-c splash <image> --yes        # écrit le logo de démarrage en flash
hsm01-c splash default --yes        # remet le logo d'usine
hsm01-c fw info | fill <r> <g> <b> | image <image> | backlight <0-100> | leds <a> <b> <c> <d>
hsm01-c fw monitor [secs] | bench [frames] | dfu
```
Images : PNG, JPEG, GIF, BMP, PPM (la liste exacte selon les bibliothèques trouvées à la
compilation est rappelée par les messages d'erreur). `background` / `splash` : palette puis 32
tuiles, une par trame, 40 ms entre deux en lisant les reports (une tuile refusée par l'appareil
arrête l'envoi avec son code d'erreur) ; `splash` suit l'écriture en flash (progression 0-100 %,
0xfe = échec, erreur 0x100 = fond incomplet) et refuse de démarrer sans `--yes`. Firmware custom
240.22 et plus (économiseur d'écran) : `fw fill / image / backlight / leds` gardent l'image en
lisant les reports jusqu'à Ctrl+C, comme le Rust.

## Build
```sh
cmake -S driver/c -B driver/c/build -DCMAKE_BUILD_TYPE=Release -DHSM01_WERROR=ON
cmake --build driver/c/build && ctest --test-dir driver/c/build
```
Dépendances : C11, pthreads, `libusb-1.0` (`-DHSM01_BUILD_USB=OFF` pour ne construire que le
protocole) ; pour les images, `libpng`, `libjpeg` (turbo) et `giflib` si présentes (paquets
`libpng-dev libjpeg-dev libgif-dev` sous Debian / Ubuntu) — sans elles, BMP et PPM seulement ;
`-DHSM01_REQUIRE_IMAGE_LIBS=ON` fait échouer la configuration s'il en manque une (CI).
Avertissements : `-Wall -Wextra -Wpedantic -Wshadow -Wconversion`, sans erreur avec gcc et clang.
Accès non-root : règle `packaging/udev/70-hercules-stream100.rules`.

## API (exemple)
```c
#include <hsm01/usb.h>
hsm01_device *d;
if (hsm01_open(&d) == HSM01_OK) {
    hsm01_start_keepalive(d);                 /* obligatoire tant que l'on pilote le device */
    hsm01_buf b; hsm01_buf_init(&b);
    hsm01_cmd_screen_on(&b);
    hsm01_cmd_screen_brightness(&b, 80);
    hsm01_cmd_led(&b, 0, HSM01_LED_BLINK);
    hsm01_send(d, &b);                        /* ajoute la fin de liste 0x00 */
    hsm01_report r;
    while (hsm01_poll(d, 100, &r) >= 0)
        if (r.has_controls) printf("buttons %04x\n", r.controls.buttons);
    hsm01_buf_free(&b);
    hsm01_close(d);
}
```
Fond d'écran : `hsm01_quantize(argb, img)` (480×272 ARGB8888 → `hsm01_indexed_image`, à allouer),
puis `hsm01_cmd_palette` et, pour chaque tuile, `hsm01_bg_tile` + `hsm01_cmd_background_tile`
dans sa propre trame ; logo : `hsm01_cmd_splash_flash(b, true)` une fois les 32 tuiles reçues.

Volontairement absents : commandes usine (0x20-0x22), reboot DFU du firmware officiel (0xF2).
L'écriture du logo (0x50) n'est faite que sur demande explicite (`--yes`) et reste réversible
(`splash default`).

## Release
Tag `c-vX.Y.Z` → `.github/workflows/release-c.yml` : gcc + clang, `-Werror`, bibliothèques
d'images exigées, CTest, archives `hsm01-c-<ver>-linux-{x86_64,aarch64}.tar.gz` (binaire, libs,
en-têtes, règle udev) + sha256. Le binaire utilise à l'exécution `libusb-1.0`, `libpng16`,
`libjpeg` et `libgif` du système.
