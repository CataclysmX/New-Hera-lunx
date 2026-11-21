# Index docs

Démarche complète pour tout refaire soi-même (outils, commandes) : [../explanation/00-sommaire.md](../explanation/00-sommaire.md).

## Protocole (Stream 100 = "HSM01")
- [protocol/usb.md](protocol/usb.md) – descripteurs, endpoints, séquence d'init, IOCTL Windows
- [protocol/framing.md](protocol/framing.md) – trames `HERCULES`+`SM`, CRC16, reports IN `sm` ✔ validé
- [protocol/commands.md](protocol/commands.md) – opcodes hôte→device (écran, LEDs, VU, images)
- [protocol/inputs.md](protocol/inputs.md) – boutons/encodeurs, statut device
- [protocol/live-tests.md](protocol/live-tests.md) – validations sur le vrai device (Linux)

## Drivers Linux
- [driver/rust.md](driver/rust.md) – implémentation Rust (`driver/rust/`), CLI `hsm01`, release CI
- [driver/c.md](driver/c.md) – implémentation C (`driver/c/`), CLI `hsm01-c`, release CI

## Application officielle
- [app/integrations.md](app/integrations.md) – plugins (audio, OBS, Twitch, Discord, Spotify, Hue…) et équivalents Linux ; Discord en détail : [app/discord.md](app/discord.md) (9 options, faisabilité, **module Discord** : mise en route, page, commandes, faux client de test)
- [app/firmware-officiel.md](app/firmware-officiel.md) – **étude** : rendre l'app compatible avec le firmware d'origine (disposition fixe de son écran, chaque fonction faisable / dégradée, options, travail à refaire, points à vérifier)
- [app/telemetrie.md](app/telemetrie.md) – **modules Motor Town et SimHub** : tableau de bord de jeu commun (`hsm01/dashboard.py`), télémétrie UDP de Motor Town (formats natif et Forza), SimHub (flux compatible du jeu ou API web du programme), faux jeux de test
- [app/scores.md](app/scores.md) – **module Scores en direct** : flux des directs de L'Équipe, matchs en cours par sport, filtre (sports, compétitions, équipes), lecture toutes les minutes ou 5 min, page, widget, pilules, faux flux de test
- [app/obs.md](app/obs.md) – **module OBS** : serveur WebSocket d'OBS (port et mot de passe lus dans ses réglages), scènes, mixeur et vumètres sur les molettes, direct / enregistrement, commandes, faux OBS de test
- [app/spotify.md](app/spotify.md) – **module Spotify** : mise en route (application Spotify Developer), page, commandes, radio, listes de lecture, faux Spotify de test
- [app/a-faire.md](app/a-faire.md) – **reprise** : à tester et reste à faire (note du 2026-11-20)
- [app/companion.md](app/companion.md) – application compagnon `hsm01-app` (firmware custom) : architecture, config TOML, état
- [../app/clients/python/README_FR.md](../app/clients/python/README_FR.md) – **bibliothèque Python `hsm01`** des modules, guide complet pour en créer (en anglais : `README.md` du même dossier)
- [app/modules.md](app/modules.md) – **créer un module** : dossier `~/.local/share/hsm01/modules/<nom>/`, `module.toml`, modèle Python, autres langages, partage
- [app/stream-deck.md](app/stream-deck.md) – **mode stream deck** : widget `deck` (une piste par molette), module « Stream deck » (4 pistes, lignes/colonnes) ; test de la maquette `concept/` ; idées d'éditeur
- [app/api.md](app/api.md) – **API ouverte** (référence) : socket Unix / WebSocket, messages JSON, `hsm01ctl`, bibliothèque Python, exemples
- [app/api-externe.md](app/api-externe.md) – étude : API ouverte (modules dans n'importe quel langage, l'app = écran optimisé), modules proposés
- [app/scripts.md](app/scripts.md) – étude : scripts utilisateur (styles, pages, menus programmables) ; suite : [API](app/scripts-api.md), [briques de menus](app/scripts-menus.md)

## Firmware custom
- [firmware/hardware.md](firmware/hardware.md) – horloges, écran LTDC, broches, PWM, entrées
- [firmware/stage1.md](firmware/stage1.md) – palier 1 : trappe DFU + mire, flash et retour arrière
- [firmware/protocol.md](firmware/protocol.md) – protocole « écran libre » (bulk + reports d'entrée)
- [firmware/stage2.md](firmware/stage2.md) – palier 2 : firmware écran libre, pièges, résultats
- [firmware/knobs.md](firmware/knobs.md) – molettes double curseur : signal, algorithme officiel, décodeur hôte

## Reverse engineering
- [re/artifacts.md](re/artifacts.md) – contenu de l'installeur, rôle de chaque binaire
- [re/dll-map.md](re/dll-map.md) – fonctions clés de `hsm_api_core_x64.dll`
- [re/tools.md](re/tools.md) – scripts, régénération de `re_work/`
- [re/status.md](re/status.md) – ce qui est établi / reste à faire
- [re/firmware-update.md](re/firmware-update.md) – versions, MAJ firmware, Wine/VM, firmware custom
- [re/firmware-hidden.md](re/firmware-hidden.md) – fonctions cachées (overlay debug, mode autonome, DFU…)

## Historique
Scripts racine (`*.py`, `server*.js`, `C/wake_hercules.c`) = essais antérieurs de rejeu de captures.
Erreur connue : ils traitent les octets 4-7 du SM comme un timestamp ; c'est **CRC16 + n° de séquence**
(cf. framing.md), et 0x14 fait 1 octet (pas 6).
