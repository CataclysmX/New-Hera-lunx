# Étude : scripts utilisateur (styles, pages, menus programmables)

État : **étude, rien d'implémenté** (2026-11-20). Objectif : que les utilisateurs écrivent eux-mêmes,
dans un langage simple proche du pseudo-code, des **styles** (dessin d'un widget), des **pages au
fonctionnement différent** et des **menus** (grilles, lignes, carrousels, dossiers…), sans
recompiler l'app, et qu'ils puissent **réutiliser et partager** ce qu'ils font. Il faut donc leur
donner tout ce que l'app sait faire, par briques combinables, plutôt que quelques menus figés.

**Alternative recommandée** : une API ouverte dans n'importe quel langage, l'app servant d'écran
optimisé ([api-externe.md](api-externe.md)) ; ce document reste valable pour l'option « scripts
intégrés » et pour les briques de menus.

Suite de l'étude :
- [scripts-api.md](scripts-api.md) : l'interface de programmation (événements, dessin, actions,
  données, temps, mémoire, paramètres, permissions) ;
- [scripts-menus.md](scripts-menus.md) : les briques de menus (dispositions, défilement fini /
  infini / par pages, focus, boutons liés à des cases, styles de cases, sources de contenu).

## Architecture en trois couches
| Couche | Écrite en | Contenu | Modifiable par l'utilisateur |
|--------|-----------|---------|------------------------------|
| **Noyau** | Rust (dans l'app) | moteur de script, dessin (mêmes polices, dégradés, étalonnage que les widgets), entrées, actions, données, temps, mémoire, permissions, cache d'images, défilement matériel | non |
| **Bibliothèque `ui`** | Lua, livrée avec l'app | défilement, focus, liaisons, dispositions (grille, ligne, liste, carrousel, colonnes), menus et dossiers, animations, styles de cases | oui : lisible, copiable, remplaçable module par module |
| **Paquets** | Lua + images + manifeste | menus, styles, pages des utilisateurs, partageables | oui |

Tout menu livré (grille par rangées, liste par lignes, carrousel infini…) est **écrit avec les
mêmes briques publiques** que celles des utilisateurs : rien de réservé à l'app. Un utilisateur peut
donc partir d'un menu existant, changer une seule brique (le style des cases, le défilement) et
garder le reste.

## Où vivent les scripts
- Widget : `kind = "script"`, `paquet = "menu-grille"`, `x`, `y`, `w`, `h` + ses paramètres
  (formulaire automatique dans la fenêtre, voir `parametres` dans l'API). Il dessine dans son
  cadre et reçoit les entrées que la page lui donne.
- Page : `[[page]] script = "lecteur"` : reçoit d'abord tous les événements ; ce qu'il ne traite
  pas (il ne renvoie pas `true`) va aux widgets scripts, puis aux liaisons de la fenêtre.
- Global (plus tard) : scripts sans écran, toujours actifs (réagir aux notifications, à l'appli en
  plein écran, changer de page automatiquement quand un jeu démarre…).

## Langage
| | **Lua 5.4** (crate `mlua`, compilé avec l'app) | **Rhai** (crate `rhai`, 100 % Rust) | Langage maison |
|---|---|---|---|
| Lisibilité | `if … then … end`, `for i = 1, 10 do` : proche du pseudo-code | type JavaScript / Rust | maximale, à concevoir |
| Connu sous Linux | Conky (dessins en Lua), AwesomeWM, OBS, jeux | peu | non |
| Vitesse (dessin 60/s) | très bonne | 2 à 5× plus lent | selon l'effort |
| Modules, réutilisation | `require` natif (limité aux paquets) | modules | à écrire |
| Bac à sable | `math`, `string`, `table`, `utf8` seulement ; mémoire plafonnée ; interruption après N instructions | limites intégrées | à écrire |

**Recommandation : Lua 5.4** (lisible, documenté, rapide, modules natifs pour la réutilisation).

## Partage et réutilisation
- **Paquet** = un dossier `~/.config/hsm01/scripts/<nom>/` : `manifeste.toml` (nom, version, auteur,
  version d'API visée, permissions demandées, paquets requis, points d'entrée), scripts, images.
- `require("ui.defilement")` (bibliothèque), `require("autre-paquet.module")` (paquet déclaré dans
  le manifeste) : jamais de fichier hors des paquets.
- Import / export d'un paquet en `.zip` depuis la fenêtre ; galerie d'exemples livrée (et copiable).
- API versionnée : un paquet écrit pour l'API 1 continue de marcher quand l'API s'enrichit.
- Les **paramètres** déclarés par un paquet (couleurs, nombre de colonnes, liste de cases…) se
  règlent dans la fenêtre, sans toucher au code : un même paquet sert à plusieurs pages.

## Sûreté
- Par défaut un script ne peut ni lire de fichiers, ni aller sur le réseau, ni lancer de programme.
- **Permissions** demandées dans le manifeste et accordées une fois dans la fenêtre : `commandes`
  (lancer des programmes, comme l'action « commande » actuelle), `reseau` (HTTP, WebSocket : OBS,
  Home Assistant…), `fichiers` (lecture d'un dossier choisi), `dbus` (appels au bureau). Sans
  permission, la fonction échoue avec un message clair.
- Garde-fous : un appel interrompu au-delà de ~5 ms / N instructions, mémoire plafonnée par paquet,
  erreur affichée sur le widget (cadre rouge, ligne) et dans la fenêtre, script en pause jusqu'à la
  prochaine modification. Une boucle infinie ne fige jamais les molettes.

## Modules pour les auteurs
- Rechargement à chaud à chaque enregistrement du fichier ; console des messages `journal(…)` et
  des erreurs dans la fenêtre ; temps de calcul par script (ms / image).
- **Commandes virtuelles** dans l'aperçu (molettes et boutons cliquables) pour tester sans
  l'appareil.
- Éditeur intégré (numéros de ligne, coloration, erreur surlignée) ; documentation de l'API générée
  depuis le noyau.

## Limite matérielle (défilement)
L'USB du Stream 100 (full speed) transporte ~720 Kio d'image par seconde : un écran entier ~3 fois
par seconde. Changer un surlignage ou une case est immédiat ; un défilement animé de toute une
grille saccaderait. D'où une **nouvelle commande firmware « copier une zone »** (240.21, copie par
le DMA2D) : l'appareil décale lui-même les pixels affichés, l'app n'envoie que la bande qui apparaît
(détails dans [scripts-menus.md](scripts-menus.md)). Sans elle : défilement par rangée, sans
animation (~0,2-0,3 s pour repeindre).

## Étapes proposées
1. **Noyau, dessin** : Lua, widget `script`, dessin, paramètres + formulaire, erreurs, rechargement.
2. **Événements et actions** : toutes les entrées (voir API), actions, temps, mémoire, scripts de page.
3. **Bibliothèque `ui`** : défilement (fini, infini, par pages, virtuel), focus, liaisons
   boutons → cases, dispositions, styles de cases, menus et dossiers ; exemples.
4. **Firmware « copier une zone »** et défilement fluide.
5. **Fenêtre** : éditeur de liste de cases (façon logiciel Stream Deck : glisser, icône, action,
   dossiers) branché sur le paramètre « liste de cases », éditeur de script, console, commandes
   virtuelles.
6. **Données et partage** : média (MPRIS), système, Steam, applications installées ; permissions
   réseau / D-Bus ; paquets (manifeste, import / export, dépendances).

## À décider avant de commencer
1. Langage : Lua (recommandé) ou Rhai.
2. Noms des fonctions : français sans accents (`rectangle`, `texte`, `touches`) ou anglais.
3. Firmware « copier une zone » pour le défilement fluide : oui (reflash) ou non.
4. Ordre : suivre les étapes ci-dessus, ou viser d'abord un premier menu complet (étapes 1 à 3
   réduites au nécessaire) pour juger sur pièce.
