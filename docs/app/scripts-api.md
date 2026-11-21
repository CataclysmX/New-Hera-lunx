# Étude : API des scripts (proposition)

Suite de [scripts.md](scripts.md). Noms provisoires en français sans accents (décision en attente).
Tout ce que l'app fait aujourd'hui est accessible ; ce qui touche au système passe par une
permission du manifeste.

## Cycle de vie et événements
| Fonction du script | Quand |
|--------------------|-------|
| `init(params)` | chargement ou rechargement du paquet, ou changement de ses paramètres |
| `draw(w)` | à chaque image où le widget est visible (`w` = cadre, paramètres) |
| `on_tick(dt)` | 60 fois par seconde (logique, animations) |
| `on_turn(e)` | molette tournée : `e.molette` (1-4), `e.delta` (crans), `e.vitesse` (crans/s, pour l'accélération), `e.maintenus` (boutons tenus pendant qu'on tourne : combinaisons) |
| `on_press(e)` / `on_release(e)` | appui / relâchement : `e.bouton` = 1-4, `"molette1"`…`"molette4"`, `"power"` |
| `on_click(e)`, `on_double(e)`, `on_hold(e)`, `on_repeat(e)` | clic court, double appui, maintien (0,5 s), répétition pendant le maintien |
| `on_page(entree)` | la page devient visible (`true`) ou non |
| `on_notification(n)`, `on_media(m)`, `on_fullscreen(app)` | notification reçue, changement du lecteur média, appli en plein écran |
| `on_message(nom, donnees)` | message d'un autre script |

Un gestionnaire qui renvoie `true` a **consommé** l'événement ; sinon il continue (widgets, puis
liaisons de la fenêtre). `appuye(b)` dit si un bouton est tenu à l'instant.

## Dessin (coordonnées de l'écran, 480×272)
- Formes : `rectangle(x, y, l, h, peinture, {arrondi, bordure, epaisseur})`, `cercle`, `ellipse`,
  `arc(cx, cy, r, debut, fin, epaisseur, peinture)`, `ligne`, `polygone(points, …)`, `chemin` (courbes).
- Texte : `texte(x, y, chaine, {taille, gras, align, couleur, largeur_max, lignes, defiler})`,
  `mesurer(chaine, taille, gras)` → largeur, hauteur.
- Images : `image(chemin, x, y, l, h, {ajuster, opacite})`, `icone(nom, …)` (thème du bureau,
  version symbolique teintée), `pochette()` (lecteur média), `avatar()` (notification).
- Peintures : `"#rrggbb"`, `rgb(…)`, `hsl(…)`, `melange(a, b, t)`, `alpha(c, a)`,
  `degrade({…}, {positions, net, vertical})`.
- Composition : `decouper(x, y, l, h)` (zone de dessin), `translater(dx, dy)`, `echelle(s)`,
  `opacite(a)`, `sauver()` / `restaurer()` ; `memoriser(cle, l, h, fonction)` dessine une fois une
  case dans une image gardée en cache et la recolle ensuite (coût quasi nul par image).
- Défilement matériel : `defiler_zone(x, y, l, h, dy)` utilise la commande firmware « copier une
  zone » (sinon redessin complet) ; la bibliothèque `ui` s'en sert toute seule.

## Actions
`volume(cible, delta)`, `volume_fixer(cible, pct)`, `muet(cible[, etat])`, `touches("ctrl+alt+t")`,
`taper("texte")`, `url("…")`, `page(n | "suivante" | "precedente" | nom)`, `luminosite(pct)`,
`led(n, pct | "clignoter")`, `notifier(titre, texte)`, `veille()` ;
avec la permission `commandes` : `commande("…")`, `commande_sortie("…", fonction(sortie))`
(asynchrone, ne bloque pas l'écran).

## Données
- Audio : `sources()` (sorties, entrées, applis présentes), `source(cible)` → `nom`, `volume`,
  `muet`, `niveau`, `crete`, `icone`, `presente` ; `spectre(cible, n)` (n bandes 0..1).
- Média (MPRIS) : `media()` → `titre`, `artiste`, `album`, `pochette`, `position`, `duree`,
  `lecture` ; `media_action("suivant" | "precedent" | "lecture" | …)`.
- Bureau : `plein_ecran()` (appli au premier plan), `applications()` (menu du bureau : nom,
  icône, lancement), `jeux_steam()`, notifications récentes.
- Système : `heure()`, `date()`, `processeur()`, `memoire_vive()`, `temperatures()`, `reseau()`.
- Avec permissions : `http(…)`, `websocket(…)` (`reseau`), `lire(…)` (`fichiers`),
  `dbus_appel(…)` (`dbus`).

## Temps et animations
`maintenant()`, `apres(ms, f)`, `chaque(ms, f)`, `annuler(id)` ;
`anim(de, a, duree, courbe)` → valeur animée (`courbe` : `lineaire`, `douce`, `rebond`, `ressort`).

## État, mémoire, communication
- Variables du script : gardées tant que le paquet est chargé.
- `memoire.lire(cle)`, `memoire.ecrire(cle, valeur)` : gardé entre les lancements
  (`~/.local/state/hsm01/scripts/<paquet>.toml`), par paquet.
- `envoyer(nom, donnees)` / `on_message` : faire parler deux scripts (une page et ses widgets, un
  menu et un compteur…).

## Paramètres (formulaire automatique dans la fenêtre)
```lua
parametres {
  colonnes = entier { min = 1, max = 8, defaut = 4 },
  accent   = couleur { defaut = "#00b4a8" },
  fond     = degrade { defaut = { "#101818", "#182424" } },
  cible    = cible_audio { defaut = "sortie" },
  cases    = liste_de_cases { },     -- éditeur façon Stream Deck dans la fenêtre
  infini   = booleen { defaut = true },
}
```
Types : `entier`, `reel`, `booleen`, `texte`, `choix {…}`, `couleur`, `degrade`, `icone`,
`image`, `cible_audio`, `action` (même éditeur que les boutons), `touche_ou_molette`,
`liste_de_cases`. Valeurs enregistrées dans `config.toml` avec le widget ou la page ;
Ctrl+Z, aperçu en direct et enregistrement comme le reste.
