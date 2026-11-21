# API ouverte de new hera-nux (version 1)

Des programmes locaux, dans n'importe quel langage, affichent des **alertes** et des **pages** sur
le Stream 100 et reçoivent ses **molettes et boutons**. L'app garde l'appareil, le dessin et
l'envoi optimisé (zones modifiées seulement, défilements en COPY : [stage2.md](../firmware/stage2.md)).
Code : `app/hsm01-app/src/api.rs`. Étude d'origine : [api-externe.md](api-externe.md).

## Connexion
| Transport | Adresse | Pour |
|-----------|---------|------|
| Socket Unix | `$XDG_RUNTIME_DIR/hsm01/api.sock` (ou `$HSM01_API_SOCKET`), droits 0600 | programmes locaux (Python, shell, Rust…) |
| WebSocket | `ws://127.0.0.1:9137/?token=…` (`[api] websocket`, `port`) | pages web, Bruno / Postman, langages sans socket Unix |

WebSocket : interrupteur dans la fenêtre (Appareil → API ouverte), appliqué tout de suite (port
changé : relancé sur le nouveau, connexions ouvertes fermées à l'arrêt). Le **jeton** est tiré au
hasard une fois et gardé d'un lancement à l'autre dans `~/.local/state/hsm01/api-token` (0600) ;
« Copier l'adresse » le met dans l'URL, « Nouveau jeton » le remplace (les connexions ouvertes
restent). Sans lui la connexion est refusée (403) : une page web de n'importe quel site peut
joindre un port local, le jeton l'en empêche. Format : un objet JSON par ligne (socket Unix) ou par message
texte (WebSocket), dans les deux sens. Noms des messages et des champs en anglais.

## Requêtes (programme → app)
Toute requête peut porter `"req": <n'importe quoi>` : la réponse (`{"type": "ok", "req": …}` ou
le résultat) le reprend. Sans `req`, seules les erreurs reviennent (`{"type": "error",
"message": …}`), pratique pour les mises à jour rapides.

| `type` | Champs | Effet / réponse |
|--------|--------|-----------------|
| `hello` | `name` | `welcome` : `api` (1), `app`, `screen` {width, height}, `firmware`, `client` |
| `alert` | `title`, `text`, `icon` (nom du thème ou chemin), `image` (photo, recadrée ; `round`), `app`, `duration` (s), `led` (1-4, clignote), `priority: "high"` (avant les autres), `wake` (rallumer l'écran) ; `style` : `card` (défaut : carte en haut) ou **`pill`** (bandeau court centré en bas, qui monte en glissant et redescend à la fin, une ligne : `text` ou à défaut `title` ; `border` contour lumineux, défaut `"#3ddc84"` ; `dot` les deux points, défaut = `border` ; `color` du texte, défaut blanc ; 3 s par défaut) | `{"type":"ok","id":N}` ; plus tard l'événement `alert_closed` |
| `page` | `page` (identifiant), `name`, `background` `"#rrggbb"`, `widgets` [ … ], `grab_power`, `leds` [4 × %], `show` ; **`widget: true`** + `size` {`w`, `h`} (16-2048 px, défaut 480 × 272) : un **widget du module** au lieu d'une page | crée ou remplace la page (montrée après celles de la config). Un widget : contenu de `size` dans ses propres coordonnées, que l'utilisateur place sur ses pages (widget « module », réduit ou défilé selon son cadre) ; jamais montré seul (`show` refusé, absent du menu des modules) ; même `update` / `set` qu'une page ; nouvelle taille : `page` à nouveau |
| `set` | `page`, `widgets` [ … ] | ajoute, ou remplace ceux de même `id` |
| `update` | `page`, `widgets` {`id`: {champ: valeur}} | change quelques champs (position, texte, couleur, même `kind`) : pour les animations |
| `remove` | `page`, `widgets` [ids] | retire des widgets |
| `leds` | `page`, `values` [4 × %] | LED des boutons pendant que la page est affichée |
| `show` | `page` | affiche la page sur le Stream 100 (pas un widget) |
| `close_page` | `page` | supprime la page |
| `media` | (rien) ou `action` : `play_pause`, `play`, `pause`, `stop`, `next`, `previous`, `seek` (`value` s, signé), `volume` (`value` 0..1, signé) | sans action : l'état du lecteur suivi (comme l'événement `media`) ; sinon agit sur lui (MPRIS) |
| `choices` | `choices` : `{"nom": ["valeur", …]}` (fusionnées avec les précédentes ; 1000 par liste au plus) | listes du **formulaire de réglages** du module (champs `choice` avec `source = "nom"`, [modules.md](modules.md)) : serveurs et salons Discord, sondes… ; disparaissent avec le programme |
| `commands` | `commands` : `[{"id", "name", "kind": "button" \| "knob", "description"}]` (la liste complète, remplace la précédente) | **commandes du module** proposées dans la fenêtre tant qu'il tourne : action « commande de module » des boutons (`button`), cible « commande de module » des molettes (`knob`), LED « état d'une commande de module » ; utilisables sur toutes les pages ; disparaissent avec le programme |
| `command_state` | `command`, `active` (bool), `value` (0..1), `text` (facultatifs, seuls ceux donnés changent) | état montré : LED allumée si `active` ; une molette sur la commande a `{name}` = `text` (sinon le nom), `{volume}` / jauge = `value`, grisée si `active` faux |
| `action` | `action` : une action de `config.toml` (`{"type": "keys", "keys": "ctrl+alt+t"}`, `command`, `url`, `media`, `page`, `back`, `brightness_up`…) | l'app l'exécute à l'image suivante ; **programmes locaux seulement** (socket Unix : refusé par le WebSocket, qui pourrait lancer des commandes) |
| `grab` | `inputs` [`knob1`…`knob4`, `button1`…`button4`, `power`] | prend ces entrées sur toutes les pages (molette : tours et clic) : événements `mine: true`, plus aucune liaison de l'app ne les utilise ; refusé si un autre programme les a déjà |
| `ungrab` | `inputs` (facultatif : toutes) | les rend à l'app (aussi à la déconnexion) |

Widgets : exactement ceux de `config.toml` en JSON, plus un `id` : `text`, `rect`, `separator`,
`theme_icon` (icône du thème par nom, ou fichier : SVG, PNG…), `image` (PNG, mise en cache par
chemin : nouveau chemin pour changer l'image ; `radius` arrondit les coins, la moitié du cadre ou
plus : disque, pour un avatar), `frame` (cadre décoratif, `sides`), `deck` (piste
de cases dessinée, défilée et exécutée par l'app : sa molette lui va avant vous, un clic vous
envoie `deck`)… Les widgets
liés aux sources audio des molettes (`vu`, `volume`, `icon`, `spectrum`) n'ont pas de cible sur une
page d'API. Exemple :
`{"id": "t", "x": 10, "y": 10, "w": 200, "h": 30, "kind": "text", "text": "Bonjour", "color": "#ffffff"}`.

## Événements (app → programme)
| `event` | Champs | Quand |
|---------|--------|-------|
| `turn` | `page`, `mine`, `knob` (1-4), `delta` (pas de 24 comptes, ~21 par tour, signé) | molette tournée |
| `press`, `release`, `click`, `hold` | `page`, `mine`, `button` : `knob1`…`knob4` (clics), `button1`…`button4`, `power` | appui, relâchement, clic court, maintien 0,5 s |
| `page_shown` | `page`, `shown` | votre page apparaît / disparaît |
| `deck` | `page`, `widget` (id), `tile` (index), `title` | une case d'un de vos widgets `deck` lancée (son action est déjà faite) |
| `command` | `command` (id), `delta` (crans de 24 comptes, signé : commande `knob`) | une de vos commandes utilisée : bouton (ou case de deck, power…) lié à elle, molette réglée dessus |
| `alert_closed` | `id`, `reason` : `timeout`, `dismissed` (clic), `standby` (écran en veille, `wake` faux), `disabled`, `replaced` (une alerte plus récente de la même application : une seule par application, en attente ou affichée ; application = `app`, sinon le programme) | fin d'une de vos alertes |
| `standby` | `asleep` | veille de l'écran (tous les programmes) |
| `media` | `player` (null : aucun), `status` (`Playing`, `Paused`, `Stopped`), `title`, `artist`, `album`, `art` (URL de la pochette : `file://` ou `https://`), `length`, `position` (s), `volume` | le lecteur suivi change (morceau, état), et chaque seconde pendant la lecture |

Lecteur suivi (`media.rs`) : l'app lit deux fois par seconde les lecteurs MPRIS de la session
(Spotify, navigateurs, VLC…) et suit celui qui joue (le même tant qu'il joue), sinon le dernier
suivi : les programmes n'ont pas besoin de D-Bus ni de `playerctl`.

**Toutes les entrées vont à tous les programmes connectés**, quelle que soit la page (`page` =
identifiant de la page d'un programme, ou nom de la page de la config). `mine: true` pour celui
qui la *prend* : il l'a réservée (`grab`) ou sa page est affichée ; les autres l'observent
(`mine: false`) et l'app continue d'appliquer ses liaisons (volume…) aux entrées que personne ne
prend. Sur une page d'API, power fait « retour » (il quitte le module) : vers la page d'où l'on
est venu par « afficher un module » / « aller à la page » / le menu des modules, sinon la dernière
page de la config affichée (on peut toujours en sortir), sauf `grab_power` ou `grab` de `power`.
« Page suivante / précédente » ne passent jamais par les pages d'API (pour y aller : menu des
modules, maintien de la molette 1 par défaut). Un clic pendant une alerte la ferme et n'est pas transmis.
Programme déconnecté : ses pages et ses réservations disparaissent ; si l'une était affichée,
l'appareil revient à la page de la config d'où on l'avait ouverte (l'écran principal) ; s'il
revient dans les 20 s (module relancé : code ou réglages changés) avec la même page et qu'aucune
autre n'a été choisie entre-temps, sa page est remontrée (power y garde son « retour »). Pour voir passer les
événements : `hsm01ctl events` (ou dans Bruno : se connecter suffit, aucun message à envoyer).

## Modules lancés par l'app (onglet « Modules »)
Un module = un **sous-dossier** de `~/.local/share/hsm01/modules/` (`module.toml` + programme), lu à la
volée (« ⟳ Rafraîchir les modules ») ; créer le sien : [modules.md](modules.md). La config ne garde
que l'interrupteur et les arguments : `[[module]]` (`name`, `enabled`, `module` = nom du dossier +
`args`, ou `command` lancée par `sh -c`). L'app les lance avec `HSM01_API_SOCKET`, `HSM01_MODULE`
(leur nom), `HSM01_MODULE_DIR` (leur dossier), `HSM01_LANG` (la langue de l'app, `en` par défaut ;
relancés quand elle change), les variables de leur `.env` (données sensibles
déclarées par `[[secret]]`, jamais dans la config : [modules.md](modules.md#données-sensibles-env))
et `PYTHONPATH` (bibliothèque Python), les relance s'ils s'arrêtent (1 s, 2 s, 4 s…
30 s au plus ; de nouveau 1 s après une minute de marche) ou si leur code change, les arrête
quand on les coupe, les retire ou quand l'app se ferme, et garde leurs 80 dernières lignes de
sortie (« Journal »). Un réglage changé s'applique après 1,2 s sans autre changement. Action
**« afficher un module »** (`{ type = "module", name = "Lecteur média" }`, sur n'importe quel clic,
maintien ou power) : montre la page du module (celle du programme qui s'est présenté sous ce
nom : la bibliothèque Python prend `$HSM01_MODULE` ; dans un autre langage, `hello` avec
`name = $HSM01_MODULE`) ; power y fait « retour ». Code :
`tools.rs`. Modules livrés (sources `app/modules/<id>/`, chacun copié **une fois** dans le dossier des
modules, au lancement qui suit son arrivée : liste `.livres` du dossier, un module supprimé ne revient
pas ; « Réinstaller les modules livrés » remet l'original) :
| Module (dossier) | Page | Commandes | Options |
|--------|------|-----------|---------|
| `media` « Lecteur média » | « Média » : pochette (téléchargée dans `~/.cache/hsm01/art`), titre, artiste, album, avancement ; en bas, une touche sous chacun des boutons 1-3 (⏮, ⏸ ou ▶ selon l'état, ⏭), qui s'éclaire à l'appui, et « volume » sous la molette 4 | bouton 1 / 2 / 3 : précédent, lecture-pause, suivant ; molette 1 : ±5 s par pas (clic : lecture-pause) ; molette 4 : volume du lecteur ; LED : bouton 2 allumé en lecture | `--alert` (alerte à chaque morceau ; réveille l'écran selon Appareil → Notifications), `--no-wake` (sans réveil : perdue pendant la veille), `--show`, `--name` |
| `system` « Système » | « Système » : processeur (% et °C), mémoire, carte graphique (°C), SSD (°C), disques, batteries ; lignes et ordre au choix ; 6 lignes à la fois, plus : barre de défilement et « 3-8 sur 9 » en tête | n'importe quelle molette : faire défiler les lignes | réglages `systeme.toml` (**onglet « Système »**) : par température (processeur, carte graphique, SSD) sonde (`auto` : Tctl / Package id 0, **edge** pour AMD et non junction, nvidia-smi, Composite ; ou « puce/étiquette » listée dans le Journal au démarrage), seuil, `pendant` (s tenus au-dessus avant l'alerte), `retour` (°C sous le seuil pour réarmer), `rappel` (min) ; disque (% , `ignorer`), batterie (%, `ignorer`) ; `style` carte ou pilule, durée, LED, réveil ; options `--no-page`, `--show` (anciennes `--cpu-temp`… : priment sur le fichier) |
| `deck` « Menu de raccourcis » | menu façon Stream Deck (exemple à adapter : `modules/deck/main.py`) ; aussi dans un widget « module » d'une page de la config | molette : rangées ; boutons : cases | — (lancé par l'app, il ne prend pas l'écran au démarrage) |
| `streamdeck` « Stream deck » | 4 pistes de cases sans fin, une par molette, comme la maquette `concept/` : 4 widgets `deck` (l'app les dessine et les exécute) ; disposition (`layout` : `colonnes`, piste k au-dessus de la molette k ; `lignes`, une ligne par molette), pistes, cases et actions dans `deck.toml` du dossier, éditable dans **son onglet « Stream deck »** (module activé) ; relance à l'enregistrement | molette k : choisir dans la piste k ; clic : lancer ; power : retour | `--layout` (si `deck.toml` n'en dit rien), `--file`, `--show` |
| `motortown` « Motor Town », `simhub` « SimHub » | tableau de bord de jeu commun (`hsm01.dashboard`) : compte-tours, rapport, vitesse, pédales, carburant, températures, voyants, écrans Course et Véhicule ; télémétrie UDP de Motor Town (natif v1.0-1.2, compatible Forza) ; SimHub : flux « compatible SimHub » du jeu (Forza Data Out, UDP 8888) ou API web du programme SimHub : [telemetrie.md](telemetrie.md) ; widget « Tableau de bord » | molette 1 : écran (Conduite, Course, Véhicule) ; boutons : touches envoyées au jeu (réglables) ; LEDs : compte-tours | `--show` ; réglages `motortown.toml`, `simhub.toml` |
| `scores` « Live scores » | les matchs en cours du flux de L'Équipe, par sport et compétition (domicile, score, extérieur, moment) ; filtre sports / compétitions / équipes ; lu toutes les minutes avec un match du filtre en cours, sinon toutes les 5 min : [scores.md](scores.md) ; widget « Live scores » ; pilule au changement de score | molette 1 : défiler ; clic molette 1 : relire ; commande `refresh` |
| `obs` « OBS » | scènes (programme, aperçu), mixeur audio d'OBS (3 sources : nom, vumètre, curseur, dB), durées du direct et de l'enregistrement ; serveur WebSocket d'OBS (v5), port et mot de passe lus dans ses réglages : [obs.md](obs.md) ; widget « État d'OBS » | molette 1 : scène (clic : l'afficher, mode studio : aperçu puis transition) ; molettes 2-4 : volume (clic : sourdine) ; boutons au choix (par défaut direct, enregistrement - arrêt par maintien -, pause, replay) ; commandes `direct`, `enregistrement`, `pause`, `replay`, `camera`, `transition`, `studio`, `scenes`, `scene:…`, `couper:…`, `volume:…`, `profil:…`, `collection:…`, favoris `source:…`, `filtre:…`, `media:…` | `--show` ; réglages `obs.toml` (onglet « OBS ») |
| `spotify` « Spotify » | pochette, titre, artiste, album, avancement ; API web de Spotify avec **votre** application Spotify Developer (ID et secret en `[[secret]]`, jeton dans `.env`) : [spotify.md](spotify.md) | molette 1 : position (clic : lecture / pause) ; clic molette 2 : aléatoire, 3 : répétition ; molette 4 : volume (clic : sourdine) ; boutons au choix (par défaut précédent, lecture / pause, suivant - maintien : radio du titre -, J'aime ; aussi radio du titre, aléatoire, répétition, sourdine) ; commandes `play_pause`, `next`, `previous`, `like`, `shuffle`, `repeat`, `mute`, `radio`, `volume`, listes favorites | `--show` ; réglages `spotify.toml` (onglet « Spotify ») |
| `discord` « Discord » | le salon vocal (membres en cases : avatar rond, qui parle, micro coupé / sourd, volume) ou, hors salon, les salons favoris ; RPC du client Discord officiel avec **votre** application (ID et secret en `[[secret]]`, jeton dans `.env`) : [discord.md](discord.md) | molette 1 : membre (clic : le couper pour soi) ; 2 : son volume ; 3 / 4 : mon entrée / ma sortie (clic : micro / sourd) ; boutons : micro, sourd, push-to-talk, quitter ; commandes `mute`, `deafen`, `input_volume`, `output_volume`, `disconnect`, `voice_mode`, salons favoris, membres suivis | `--show` ; réglages `discord.toml` (onglet « Discord ») |

Capteurs du module Système : `/proc/stat`, `/proc/meminfo`, `/sys/class/hwmon` (`k10temp`,
`coretemp`, `amdgpu`, `nvme`…), `nvidia-smi` s'il existe, `/proc/mounts`, `/sys/class/power_supply`.

## Modules fournis
- **`hsm01ctl`** (compilé avec l'app, `app/target/release/hsm01ctl`) :
  `hsm01ctl alert "Compilation finie" "0 erreur" --icon dialog-ok --led 2 --wait` (attend la
  fermeture et affiche la raison) ; `hsm01ctl send '{"type": …}' [--wait s]` (requête brute) ;
  `hsm01ctl events [--grab knob4,button4]` (affiche tous les événements jusqu'à Ctrl+C).
- **Bibliothèque Python** `app/clients/python/hsm01` (bibliothèque standard seulement) :
  `Client` (requêtes ; événements via `on`, exécutés dès leur arrivée par un fil dédié, même
  sans `run` ; `run(tick=…)` pour les animations ; `grab` / `ungrab`), `widgets` (fonctions d'aide),
  `menu.GridMenu` (menu façon Stream Deck : grille 4 colonnes, défilement infini animé par
  rangées, bouton k = case k de la rangée active, dossiers, textes vivants, LED).
- Exemples : les modules livrés (`app/modules/`), `app/clients/python/examples/alert.py "Titre"
  "Texte"`, modèle de module `examples/tool_template.py`, page web `app/clients/web/alert.html`
  (WebSocket).

```python
from hsm01 import Client, widgets as W
c = Client('mon-module')
c.page('p', [W.text('t', 0, 100, 480, 40, 'Bonjour', size=28)], show=True)
c.on('turn', lambda e: c.update('p', {'t': {'text': f'molette {e["knob"]} : {e["delta"]}'}}))
c.run()
```

## Tests
`cargo test --release -p hsm01-app api` : JSON des widgets, requêtes sur un vrai socket, WebSocket
(jeton exigé, `grab`, arrêt à chaud) ; `inputs_reach_the_api_clients` : routage des entrées
(observées par tous, prises par la page affichée ou par `grab`, power gardé par l'app). `-- --ignored python_menu` : le menu Python contre le serveur, sans appareil
(défilement d'une rangée, clic → alerte, boucle infinie).
