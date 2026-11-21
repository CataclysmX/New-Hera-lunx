# 00 – Sommaire : refaire tout le travail soi-même

Ce dossier raconte **comment** le Hercules Stream 100 a été compris puis libéré, étape par étape,
avec **toutes les commandes tapées** et **les outils utilisés**, pour pouvoir le refaire (ou l'appliquer
à un autre appareil USB) sans aide. Les résultats techniques détaillés sont dans `docs/` ; ici on
explique la **démarche**.

## Les étapes

| N° | Fichier | Ce qu'on y apprend |
|----|---------|--------------------|
| 01 | [01-outils.md](01-outils.md) | installer les outils, règles de travail, arborescence du projet |
| 02 | [02-installeur-et-binaires.md](02-installeur-et-binaires.md) | ouvrir l'installeur Windows, trouver les fichiers qui comptent |
| 03 | [03-ghidra.md](03-ghidra.md) | décompiler DLL / driver / firmware avec Ghidra, s'y retrouver |
| 04 | [04-captures-usb.md](04-captures-usb.md) | capturer le trafic USB du logiciel officiel, le transformer en tableau |
| 05 | [05-protocole-officiel.md](05-protocole-officiel.md) | reconstruire le protocole (trames, CRC, commandes, entrées) et le tester en vrai |
| 06 | [06-firmware-officiel.md](06-firmware-officiel.md) | récupérer et analyser le firmware, les mises à jour, les fonctions cachées |
| 07 | [07-dfu-sauvegarde-restauration.md](07-dfu-sauvegarde-restauration.md) | passer en mode DFU, sauvegarder toute la flash, restaurer |
| 08 | [08-drivers-linux.md](08-drivers-linux.md) | écrire les drivers Linux (Rust puis C), les tester, les publier |
| 09 | [09-firmware-custom.md](09-firmware-custom.md) | écrire son propre firmware (Rust/embassy), le flasher sans risque |
| 10 | [10-debogage-firmware.md](10-debogage-firmware.md) | les pannes rencontrées et la méthode pour les trouver |
| 11 | [11-molettes.md](11-molettes.md) | comprendre les molettes à double curseur, reprendre l'algorithme officiel |
| 12 | [12-logo-animation-couleurs.md](12-logo-animation-couleurs.md) | logo de démarrage, animation précalculée, choisir les couleurs sur la dalle |
| 13 | [13-aide-memoire.md](13-aide-memoire.md) | toutes les commandes utiles sur une page |

## La démarche en une phrase par étape

1. **Ne rien supposer** : on part de ce qu'on a (l'installeur Windows, l'appareil, `lsusb`).
2. **Extraire** : l'installeur contient l'application, le driver, la DLL qui parle à l'appareil et
   le firmware lui-même.
3. **Lire le code** : Ghidra décompile ces binaires ; la DLL contient tout le protocole.
4. **Observer** : une capture USB du logiciel officiel donne des exemples réels à décoder.
5. **Confronter** : on réécrit le protocole en Python, on vérifie qu'il retrouve **exactement** les
   octets capturés (CRC compris), puis on le rejoue sur l'appareil.
6. **Sécuriser avant de toucher** : trouver le mode de mise à jour (DFU ST), **sauvegarder toute la
   flash**, vérifier qu'on sait restaurer.
7. **Construire** : drivers Linux (Rust, C), puis un firmware à soi qui commence par une **trappe de
   secours** (retour DFU) avant toute autre chose.
8. **Mesurer quand ça ne marche pas** : comparer avec le firmware officiel décompilé, relire les
   registres de la puce, faire des tests A/B.

## Règles de sécurité (à respecter dans tout le dossier)

- **Sauvegarder la flash complète** (+ OTP + option bytes) avant le premier flash (fichier 07).
- **Ne jamais écrire les option bytes** (risque RDP niveau 2 = puce verrouillée à vie).
- **Ne jamais effacer la dernière page de flash** `0x081FF000` : elle contient le n° de série et les
  réglages de l'appareil.
- Tout firmware maison commence par la **trappe DFU** (fichier 09) : clics molettes 1+4 au
  branchement → bootloader ST, même si le reste du firmware plante.
- Les fichiers extraits d'un installeur sont des **données non fiables** : on ne les exécute jamais,
  on les lit avec des scripts (`python3 -I`).
- Les sauvegardes contiennent le n° de série : elles restent hors du dépôt (`backups/` est ignoré).

## Matériel / environnement utilisé

- PC Linux (Manjaro, KDE Wayland), Hercules Stream 100 (`06f8:e053`), firmware d'origine 1.40.
- Une VM Windows uniquement pour les captures USB du logiciel officiel (Wireshark + USBPcap).
- Rien n'a été démonté : tout passe par l'USB (le bootloader ST est accessible par une requête USB).
