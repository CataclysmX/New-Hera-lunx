# hsm01 – driver Rust (Hercules Stream 100, Linux)

Driver 100 % userspace (libusb), aucun module noyau. Workspace : `driver/rust/`.

| Crate | Rôle |
|-------|------|
| `hsm01-proto` | protocole pur, sans I/O : framing `HERCULES`/`SM` + CRC16 (`frame`), commandes (`cmd`), images RLE (`rle`), reports IN (`input`). Testé contre des trames capturées de l'app officielle (reconstruction octet par octet). |
| `hsm01-usb` | transport : init (config 1, interfaces 0/1, alt 1), requêtes vendor, écriture **isochrone** (FFI libusb, rusb ne l'expose pas), lecture interrupt, keep-alive 500 ms, ack automatique des infos device. |
| `hsm01-cli` | binaire `hsm01` |

## Installation
```sh
sudo cp 70-hercules-stream100.rules /etc/udev/rules.d/   # accès non-root (uaccess)
sudo udevadm control --reload && sudo udevadm trigger
```
Build depuis les sources : `cd driver/rust && cargo build --release` (dépend de `libusb-1.0`).

## Utilisation
```
hsm01 info                 # firmware, n° de série, infos device (écran 480x272, 9 boutons, 4 LEDs)
hsm01 monitor [secs]       # événements boutons / molettes / écran
hsm01 demo [secs]          # mire : icônes, labels, chenillard LEDs, VU-mètres animés
hsm01 brightness <0-100>
hsm01 led <0-3> <off|on|blink>
hsm01 background <image> [secs]   # fond plein écran 480x272 (RAM seulement, aucun effet persistant)
hsm01 splash <image> --yes        # écrit le logo de démarrage en flash
hsm01 splash default --yes        # restaure le logo d'usine
```
Avec le firmware custom (`firmware/rust`, version 240.xx, [../firmware/stage2.md](../firmware/stage2.md)) :
```
hsm01 fw info | fill <r> <g> <b> | image <file> | backlight <0-100> | leds <a> <b> <c> <d>
hsm01 fw monitor [secs]    # boutons + pas des molettes (décodeur hsm01_proto::free::knob)
hsm01 fw bench [frames]    # débit plein écran
hsm01 fw dfu               # reboot vers le bootloader ROM
```
Images : PNG/JPEG/BMP/GIF, redimensionnées + recadrées en 480×272, quantifiées en 256 couleurs
(median-cut, `hsm01-proto::background`). Le port C ([c.md](c.md)) donne les mêmes palettes et
pixels : le test `same_result_as_the_c_port` et `driver/c/tests/test_background.c` vérifient les
mêmes empreintes FNV-1a sur deux images.

## API (exemple)
```rust
use hsm01_proto::cmd::{Command, LedState};
let dev = std::sync::Arc::new(hsm01_usb::Device::open()?);
let _ka = dev.spawn_keepalive();                  // obligatoire tant que l'on pilote le device
dev.send(&[Command::ScreenOn, Command::ScreenBrightness(80),
           Command::Led { index: 0, state: LedState::Blink }])?;
while let Some(report) = dev.poll(std::time::Duration::from_millis(100))? {
    if let Some(c) = report.controls { println!("{:?}", c); }
}
```
Volontairement absents : commandes usine (0x20-0x22), reboot DFU (0xF2). L'écriture du logo
(0x50) n'est faite que sur demande explicite (`--yes`) et reste réversible (`splash default`).

## Release
Tag `rust-vX.Y.Z` → workflow `.github/workflows/release-rust.yml` : fmt, clippy `-D warnings`,
tests, build release x86_64 + aarch64, archive `hsm01-rust-<ver>-<target>.tar.gz` (+ sha256) publiée.
