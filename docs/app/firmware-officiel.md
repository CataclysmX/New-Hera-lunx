# Étude : rendre l'app compatible avec le firmware d'origine (1.40 / 1.42)

État (2026-11-21) : l'app n'accepte que le firmware custom 240.xx ; sur le firmware d'origine elle
s'arrête avec « l'app demande le firmware custom ». Les pilotes, eux, savent déjà tout piloter
(`hsm01-usb::Device` en Rust, `usb.h` en C : isochrone, keep-alive, ack, commandes, fond par
tuiles, `hsm01 demo` validé à l'écran). Cette étude dit ce que le firmware d'origine permet, ce que
chaque fonction de l'app devient, les options possibles et le travail à refaire.

## 1. Ce que permet le firmware d'origine

Le firmware compose lui-même l'écran, ligne par ligne (boucle de rendu `FUN_0801075c` de la 1.42,
décompilée avec Ghidra ; zones de la ligne de séparation : `FUN_0800c5f0` ; fonds : `FUN_0800b6f0`),
dans une **disposition fixe en 4 colonnes de 120 px** :

| Zone (y) | Contenu | Commande | Mise à jour |
|----------|---------|----------|-------------|
| 0-79 | rien que le fond | – | – |
| 80-199 | **2 barres VU par colonne** (8 en tout) : niveau, crête, écrêtage rouge | `0x34` style (3 couleurs + dégradé), `0x40` géométrie, `0x41` niveau | 6 o par barre : à chaque image |
| 208 | ligne de séparation (selon le style de la zone d'action) | `0x38` `azStyle` 0-5, `azRGB` | réglage |
| 208-271 | séparateurs verticaux à x = 120, 240, 360 (selon le style) | idem | réglage |
| 220-251 | **icône 32×32** par colonne, x = 44 + 120 × canal | `0x35` (RLE alpha 4 bits + RGB565), 2 banques | ≤ 3 Ko : à chaque image |
| 254-269 | **libellé 110×16** par colonne, x = 5 + 120 × canal | `0x36` (même RLE), 2 banques | ≤ 5 Ko : à chaque image |
| tout | **fond** : préréglages (types 1-6, 100), dégradé (`darkest` → `lightest`), ou **image 480×272 en 256 couleurs** | `0x38` `bkgType`, `0x33` palette, `0x37` 32 tuiles | image : **≈ 1,3 s** (32 × 4 Ko, une tuile toutes les 40 ms) |

Particularités utiles du fond :
- les 32 tuiles sont **entrelacées** : la tuile 0 seule peint tout l'écran en blocs 8×4, chaque
  tuile suivante l'affine. Un nouveau fond apparaît donc flou **tout de suite**, puis net ;
- il n'y a **pas de mise à jour partielle** : chaque tuile couvre tout l'écran, une petite zone
  modifiée oblige à tout renvoyer ;
- la **palette** (`0x33`, 256 entrées RGB565) se change à part, en quelques octets : on peut
  recolorer, faire clignoter, afficher ou masquer **instantanément** tout ce qui a été dessiné avec
  des couleurs de palette réservées.

Le reste :
- **LEDs** : allumée, éteinte, clignotante (`0x30`) ; une seule intensité pour les 4 (`0x39`). Pas
  d'intensité par LED (le custom en a une en %).
- **Écran** : allumé / éteint (`0x12` / `0x13`), luminosité (`0x31`). Logo de démarrage modifiable
  (`0x50`, avec le fond courant).
- **Entrées** : mêmes bits de boutons ; molettes en accumulateurs déjà décodés (512 pas par tour,
  le même sens et la même unité que le décodeur de l'app) ; clics, maintiens et doubles appuis à
  calculer côté hôte comme aujourd'hui.
- **Comportements imposés** :
  - le bouton **power** agit lui-même (court : luminosité +20 % en boucle ; long : écran éteint) ;
    l'app reçoit aussi l'appui, mais ne peut pas empêcher l'effet ;
  - sans trame de l'hôte pendant ~1 900 images, le firmware passe en **mode autonome** (ses
    propres fonds de démonstration) ;
  - il faut un **keep-alive** toutes les 500 ms et acquitter le message d'infos (`0x14`) ;
  - le combo clic molette 1 + double appui bouton 4 affiche un écran de diagnostic.
- **Débit** : isochrone, un message SM de 8,5 Ko au plus par envoi.

Le firmware n'a ni texte libre, ni rectangle, ni image placée où l'on veut : **tout ce qui n'est
pas une icône, un libellé ou un VU-mètre doit être cuit dans le fond**.

## 2. Ce dont l'app dépend aujourd'hui (firmware custom)

- écran libre : tout est dessiné par l'app (`render.rs`) puis envoyé par petites tuiles 16×16
  modifiées (`screen.rs` : BLIT / BLIT_RLE / FILL, COPY pour les défilements, 60 images/s) ;
- entrées brutes des curseurs des molettes, décodées par l'app (`free::knob`) ; `screen_epoch`
  (économiseur 240.22) ;
- rétroéclairage 0-100 % (veille en fondu), LEDs à intensité libre ;
- connexion en bulk (`FreeDevice`), sans keep-alive ni message d'infos.

## 3. Chaque fonction de l'app sur le firmware d'origine

| Fonction | Firmware d'origine | Comment |
|----------|--------------------|---------|
| Page Audio (4 molettes : nom, VU, volume) | **oui, natif** | icône de l'appli (thème, 32×32), libellé (nom ou « 55 % »), barre VU 1 = niveau, barre VU 2 ou marqueurs `0x40` = volume ; c'est l'usage prévu par Hercules |
| Changement de page | oui | fond de la nouvelle page (flou immédiat, net en ~1,3 s) + icônes, libellés, VU |
| Titre de page, textes fixes, images, cadres, séparateurs | dégradé | dessinés dans le fond, renvoyés seulement quand ils changent (avec un délai de regroupement) |
| Textes qui changent souvent (variables `{volume}`, heure, titre du morceau) | dégradé | dans les libellés 110×16 s'ils tiennent au bas d'une colonne ; sinon dans le fond, ≥ 1 s de retard |
| Texte qui défile (noms longs) | oui, dans un libellé | le défilement est redessiné dans le libellé à chaque image |
| Jauges, VU-mètres libres, styles (arc, aiguille, segments) | non (sauf les 8 barres) | remplacés par les barres natives, à leur place fixe |
| Analyseur de spectre | dégradé | 8 bandes sur les 8 barres VU |
| Image de fond PNG / JPEG | oui | fond 256 couleurs ; GIF animé : non (1re image) |
| Assombrir le fond, étalonnage des couleurs | oui | appliqués avant la réduction en 256 couleurs |
| Notifications, alertes, pilules | dégradé | fond temporaire (flou puis net) ou libellés + LED clignotante ; retour au fond de la page ensuite (≈ 2 × 1,3 s) |
| Menu des modules (bandeau animé) | dégradé | liste dans les 4 libellés (molette 1 = choisir), sans animation ; ou fond « menu » |
| Stream deck, menus qui défilent (COPY) | dégradé | 4 cases = 4 colonnes (icône + libellé), la molette change de rangée ; pas de glissement |
| Pages de modules (API, widgets libres) | dégradé | cuites dans le fond, rafraîchies au plus une fois par seconde ; une API de « cases natives » (icône, libellé, VU par colonne) à ajouter pour les données vives |
| Widgets de modules dans une page (membres Discord…) | dégradé | dans le fond ; liste qui défile : non |
| Tableaux de bord de jeu (Motor Town, SimHub) | dégradé | rapport, vitesse, régime dans les libellés / barres VU (régime = barre), LEDs compte-tours en on / clignotant |
| Veille | oui, sans fondu | `0x13` écran éteint ; réveil `0x12` + luminosité |
| Luminosité, « définir » / « cycle » | oui | `0x31` ; le bouton power la change aussi tout seul |
| LEDs (états, intensités) | dégradé | allumée si ≥ 50 %, clignotement natif ; intensité commune |
| Actions du bouton power | gênées | le firmware change la luminosité (court) ou éteint l'écran (long) en même temps |
| Logo de démarrage | **mieux** | réglable depuis l'app (`0x50`, réversible) ; le custom a un logo compilé |
| Écran « SIGNAL NOT FOUND » à l'arrêt | non | le firmware d'origine passe en mode autonome (ses fonds) |

## 4. Options

**A. Mode « mixeur » natif.** Un modèle de page propre au firmware d'origine : 4 colonnes (icône,
libellé, 2 barres VU, LED) + un fond (préréglage, dégradé ou image). Les pages actuelles sont
traduites colonne par colonne d'après la cible de chaque molette. Rapide (tout à 60 images/s sauf
le fond), fiable, proche du logiciel Hercules ; mais l'éditeur libre ne sert presque plus.

**B. Projection du rendu libre.** Garder `render.rs` tel quel et envoyer chaque image comme fond
(256 couleurs) dès qu'elle change. Aucun travail sur les pages, mais **≥ 1,3 s par changement**,
défilements et animations impossibles, et ~130 Ko à chaque mise à jour : inutilisable pour les VU
ou les molettes.

**C. Hybride (conseillé).** A pour tout ce qui bouge (icônes, libellés, VU, LEDs), B pour le décor
fixe de la page (titre, images, cadres, textes peu changeants) cuit dans le fond et renvoyé
seulement quand il change ; astuces de palette pour la sélection et les clignotements.
L'éditeur garde ses widgets ; ceux qui ne peuvent pas être vivants sont marqués « fixe sur le
firmware d'origine » dans l'inspecteur, et l'aperçu montre la vraie disposition (zones des icônes,
libellés et barres).

**D. Proposer le firmware custom.** Détecter le firmware d'origine et proposer, dans la fenêtre,
de passer au firmware custom (sauvegarde, entrée DFU par la requête `0xF2`, `dfu-util`), le retour
au firmware d'origine restant possible. Peu de travail, toutes les fonctions ; mais flasher fait
peur et reste à la charge de l'utilisateur. **À combiner avec C** : C pour qui ne veut pas flasher,
D pour qui veut tout.

## 5. Travail à refaire (pour C + D)

1. **Couche appareil à deux moteurs** (moyen) : une interface commune (envoyer, lire les entrées,
   luminosité, LEDs, veille) avec `FreeDevice` (custom) et `Device` (d'origine : isochrone,
   keep-alive, ack `0x14`, statut, erreurs `lastInvalidCmd`) ; détection du firmware à la
   connexion au lieu du refus actuel.
2. **Entrées d'origine** (petit) : deltas des accumulateurs (`encoder_delta`) au lieu du décodeur
   des curseurs ; mêmes boutons ; filtre « clic qui fait bouger la molette » ; power : resynchroniser
   la luminosité de l'app sur l'octet d'état (bits 0-6) après un appui.
3. **Rendu natif** (gros) : traduire une page en 4 colonnes (icône du thème ou image → RLE 32×32,
   texte → libellé 110×16 avec la police de l'app, VU / volume → barres `0x41` / `0x40`, styles
   `0x34` depuis les couleurs du widget) ; n'envoyer que ce qui change ; LEDs → `0x30` / `0x39`.
4. **Fond cuit** (moyen) : rendre la page sans ses éléments vivants, réduire en 256 couleurs
   (`hsm01-proto::background`), envoyer palette + tuiles au changement de page ou quand le décor
   change (regroupé, au plus ~1 fois / s) ; couleurs de palette réservées pour sélection et
   clignotements.
5. **Alertes et menus** (moyen) : alerte → fond temporaire + LED, retour automatique ; menu des
   modules et stream deck → libellés / icônes des 4 colonnes.
6. **API** (moyen) : requête de « cases natives » par colonne pour les modules ; leurs pages libres
   cuites dans le fond à cadence limitée ; prévenir les modules du mode (`hello` : `firmware:
   "origine"`) pour qu'ils simplifient leur page.
7. **Fenêtre** (moyen) : aperçu fidèle au firmware d'origine, marquage des widgets « fixes »,
   réglages propres (préréglage / dégradé de fond, style de zone d'action, logo de démarrage),
   assistant de passage au firmware custom (option D).
8. **Tests** (moyen) : simulateur du firmware d'origine (composition 4 colonnes d'après les
   coordonnées ci-dessus) pour les tests et l'aperçu ; essais sur l'appareil.

## 6. À vérifier sur l'appareil (firmware d'origine reflashé) avant de coder

- **cadence réelle des tuiles** : plus vite que 40 ms par tuile ? (requête vendor `0x0e` = place
  libre dans la file isochrone, utile pour régler le débit sans perte) ;
- rôle des **deux banques** d'icônes / libellés (la DLL envoie les deux : double tampon probable) ;
- réglages par canal **`0x32`** (6 octets, la 1.42 en dérive des drapeaux d'affichage) et
  géométrie **`0x40`** des barres (position, largeur, marqueurs de volume) ;
- aspect des **fonds préréglés** (types 1-6, 100, dégradé) et des **6 styles** de zone d'action ;
- ce que montre la zone **y 0-79** et le champ `screenType` ;
- le bouton power avec l'hôte connecté : seulement la luminosité, ou aussi l'écran éteint ?

## 7. Risques

- Repasser l'appareil au firmware d'origine pour développer et tester (sauvegarde et DFU déjà
  maîtrisés, voir [../firmware/stage1.md](../firmware/stage1.md)) : à chaque aller-retour.
- Deux chemins de rendu à maintenir : toute nouvelle fonction devra dire ce qu'elle devient sur
  le firmware d'origine.
- Les modules tiers qui dessinent des pages riches seront lents sur le firmware d'origine tant
  qu'ils n'utilisent pas les cases natives.
