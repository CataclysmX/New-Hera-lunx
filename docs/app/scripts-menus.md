# Étude : briques de menus (bibliothèque `ui`)

Suite de [scripts.md](scripts.md) et [scripts-api.md](scripts-api.md). Un menu n'est pas un bloc
figé : c'est l'assemblage de **sept briques indépendantes**, chacune remplaçable par celle d'un
utilisateur. Changer une brique (la disposition, le défilement, le style des cases) garde tout le
reste.

| Brique | Rôle | Fournies |
|--------|------|----------|
| source | quelles cases | liste réglée dans la fenêtre, fonction du script, fournisseurs (applis audio, applications, jeux Steam…) |
| disposition | où sont les cases | grille, ligne, liste par lignes, colonnes indépendantes, libre, pages |
| défilement | comment on parcourt | fini, infini, par pages, virtuel ; par case / rangée / page ; animé ou non |
| focus | ce qui est « actif » | aucun, rangée, case, colonne |
| liaisons | quelle entrée fait quoi | boutons → cases (relatif ou épinglé), molettes, clics, maintiens, combinaisons, LED |
| style | à quoi ressemble une case | plat, relief, néon, pochette, icône seule, réglage… |
| navigation | dossiers | pile, retour, accueil, position gardée, transitions |

## Cases (éléments)
`{ titre, icone | image, couleur | degrade, valeur = f, allume = f, badge, action, maintien,
double, dossier = {…} | f, desactive = f, style, donnees }` : `valeur` / `allume` / `desactive`
peuvent être des fonctions (relues à chaque image : volume, état du micro, heure…) ; `action`,
`maintien`, `double` = une action de l'app (même éditeur que les boutons) ou une fonction Lua.

## Dispositions
- **Grille** `colonnes × rangees` (4 × 2 par défaut : une colonne par molette).
- **Ligne** : une seule rangée horizontale (carrousel), case centrale mise en avant si voulu.
- **Liste par lignes** : une entrée par ligne (icône, titre, valeur ou petit contrôle à droite :
  interrupteur, barre de réglage) ; menu de réglages, liste de pistes, de jeux…
- **Colonnes indépendantes** : 4 rouleaux, chaque molette fait défiler sa propre colonne.
- **Libre** : chaque case à une position fixe (copie exacte d'un agencement façon Stream Deck).
- **Pages** : plusieurs écrans de cases, power ou une molette pour passer de l'un à l'autre.

## Défilement
- **Fini** (butées, petit rebond), **infini** (après la dernière case revient la première, sans
  coupure : indices modulo le nombre de cases ; en mode « auto », pas de boucle s'il y a moins de
  cases que de places), **par pages** (saut d'un écran, points indicateurs), **virtuel** (nombre de
  cases illimité ou inconnu : la source fournit la case n° i à la demande, ex. jours, historique).
- Pas : une case, une **rangée** (menu « ligne par ligne »), une page ; crans de molette par pas.
- Accélération (tourner vite saute plus loin), inertie facultative, arrêt aligné sur une case,
  indicateur de position (barre ou points).
- Animé avec la commande firmware « copier une zone » (ci-dessous), sinon par sauts.

## Liaisons (entrées → cases)
- **Relatives à la vue** : bouton k = case k de la rangée active, ou k-ième case visible, ou case
  centrale de la colonne k (rouleaux).
- **Épinglées** : une case précise attachée à un bouton (`bouton = 2`), toujours accessible même
  hors de l'écran ; badge « 2 » sur la case ; la LED du bouton montre son état (allumée, éteinte,
  clignote).
- Molettes : défiler (tout le menu, une colonne, une ligne), **régler la valeur** de la case active
  (volume, luminosité, position dans une piste), changer de dossier.
- Clic / double / maintien : déclencher, action secondaire de la case, ouvrir, retour, accueil.
- Combinaisons : bouton tenu + molette = régler la case liée à ce bouton ; power + molette =
  changer de menu.
- Chaque menu a une table de liaisons par défaut, **modifiable dans la fenêtre** (paramètre
  `liaisons`), sans toucher au script.

## Styles de cases
Un style est une fonction `style(case, etat, cadre)` ; `etat` = normal, focus, enfoncée (petite
animation), allumée, désactivée. Les styles livrés utilisent `memoriser` : une case immobile ne
coûte presque rien. Un utilisateur écrit le sien (ou copie un style livré) et le donne à n'importe
quel menu.

## Navigation
Pile de dossiers (retour, accueil, fil d'Ariane), position gardée par dossier, dossiers dynamiques
(fonction appelée à l'ouverture), transition à l'ouverture (glissement, zoom).

## Assemblage (exemple)
```lua
local ui = require("ui")
local menu = ui.menu {
  source      = ui.sources.parametre("cases"),       -- ou ui.sources.applis_audio()
  disposition = ui.grille { colonnes = 4, rangees = 2 },
  defilement  = ui.defilement { mode = "infini", pas = "rangee", anime = true },
  focus       = "rangee",
  liaisons    = { boutons = "rangee_active", molette1 = "defiler", power = "retour" },
  style       = ui.styles.neon { accent = params.accent },
}
function draw(w) menu:dessiner(w) end
function on_turn(e) return menu:tourner(e) end
function on_click(e) return menu:cliquer(e) end
```
Remplacer seulement `disposition = ui.liste { hauteur_ligne = 34 }` donne un menu par lignes avec
les mêmes cases, le même défilement et les mêmes liaisons.

## Menus d'exemple livrés (tous faits avec ces briques)
1. **Stream Deck par rangées** : grille 4 × 2, défilement infini rangée par rangée, bouton k =
   case k de la rangée active.
2. **Menu par lignes** : molette 1 choisit la ligne, molette 2 règle sa valeur, clic = activer.
3. **Carrousel infini** de grandes pochettes (média, jeux), case centrale en avant.
4. **Quatre rouleaux** : chaque molette fait tourner sa colonne en boucle, bouton k = case
   centrale de la colonne k.
5. **Agencement fixe** : 15 cases sur 3 pages, 4 épinglées aux boutons.

## Commande firmware « copier une zone » (pour le défilement fluide)
- `COPY(x, y, l, h, dx, dy)` dans le flux bulk OUT : l'appareil recopie un rectangle de son image
  à un autre endroit (DMA2D du STM32L4R7, ou ligne par ligne dans le bon ordre quand les zones se
  recouvrent).
- L'app applique la même copie à son image de référence (`screen.rs`) avant le diff par tuiles :
  seule la bande qui apparaît est envoyée (480 × quelques lignes par image au lieu de la grille).
- Sert aussi aux textes qui défilent et aux transitions de page. Firmware 240.21, trappe DFU
  inchangée.
