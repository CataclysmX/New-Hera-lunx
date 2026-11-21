# 01 – Les outils et l'organisation du travail

## Installation (Manjaro / Arch)

```sh
# analyse de binaires
sudo pacman -S --needed p7zip binutils xxd file python python-pyusb libusb
pamac build ghidra            # Ghidra (AUR) – installé dans /opt/ghidra, nécessite Java (jdk21)
sudo pacman -S --needed qt6-tools     # lconvert6 : lire les traductions Qt (.qm) de l'app officielle

# USB
sudo pacman -S --needed usbutils dfu-util wireshark-qt   # lsusb, dfu-util, Wireshark (usbmon)

# compilation
sudo pacman -S --needed rustup cmake gcc clang pkgconf llvm   # llvm : llvm-objcopy / llvm-objdump
rustup default stable
rustup target add thumbv7em-none-eabihf     # cible ARM Cortex-M4F (STM32L4) pour le firmware
```

Vérifications faites au départ :

```sh
which ghidra 7z dfu-util tshark         # ce qui est disponible
lsusb | grep -i -E "hercules|06f8"      # l'appareil est-il vu ?
lsusb -v -d 06f8:e053 | head -250       # descripteurs USB complets
readlink -f /usr/bin/ghidra             # où est installé Ghidra (→ /opt/ghidra)
ls /opt/ghidra/support/                 # contient analyzeHeadless (mode sans interface)
```

## Accès USB sans root

Règle udev (fichier `packaging/udev/70-hercules-stream100.rules` du dépôt) :

```
SUBSYSTEM=="usb", ATTR{idVendor}=="06f8", ATTR{idProduct}=="e053", MODE="0660", TAG+="uaccess"
SUBSYSTEM=="usb", ATTR{idVendor}=="0483", ATTR{idProduct}=="df11", MODE="0660", TAG+="uaccess"
```

```sh
sudo cp packaging/udev/70-hercules-stream100.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

La 2ᵉ ligne couvre le **mode DFU** (bootloader ST `0483:df11`) : `dfu-util` marche alors sans `sudo`.

## Organisation du dépôt

```
docs/        résultats (protocole, matériel, firmware, drivers) – à lire après ce guide
tools/       scripts « appareil » (tests live, flash, mesures)
tools/re/    scripts d'analyse (installeur, PE, captures, firmware) + scripts Ghidra (.java)
re_work/     tout ce qui est extrait / décompilé (régénérable, jamais versionné)
backups/     sauvegardes flash de l'appareil (n° de série dedans : jamais versionné)
driver/rust  driver Linux Rust (bibliothèques + CLI hsm01)
driver/c     portage C (CMake, CLI hsm01-c)
firmware/    firmware custom (rust/) + tests hôte du parseur (proto-tests/)
public/      images (logo de démarrage)
```

## Règles de travail qui ont évité des erreurs

- **Un script par tâche**, rangé dans `tools/` ou `tools/re/`, plutôt que des commandes jetables :
  on peut tout relancer, et les scripts documentent eux-mêmes ce qu'ils font (`Usage:` en tête).
- **`python3 -I script.py`** : mode isolé, Python n'importe rien depuis le dossier courant (un
  fichier extrait nommé `json.py` ne peut pas se faire exécuter).
- **Tout ce qui est extrait est non fiable** : lu, jamais lancé.
- **Écrire les découvertes au fur et à mesure** dans de petits `.md` (`docs/`), avec la preuve
  (capture, fonction Ghidra, test live) et le niveau de certitude (✔ / ◐ / ✘).
- **Chaque hypothèse est confrontée** : à une capture, au code décompilé, puis à l'appareil.
