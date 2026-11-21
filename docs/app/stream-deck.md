# Mode « stream deck » (2026-11-20)

**Réalisé** : le widget `deck` (une piste, une molette au choix, sur n'importe quelle page :
[companion.md](companion.md)) et le module livré « Stream deck » (`app/modules/streamdeck/` : 4 pistes
comme la maquette, `--layout colonnes|lignes`, cases dans `deck.toml` : [api.md](api.md)). Le
reste de cette page : la maquette, son test, et les idées d'éditeur pas encore faites.

Maquette de départ : `concept/stream_deck_non_tactile_hercules_demo.html` (page web, non tactile).
Principe : **4 pistes, une par molette**, chacune un carrousel **infini** de cases (titre,
sous-titre, icône, action) ; tourner la molette k déplace la sélection de la piste k, cliquer la
molette k valide la case sélectionnée. Deux dispositions : 4 lignes ou 4 colonnes.

## Test de la maquette
Chromium sans interface, copie de la page + script de test injecté (molette souris, clavier,
mesure des positions ; transitions coupées pour mesurer l'état final). 11 vérifications :
| Vérification | Résultat |
|---|---|
| 4 lignes affichées ; molette de la souris sur une molette ; clic = valide la case en surbrillance (journal) | ✔ |
| Horizontal : case active toujours au bord gauche | ✘ dérive de 44 px après 6 crans : `translateX(-n × 25 %)` oublie l'espace de 0,5 rem entre cases ; 4e case coupée (3 entières visibles) |
| Vertical : case active visible après 1 cran | ✘ 473 px au-dessus de la fenêtre : `translateY(calc(-n × (100 % + 0,5rem)))`, 100 % = hauteur de **toute** la piste (16 cases), pas d'une case ; les colonnes montrent d'autres cases |
| Boucle infinie sans à-coup | ✘ au passage 1re ↔ 8e case la ligne glisse de 7 cases en arrière (le décalage revient de 7 à 0) |
| Maj+1..4 = tourner à gauche | ✘ Maj+1 donne « ! » (`e.key`), jamais « 1 » |
| AZERTY | ✘ la touche 1 donne « & » (rien) ; la touche A (place du Q QWERTY) **valide** la molette 1 au lieu de la tourner. Utiliser `e.code` (`Digit1`, `KeyQ`…) |
| Lueur de la ligne OBS (violette) | ✘ classe `shadow-neon-purple` absente de la config Tailwind |

Autres remarques : la case active est la première visible (le commentaire dit « au centre ») ;
« Journal d'Validation ».

## Sur l'écran (480×272)
- **Colonnes** (défaut proposé) : la piste k est **au-dessus de la molette k** — on tourne la
  molette sous ce qu'on regarde. Bandeau 24 px (nom et couleur des pistes), 4 colonnes de 120 px,
  cases 112×76, **case sélectionnée au centre** (précédente au-dessus, suivante en dessous, 3 cases
  entières visibles), cadre et lueur à la couleur de la piste.
- **Lignes** : 4 lignes de 68 px, étiquette 64 px, cases 128×60 avec la sélection au centre.
- Défilement animé (glissement ~150 ms) : envoyé en COPY (firmware 240.21+), presque rien sur
  l'USB. Boucle infinie calculée par l'index (case `i mod n`), donc sans retour en arrière.
- Clic : impulsion sur la case + LED k qui clignote ; clic long : action secondaire (facultative).
- Un cran = une case, sensibilité réglable (par défaut 1/16 de tour).

## Définir les cases : l'éditeur (dans l'app)
Un widget **« stream deck »** (onglet Widgets, comme le widget module : un cadre sur une page, on
peut garder un titre au-dessus). Sur sa page, molettes 1-4 (rotation et clic) vont au deck ;
boutons 1-4 au choix : réglages de la page, ou « valider la piste k ».
```
Disposition [colonnes | lignes]    Cran [1/16 tour]    Boutons 1-4 [page | valider]
┌ Molette 1 ───┐┌ Molette 2 ───┐┌ Molette 3 ───┐┌ Molette 4 ───┐
│ Micro    ●   ││ Média    ●   ││ Discord  ●   ││ OBS      ●   │  nom + couleur de la piste
│ [ic] Couper  ││ [ic] Lecture ││ [ic] Muet    ││ [ic] Jeu     │  cases : glisser pour ranger,
│ [ic] Gain +3 ││ [ic] Suivant ││ [ic] Sourdine││ [ic] Chat    │  y compris d'une piste à l'autre
│ [ + case ]   ││ [ + case ]   ││ [ + case ]   ││ [ + case ]   │
└──────────────┘└──────────────┘└──────────────┘└──────────────┘
Case : Molette 2 › Suivant                                   [Tester]
Titre [Suivant]   Sous-titre [Titre suivant]
Icône [media-skip-forward] + suggestions du thème (grille cliquable) [Fichier…]
Couleur [celle de la piste | …]   Action [média ▾ suivant ▾]   Clic long [rien]
```
- **Miroir de l'appareil** : la grille de l'éditeur a la disposition choisie ; la case cliquée
  est aussi amenée au centre dans l'aperçu à gauche (on voit le rendu réel).
- **Ajout rapide** (« + case ») : *Application…* (liste des applications installées avec leur
  icône, recherche ; remplit titre, icône et commande), *Média* (lecture, suivant, précédent,
  stop : 4 cases d'un coup), *Raccourci…* (« appuie sur la combinaison » : capturée, plus besoin
  de l'écrire en positions QWERTY), *Commande*, *Page*, *Module*, *Muet*, *Luminosité*.
- **Clavier** : Suppr, Ctrl+D (dupliquer), Ctrl+↑/↓ (déplacer dans la piste), comme les widgets.
- **Tester** : lance l'action depuis le PC, sans l'appareil.
- Nouvelles actions (utiles partout, boutons compris) : `media` (lecture-pause, suivant,
  précédent, stop via le suivi des lecteurs déjà présent) et `app` (application `.desktop`).

Enregistré dans `config.toml` :
```toml
[[page.widget]]
kind = "deck"
x = 0
y = 0
w = 480
h = 272
layout = "columns"
[[page.widget.tracks]]
name = "Média"
color = "#ffaa00"
[[page.widget.tracks.tiles]]
title = "Suivant"
subtitle = "Titre suivant"
icon = "media-skip-forward"
action = { type = "media", action = "next" }
```

## Fait / reste
- Fait : widget `deck` = **une** piste (on peut n'en mettre qu'un, sur une seule molette), rendu
  natif (sélection centrée, glissement, éclair), action `media`, éditeur simple dans l'inspecteur
  (cases repliables, ↑ ↓, « + case », « + média ») ; requête API `action` ; module « Stream deck »
  (4 widgets `deck` posés par un petit programme Python, disposition en paramètre).
- Fait aussi : **onglet « Stream deck »** de la fenêtre (module activé, `[settings]` de son
  `module.toml`) : disposition, pistes (nom, couleur), cases, écrit dans `deck.toml` (`deckfile.rs`,
  commentaires gardés). Mécanisme générique : tout module peut déclarer son fichier ([modules.md](modules.md)).
- Pas encore : l'éditeur « miroir » ci-dessus (glisser-déposer entre pistes), *Application…*
  (liste des `.desktop`), capture de raccourci, action `app`, boutons 1-4 = valider la piste k,
  sensibilité réglable (fixe : 40 comptes par case), deck dans une page de module incrustée (widget
  `tool`) : affiché mais sans molette. Le module Python `deck` (grille 4×2) reste comme exemple.
