# 08 – Les drivers Linux (Rust, puis C)

Aucun module noyau n'est nécessaire : l'appareil est en classe « vendor », **libusb** suffit (avec la
règle udev du fichier 01). Les drivers sont en espace utilisateur.

## 1. Principe de conception

- **Une bibliothèque « protocole » pure** (aucun accès USB) : trames, CRC, commandes, images RLE,
  analyse des entrées. Elle se teste sans appareil, avec les **trames capturées** comme vecteurs de
  test (on vérifie qu'on reconstruit exactement les octets du logiciel officiel).
- **Une bibliothèque « transport »** (USB) : init, requêtes vendor, écriture isochrone, lecture
  interrupt, keep-alive.
- **Un outil en ligne de commande** pour tout essayer.

Rust d'abord (`driver/rust/` : `hsm01-proto`, `hsm01-usb`, `hsm01-cli`), puis un portage C
(`driver/c/`) avec les mêmes tests.

## 2. Rust

```sh
cd driver/rust
cargo fmt
cargo clippy --all-targets -- -D warnings      # zéro avertissement toléré
cargo test                                      # tests (dont reconstruction des trames capturées)
cargo build --release
./target/release/hsm01 info                     # firmware, n° de série, infos
./target/release/hsm01 monitor 30               # boutons / molettes / écran
./target/release/hsm01 demo 20                  # icônes, labels, LEDs, VU-mètres animés
./target/release/hsm01 brightness 80
./target/release/hsm01 led 0 blink
./target/release/hsm01 background image.png 15  # fond d'écran (RAM seulement)
cd ../..
```

Particularité : `rusb` ne sait pas faire d'isochrone ; on appelle directement libusb (FFI) pour ces
transferts. Avec le firmware custom (fichier 09), les commandes `hsm01 fw …` utilisent le transfert
bulk, beaucoup plus simple.

## 3. C (CMake)

```sh
cd driver/c
cmake -S . -B build -DHSM01_WERROR=ON -DCMAKE_BUILD_TYPE=Release
cmake --build build -j
ctest --test-dir build --output-on-failure
./build/hsm01-c info
# vérifier avec les deux compilateurs :
for cc in gcc clang; do
  rm -rf build-$cc
  cmake -S . -B build-$cc -DCMAKE_C_COMPILER=$cc -DHSM01_WERROR=ON -DCMAKE_BUILD_TYPE=Release
  cmake --build build-$cc -j && (cd build-$cc && ctest --output-on-failure)
done
cd ../..
```

## 4. Publication (GitHub Actions)

`.github/workflows/release-rust.yml` (tag `rust-v*`) et `release-c.yml` (tag `c-v*`) : formatage,
lint, tests, compilation x86_64 + aarch64, archives + sha256 attachées à la release.

```sh
python3 -I tools/check_yaml.py .github/workflows/release-rust.yml .github/workflows/release-c.yml
git tag rust-v0.1.0 && git push origin rust-v0.1.0      # déclenche la release
```

## 5. Pièges rencontrés

- Le firmware officiel exige un **keep-alive** toutes les 500 ms et l'**acquittement** (`0x14`) des
  messages d'info, sinon il considère que l'hôte est parti (il affiche son logo puis éteint l'écran).
  Il faut donc **lire en continu** les rapports, même quand on n'attend rien.
- En mode hôte, l'écran officiel reste au rétroéclairage 0 tant qu'on n'envoie pas `ScreenBrightness`.
- Les champs d'une requête de contrôle (`wValue` / `wIndex`) se vérifient dans le code qui l'émet
  (driver Windows), pas à l'intuition.
