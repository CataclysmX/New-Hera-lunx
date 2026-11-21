# Étude : API ouverte (modules des utilisateurs dans n'importe quel langage)

État : **choisie et en partie faite** (2026-11-20) : référence dans [api.md](api.md). Fait :
serveur (socket Unix + WebSocket à jeton), niveau déclaratif (alertes, pages de widgets,
événements), `hsm01ctl`, bibliothèque Python + menu façon Stream Deck (côté client),
détection des défilements + firmware COPY. Pas encore : surfaces en mémoire partagée, menus
déclaratifs côté serveur, lancement des modules par l'app, bibliothèques Rust / JavaScript.
Étude d'origine : Alternative aux scripts intégrés de
[scripts.md](scripts.md) : au lieu d'imposer un langage dans l'app, l'app devient un **serveur
d'écran** pour le Stream 100 et publie une API locale ; chacun écrit ses modules en Python, Rust,
JavaScript, shell… C'est le modèle des logiciels du même genre (kit de développement du Stream Deck,
plugins de Touch Portal : un programme à part qui parle à l'application par un socket en JSON).

## Partage des rôles
| Notre app (serveur) | Les modules (clients) |
|---------------------|----------------------|
| seule à parler à l'appareil USB | n'importe quel langage, n'importe quelle bibliothèque (OBS, Home Assistant, Discord, Twitch, Spotify… ont déjà les leurs) |
| **optimisation de l'envoi** : zones modifiées seulement (tuiles 16 px, RLE, budget par image), détection automatique des défilements + commande firmware « copier une zone » | dessinent ce qu'ils veulent, ou décrivent des cases / widgets |
| **composition** : pages, couches, alertes par-dessus tout, veille, luminosité, LED | reçoivent les entrées qui les concernent |
| **routage des entrées** : quelle molette / quel bouton va à quel module (par page, par zone) | réagissent (actions, état, réseau…) |
| composants prêts : widgets actuels, menus (briques de [scripts-menus.md](scripts-menus.md)), alertes | |

L'optimisation seule ne suffit pas : il faut aussi **composer** (plusieurs modules sur une même page,
une alerte par-dessus) et **arbitrer les entrées** (deux modules ne doivent pas recevoir le même
bouton). C'est le travail d'un petit gestionnaire d'écran, que l'app fait déjà en partie
(`screen.rs`, `engine.rs`).

## Deux niveaux d'API
1. **Déclaratif** (le plus simple, rendu par l'app) : un module envoie « case 3 : titre Micro, icône
   `audio-input-microphone`, allumée » ou « menu : ces 12 cases, grille 4 × 2, défilement infini »,
   puis reçoit « case 3 activée ». Pas de dessin à écrire, aspect cohérent avec le reste, envoi
   minimal (l'app sait exactement ce qui a changé, et quand un menu défile).
2. **Surfaces** (liberté totale) : l'app donne au module un rectangle de l'écran (ou l'écran
   entier) en mémoire partagée ; le module dessine avec ce qu'il veut (Cairo, Pillow, Qt, un
   navigateur…), puis signale les zones modifiées (et, s'il le sait, un défilement). L'app compose
   et n'envoie que la différence ; un défilement non signalé est détecté (comparaison des lignes
   décalées).

## Transport
- Socket Unix `$XDG_RUNTIME_DIR/hsm01/api.sock`, messages JSON (une ligne par message) ; images en
  mémoire partagée (`memfd`) ou en PNG / SVG dans le message pour les modules simples.
- **Commande `hsm01ctl`** pour le shell : `hsm01ctl alerte "Compilation finie" --icone dialog-ok`,
  `hsm01ctl page 2`, `hsm01ctl led 1 clignoter`, `hsm01ctl case 3 --titre "42 %"` (cron, scripts
  de compilation, raccourcis du bureau…).
- WebSocket local en option (modules web), avec jeton.
- Bibliothèques : Python d'abord (le plus accessible), Rust (utilisée par l'app elle-même), puis
  JavaScript ; protocole documenté pour tout autre langage.

## Messages (aperçu)
| Sens | Message |
|------|---------|
| → | `bonjour {nom, version}` ; `page.creer`, `page.afficher` |
| → | `case.definir {id, titre, icone, image, couleur, valeur, allume}` ; `menu.definir {cases, disposition, defilement, liaisons, style}` ; `widget.definir {…}` (tous les widgets actuels) |
| → | `surface.creer {page, x, y, l, h, z}` → mémoire partagée ; `surface.valider {zones, defilement}` |
| → | `alerte {titre, texte, icone, image, duree, priorite, boutons:[…], reveil, led}` |
| → | `entrees.prendre {page, molettes, boutons}` (réservation, rendue à la fermeture du module) ; `led`, `luminosite` (demandes, l'app arbitre) |
| ← | `entree {molette, delta, vitesse, bouton, appui, clic, double, maintien}` ; `case.activee {id}` ; `alerte.reponse {bouton}` ; `page.visible {page, oui}` ; `veille {oui}` |

## Cycle de vie des modules
- Lancés par l'app (déclarés dans la config, avec leurs paramètres réglables dans la fenêtre) ou
  indépendants (ils se connectent quand ils veulent).
- L'app relance un module qui plante, l'arrête à la fermeture ; un module déconnecté libère ses
  entrées et ses surfaces (la page revient à ses widgets).
- Pas de bac à sable : un module est un programme de l'utilisateur, avec ses droits (comme n'importe
  quelle application). N'installer que des modules de confiance.

## Ce qu'on perd par rapport aux scripts intégrés
- Pour un non-développeur, un module = un programme à lancer : compensé par les **modules livrés**
  (ci-dessous) réglables dans la fenêtre, et par les menus déclaratifs éditables sans code.
- Un peu plus de latence (local, < 1 ms) et un processus par module.

## Modules proposés (livrés avec l'app, et exemples pour les auteurs)
Menus :
- **Stream Deck par rangées** : grille 4 × 2, défilement infini, bouton k = case k de la rangée.
- **Menu par lignes** (réglages) : molette 1 choisit, molette 2 règle, clic active.
- **Lanceur d'applications** (menu du bureau, favoris) et **jeux Steam** (carrousel de pochettes).
- **Mixeur** : une case par appli qui joue du son (volume à la molette, muet au clic).
- **Sortie / entrée audio** : choisir le casque, les enceintes, le micro.
- **Contexte** : cases différentes selon l'appli au premier plan (jeu, OBS, navigateur).
- **Soundboard** et **macros** (suite de touches, texte, commandes).
Contrôle :
- **Média** : pochette, titre, position, piste suivante / précédente (MPRIS).
- **OBS** : scènes, enregistrement, diffusion, micro (WebSocket d'OBS).
- **Discord** : micro / casque coupés, salon vocal.
- **Maison** : lumières et prises (Home Assistant).
Infos :
- **Système** : processeur, carte graphique, températures, mémoire, réseau.
- **Horloge / météo**, **minuteur / pomodoro**, **calendrier** (prochain rendez-vous).
Alertes (même fenêtre que les notifications, avec boutons de réponse) :
- notifications du bureau (déjà faites, deviendraient un module comme les autres) ;
- **alertes système** : batterie de la souris / du casque, température élevée, disque presque
  plein ;
- **alertes de stream** : abonnement, raid, don ;
- **fin de tâche** : `make && hsm01ctl alerte "Compilation finie"`, téléchargement terminé ;
- **rappels** : minuteur écoulé, rendez-vous dans 10 min ;
- tout module peut envoyer les siennes (priorité, durée, LED qui clignote, réveil de l'écran).

## Recommandation
**L'API ouverte comme fondation**, avec les menus et les alertes comme composants déclaratifs du
serveur (les briques de [scripts-menus.md](scripts-menus.md) restent valables, appelées par l'API
au lieu d'un script Lua). Raisons : n'importe quel langage, les bibliothèques existantes (OBS,
Home Assistant, Discord…) utilisables telles quelles, un seul endroit qui optimise l'envoi pour
tout le monde, et les modules restent hors de l'app (un module qui plante ne fige pas l'écran).
Les scripts Lua intégrés ([scripts.md](scripts.md)) deviennent facultatifs : à reconsidérer plus
tard pour de petits styles de cases, si le besoin se confirme.

Choix à faire :
1. API ouverte (recommandée), scripts Lua intégrés, ou les deux.
2. Première bibliothèque cliente : Python (recommandé) ou autre.
3. Noms des messages et des champs : français ou anglais (l'anglais aide le partage des modules
   hors de France).
4. Firmware « copier une zone » (défilement fluide) : oui ou non.

## Étapes proposées
1. **Serveur** : extraire de l'app le cœur « écran » (composition, optimisation, entrées, alertes)
   derrière une interface interne ; les pages actuelles deviennent son premier client.
2. **API** : socket + JSON, niveau déclaratif (cases, widgets, alertes), `hsm01ctl`, bibliothèque
   Python ; 2-3 modules (alertes système, média, Stream Deck par rangées).
3. **Surfaces** (mémoire partagée), détection de défilement, firmware « copier une zone ».
4. **Menus déclaratifs** complets (briques de [scripts-menus.md](scripts-menus.md)) et éditeur dans
   la fenêtre ; gestion des modules (lancement, paramètres, relance).
5. Autres modules, bibliothèques Rust et JavaScript, documentation du protocole.
