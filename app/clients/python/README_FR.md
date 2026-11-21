# hsm01 : la bibliothèque Python des modules de new hera-nux

[English version](README.md)

Un **module** est un petit programme qui ajoute des pages, des widgets, des alertes et des
commandes à new hera-nux, l'application compagnon du Hercules Stream 100. L'app le lance, lui
donne les molettes et les boutons de l'appareil, et dessine ce qu'il demande. Cette bibliothèque
(`hsm01`, Python 3.11 ou plus, **bibliothèque standard seulement**) est celle des modules livrés
(Discord, Spotify, OBS, Motor Town, SimHub, scores en direct…) ; n'importe qui peut s'en servir
pour en écrire d'autres.

Sommaire : [Démarrage rapide](#démarrage-rapide) · [Le dossier d'un module](#le-dossier-dun-module) ·
[Ce que l'app donne au programme](#ce-que-lapp-donne-au-programme) · [L'écran et ses widgets](#lécran-et-ses-widgets) ·
[Client](#client) · [Événements](#événements) · [Fils d'exécution](#fils-dexécution) ·
[Données sensibles](#données-sensibles--hsm01secrets) · [Traductions](#traductions--hsm01i18n) ·
[Aides toutes faites](#aides-toutes-faites) · [Bonnes pratiques](#bonnes-pratiques) · [Tester](#tester)

## Démarrage rapide
1. Dans la fenêtre de l'app, onglet **Modules** : taper un nom, puis **Créer un module**. Un
   dossier s'ouvre avec `module.toml`, `main.py` (un exemple qui marche : une page, un compteur
   sur la molette 1, une alerte sur le bouton 1, deux commandes) et `i18n/en.toml`, `fr.toml`.
2. Allumer le module sur sa fiche. Maintenir la molette 1 sur l'appareil : le **menu des
   modules** propose sa page.
3. Modifier `main.py` et enregistrer : l'app relance le module toute seule. Ce qu'il affiche va
   dans le **Journal** de sa fiche.

Le plus petit module :
```python
from hsm01 import Client
from hsm01 import widgets as W

client = Client('bonjour')                 # lancé par l'app : le nom de sa fiche à la place
client.page('main', [
    W.text('title', 0, 100, 480, 40, 'Bonjour', size=28, bold=True),
], name='Bonjour')


@client.on('click')
def click(ev):
    if ev['mine'] and ev['button'] == 'button1':   # bouton 1, pendant que notre page est affichée
        client.pill('BOUTON 1')


client.run()                               # jusqu'à la fermeture de l'app
```
Pour essayer un programme à la main pendant que l'app tourne (hors de l'onglet Modules) :
`PYTHONPATH=<dépôt>/app/clients/python python3 main.py`.

## Le dossier d'un module
```
~/.local/share/hsm01/modules/          ($XDG_DATA_HOME/hsm01/modules)
└── mon-module/                        ← son identifiant (le nom du dossier)
    ├── module.toml                    ← sa description
    ├── main.py                        ← son programme
    ├── i18n/en.toml, fr.toml          ← ses textes en anglais, en français
    ├── mon-module.toml                ← ses réglages, s'il en a (nom libre)
    ├── .env                           ← ses données sensibles, à vous seul (écrit par l'app)
    └── …                              ← tout ce dont il a besoin (images, données)
```
`module.toml` :

| Clé | Sens |
|-----|------|
| `name` | nom affiché dans l'onglet Modules (et par l'action « afficher un module ») |
| `description` | texte sous le nom |
| `file` | programme Python, relatif au dossier (lancé par `python3`) |
| `command` | ou n'importe quel programme : `["node", "index.js"]`, `["./mon-module"]` (relatifs au dossier) |
| `args` | arguments par défaut (modifiables dans l'onglet Modules) |
| `[settings]` | `file` (son fichier de réglages, TOML, dans le dossier), `editor` (`formulaire` : un formulaire de `field` ; `texte` : le texte TOML ; `deck` : l'éditeur du stream deck), `guide` (texte en haut de son onglet, un paragraphe par ligne, les adresses deviennent des liens) — tant que le module est allumé, la fenêtre lui donne **un onglet à lui** |
| `[[settings.field]]` | un champ du formulaire (ci-dessous) |
| `[[secret]]` | une valeur sensible : `name` (variable d'environnement), `label`, `description`, `hidden` (affichée en points) — saisie dans son onglet, gardée dans son `.env` |

Champs du formulaire de réglages (`[[settings.field]]`), enregistrés dans le fichier de réglages
sous `key` (avec des points pour les tables imbriquées : `alertes.cpu.seuil`), avec `label` et
`help` (au survol) :

| `kind` | Clés en plus | Affiché comme |
|--------|--------------|---------------|
| `number` | `min`, `max`, `step`, `unit`, `default`, `optional` (peut rester vide) | un nombre |
| `text` | `default` | une ligne de texte |
| `bool` | `default` | un interrupteur |
| `choice` | `choices` (valeurs enregistrées) et `choice_labels` (textes affichés, même ordre, traduits), ou `source` (une liste que le module lancé donne par `client.choices` ; `{clé}` = la valeur d'un champ voisin : `source = "salons:{serveur}"`), `optional` | une liste déroulante (saisie à la main tant que le module est arrêté) |
| `list` | `field` (les champs de chaque élément), `add` (son bouton) | des éléments ajoutés et retirés (tableau de tables) |
| `section` | — | un titre |

Enregistrer le formulaire relance le module : il lit son fichier de réglages une fois, au
démarrage (`tomllib.load`). Les modules livrés sont des exemples complets :
`app/modules/*/module.toml`.

## Ce que l'app donne au programme
| Variable d'environnement | Contenu |
|--------------------------|---------|
| `HSM01_API_SOCKET` | le socket de l'API (`Client` s'en sert) |
| `HSM01_MODULE` | le nom du module dans l'onglet Modules : `Client` le dit dans `hello`, ainsi « afficher un module » et le menu des modules trouvent sa page |
| `HSM01_MODULE_DIR` | son dossier |
| `HSM01_LANG` | la langue de l'app (`en` par défaut, `fr`…) |
| les lignes de son `.env` | ses données sensibles, une variable chacune |
| `PYTHONPATH`, `HSM01_PYLIB` | cette bibliothèque : `import hsm01` marche tel quel |
| `PYTHONUNBUFFERED=1` | les `print` arrivent tout de suite dans le Journal |

Vie d'un module : lancé quand son interrupteur est allumé ; relancé s'il s'arrête (1 s, 2 s,
4 s… 30 s au plus), quand son code change, quand ses réglages ou ses données sensibles sont
enregistrés, et quand la langue de l'app change ; arrêté quand on le coupe ou le retire, ou
quand l'app se ferme. **Il doit s'arrêter quand la connexion se ferme** : `client.run()` rend
alors la main. Sa fiche a aussi **« pas de veille quand il est affiché »** : l'écran ne s'éteint
pas tant que sa page (ou un de ses widgets) est à l'écran.

## L'écran et ses widgets
480 × 272 pixels, origine en haut à gauche. Couleurs `'#rrggbb'` ; un **dégradé** est une liste
de couleurs (début → fin), partout où une couleur est acceptée.
`from hsm01 import widgets as W` ; chaque fonction rend un dict avec un `id` (pour le changer ou
le retirer ensuite) :

| Fonction | Dessine |
|----------|---------|
| `W.text(id, x, y, w, h, text, size=18, color, align='center', bold=False, scroll=True)` | une ligne de texte ; trop longue : défile en aller-retour (`scroll`), ou est coupée avec « … » |
| `W.rect(id, x, y, w, h, color, radius=0)` | un rectangle, coins arrondis |
| `W.separator(id, x, y, w, h, color, thickness=2, vertical=False, style='solid', fade=False)` | un trait : `solid`, `dashed`, `dotted` ; `fade` : extrémités en fondu |
| `W.icon(id, x, y, w, h, name, tint=None)` | une icône du thème du bureau par nom (`'audio-input-microphone'`, `'firefox'`) ou un fichier image ; `tint` : monochrome dans cette couleur |
| `W.image(id, x, y, w, h, path, radius=0)` | un fichier PNG ajusté au cadre ; `radius` arrondit les coins (la moitié du cadre : un disque, pour les avatars). Images gardées en cache **par chemin** : donner un nouveau chemin pour changer l'image |
| `W.deck(id, x, y, w, h, knob, tiles, vertical=True, color)` | une piste de stream deck gérée par l'app : les `tiles` (`W.tile(title, subtitle, icon, action)`) défilent sans fin avec la molette `knob`, son clic lance l'action de la case et vous envoie un événement `deck` |

Tous les widgets sauf `deck` acceptent `frame=W.frame(border, width=2, fill=None, radius=8,
padding=4, sides=(haut, droite, bas, gauche))` : une bordure, un fond facultatif, le contenu en
retrait.

## Client
`client = Client(nom, path=None, timeout=5.0)` se connecte (lève `ConnectionError` si l'app ne
tourne pas) et dit bonjour ; `client.welcome` garde la réponse de l'app (`api`, `app`, `screen`
{width, height}, `firmware`). Une requête refusée lève `ApiError` ; `call` attend `timeout`
secondes au plus (`TimeoutError`).

**Pages et widgets**

| Méthode | Fait |
|---------|------|
| `page(page, widgets, name=None, background=None, show=False, grab_power=False, leds=None)` | crée ou remplace votre page `page` (dans le menu des modules sous le nom `name`) ; `show` : à l'écran tout de suite ; `leds` : 4 × % tant qu'elle est affichée ; `grab_power` : power vous revient au lieu de « retour » |
| `widget(id, w, h, widgets, name=None, background=None)` | un **widget du module** : contenu de `w` × `h` px dans ses propres coordonnées, que l'utilisateur place sur ses pages (onglet Widgets → « module » → votre module → montrer : ce widget), réduit ou défilé selon son cadre ; le rappeler pour une nouvelle taille |
| `update(page, {id: {champ: valeur}})` | change quelques champs de quelques widgets (texte, couleur, position…) : peu coûteux, pour les valeurs en direct et les animations |
| `set(page, *widgets)` | ajoute des widgets, ou remplace ceux de même id |
| `remove(page, *ids)` | retire des widgets |
| `leds(page, values)` | les 4 LEDs des boutons (%) tant que la page est affichée |
| `show(page)` | affiche votre page sur l'appareil |
| `close_page(page)` | supprime la page |

**Alertes**

| Méthode | Fait |
|---------|------|
| `alert(title, text='', icon=, image=, round=, app=, duration=, led=, priority='high', wake=)` | une carte en haut de l'écran ; rend son id (`alert_closed` dit quand elle part). `icon` : nom du thème ou chemin ; `image` : une photo (`round` : un disque) ; `app` : une alerte par application dans la file ; `led` : 1-4 clignote ; `wake` : rallume l'écran |
| `pill(text, border='#3ddc84', dot=None, color='#ffffff', duration=3, …)` | un bandeau court en bas de l'écran (une ligne, en gras), bordé de `border` |

**Commandes** : proposées à l'utilisateur tant que le module tourne, utilisables sur **toutes**
les pages (action « commande de module » d'un bouton, cible « commande de module » d'une
molette, LED « état d'une commande de module »).

| Méthode | Fait |
|---------|------|
| `@client.command(id, name, kind='button', description='')` | décorateur : `fn(ev)` s'exécute quand elle est utilisée ; `kind='knob'` : `ev['delta']` = crans signés |
| `commands([(id, name, fn), (id, name, fn, 'knob', description)…], replace=False)` | plusieurs d'un coup ; `replace=True` : listes qui changent (scènes, listes de lecture) |
| `command_state(id, active=None, value=None, text=None)` | ce que l'app en montre : `active` (LED allumée ; `False` grise les widgets d'une molette), `value` 0..1 (jauges), `text` (le `{name}` d'une molette) |

**Autres**

| Méthode | Fait |
|---------|------|
| `choices(**listes)` | les listes des champs `choice` de votre formulaire avec `source = "nom"` : `choices(serveurs=['A', 'B'])` ; noms que Python refuse : `choices(**{'salons:A': […]})` |
| `grab(*entrées)` / `ungrab(*entrées)` | prendre `'knob1'`…`'knob4'`, `'button1'`…`'button4'`, `'power'` sur toutes les pages (`mine` vrai, les liaisons de l'app n'agissent plus) / les rendre |
| `action(action)` | exécute une action de l'app, écrite comme dans `config.toml` : `{'type': 'keys', 'keys': 'ctrl+alt+t'}`, `{'type': 'media', 'action': 'next'}`, `{'type': 'page', 'index': 2}`… (programmes locaux seulement) |
| `call({'type': 'media'})` | le lecteur que suit l'app (`player`, `status`, `title`, `artist`, `album`, `art`, `length`, `position`, `volume`) ; `{'type': 'media', 'action': 'play_pause'}` agit dessus |
| `run(tick=None, fps=60)` | attend que l'app parte (ou Ctrl+C), en appelant `tick(dt)` `fps` fois par seconde s'il est donné (horloges, animations) |
| `post(msg)` / `call(msg)` | n'importe quelle requête du protocole, sans / avec attente de la réponse |

## Événements
`@client.on(événement)` (ou `client.on(événement, fn)`) appelle `fn(ev)` à chaque événement ;
`'*'` pour tous. **Toutes les entrées vont à tous les programmes**, quelle que soit la page :
`ev['mine']` est vrai pour celui qui la prend (sa page est affichée, ou il a réservé l'entrée),
et c'est seulement alors que les liaisons de l'app la laissent tranquille. `ev['page']` :
l'identifiant de votre page, ou le nom d'une page de la config de l'utilisateur.

| Événement | Champs | Quand |
|-----------|--------|-------|
| `turn` | `page`, `mine`, `knob` (1-4), `delta` (crans signés, ~21 par tour) | une molette tourne |
| `press`, `release`, `click`, `hold` | `page`, `mine`, `button` : `knob1`…`knob4` (clics des molettes), `button1`…`button4`, `power` | appui, relâchement, clic court, maintien 0,5 s |
| `page_shown` | `page`, `shown` | votre page apparaît / part |
| `deck` | `page`, `widget`, `tile`, `title` | une case d'un de vos widgets `deck` a été lancée |
| `command` | `command`, `delta` | une de vos commandes a été utilisée (géré par `@client.command`) |
| `alert_closed` | `id`, `reason` (`timeout`, `dismissed`, `standby`, `disabled`, `replaced`) | une de vos alertes est partie |
| `standby` | `asleep` | l'écran se met en veille / se réveille |
| `media` | comme `call({'type': 'media'})` | le lecteur suivi change, et chaque seconde pendant la lecture |
| `error` | `message` | une requête envoyée par `post` a été refusée |
| `disconnected` | — | l'app est partie |

Sur votre page, l'app garde deux entrées : **maintenir la molette 1** ouvre le menu des modules,
**power** fait « retour » (sauf `grab_power` ou `grab('power')`). Un clic pendant une alerte la
ferme et n'est pas transmis.

## Fils d'exécution
Un fil de lecture reçoit les messages de l'app ; **les gestionnaires s'exécutent un par un** dans
un fil de répartition, en tenant `client.lock` ; le `tick` de `run` le tient aussi, donc
gestionnaires et `tick` ne tournent jamais en même temps. Les méthodes du client s'appellent
depuis n'importe quel fil. Si vos propres fils (une boucle réseau, un socket) changent un état
que lisent les gestionnaires, protégez-le par un verrou à vous, et enregistrez les gestionnaires
dans `with client.lock:` quand des événements peuvent déjà arriver.

## Données sensibles : hsm01.secrets
Jetons, mots de passe, clés : déclarés par `[[secret]]` dans `module.toml`, saisis dans l'onglet
du module, gardés dans son `.env` (à vous seul), donnés comme variables d'environnement. Jamais
dans `config.toml`, le code ou les journaux.

| Fonction | Fait |
|----------|------|
| `secrets.get(nom, default=None)` | une valeur : l'environnement, sinon le `.env` (enregistrée entre-temps) |
| `secrets.save(nom, valeur)` | garde une valeur obtenue en cours de route (un jeton OAuth…) pour les lancements suivants, sans relancer ; vide : la retire |
| `secrets.read()`, `secrets.path()` | toutes les valeurs ; le chemin du `.env` |

## Traductions : hsm01.i18n
Écrire les textes en anglais et les donner à `tr` ; `i18n/<code>.toml` dans le dossier du module
les donne dans chaque langue, choisie dans l'app (onglet Appareil), transmise par `HSM01_LANG` :
```python
from hsm01.i18n import tr, point
tr('No live match')                # « Aucun match en cours » en français
tr('{n} members', n=3)             # mêmes {paramètres} dans toutes les langues
point('2.4 km')                    # décimales à la façon de la langue : « 2,4 km » en français
```
```toml
# i18n/fr.toml                       (en.toml : "texte du code" = "anglais affiché")
name = "Français"

[text]
"No live match" = "Aucun match en cours"
"{n} members" = "{n} membres"
```
L'app traduit le `module.toml` du module avec le même fichier : `name`, `description`, le
`guide` ligne par ligne, les `label`, `help`, `unit`, `add`, `choice_labels` du formulaire, et
les `label`, `description` des `[[secret]]`. Gardez les valeurs enregistrées dans les réglages
(`choices`, clés) dans une seule langue : ce sont des identifiants. `en.toml` contient l'anglais
lui-même (changer une valeur le reformule) ; le copier pour commencer une autre langue. Pour les
modules livrés, `python3 -I tools/i18n_check.py fr` vérifie qu'il ne manque rien, et
`python3 -I tools/i18n_check.py en --fill` réécrit les fichiers anglais.

## Aides toutes faites
| Module | Pour |
|--------|------|
| `hsm01.menu` | `GridMenu(client, tiles, page='menu', title='Menu', columns=4, rows=2)` : une grille à la Stream Deck de `Tile(title, icon, action=fn(menu), hold=fn, folder=[…], color)`, une colonne par bouton, rangées défilées par les molettes, dossiers ; `title` et `icon` peuvent être des fonctions (horloges, états). Exemple : `app/modules/deck/main.py` |
| `hsm01.dashboard` | `Dashboard(client, name, page='tableau', hints, options)` : un tableau de bord de voiture (vitesse, régime, rapport, pédales, carburant, voyants, écrans course et véhicule, compte-tours sur les LEDs, pilules) ; lui donner des échantillons par `feed(sample)` (clés dans sa docstring), `message(status, lines)` quand rien n'arrive, `client.run(tick=dash.tick, fps=20)` |
| `hsm01.udp` | télémétrie de jeu en UDP : `Receiver(host, port, relays)` (`recv(timeout)` → `(data, addr)` ou None, chaque datagramme aussi relayé vers `relays`), `settings(table, port_par_défaut)` → IP d'écoute, port, relais et les problèmes en mots, `ip`, `port`, `this_pc()` |
| `hsm01.forza` | le format UDP « Forza Data Out » que beaucoup de jeux copient : `decode(data)` → champs, `sample(raw, game)` → un échantillon du tableau de bord |

## Bonnes pratiques
- **N'envoyer que ce qui change** : garder ce qui a été envoyé et donner à `update` les champs qui
  diffèrent (les modules livrés le font pour toutes leurs pages et widgets). L'app n'envoie à
  l'appareil que les pixels qui changent, mais chaque requête a un coût.
- Construire la page une fois avec tous ses widgets (textes vides au besoin), puis l'`update`.
- Pour changer une image, l'écrire sous un nouveau chemin (l'app garde les images par chemin).
- Démarrer vite, puis faire le travail lent (réseau, appareils) dans un fil ; afficher un
  texte « connexion… » en attendant.
- Des `print` courts et utiles : c'est le Journal du module. Jamais de jeton dedans.
- S'arrêter quand la connexion se ferme (`client.run()` rend la main) : l'app relance le module
  quand elle revient.
- Interroger les services web avec modération (le module de scores lit son flux une fois par
  minute au plus), et dire qui vous êtes dans un `User-Agent`.

## Tester
- `PYTHONPATH=<dépôt>/app/clients/python python3 main.py` pendant que l'app tourne : les erreurs
  dans le terminal. `hsm01ctl events` montre chaque événement envoyé par l'app.
- Les modules livrés sont testés contre de faux serveurs (`tools/fake_*.py` : Discord, Spotify,
  OBS, jeux, scores en direct) par les tests de bout en bout de l'app
  (`app/hsm01-app/src/api.rs`, `cargo test --release -- --ignored`) : un modèle à copier pour les
  vôtres.

Dans un autre langage que Python : le programme parle en lignes JSON sur le socket Unix ; le
protocole est décrit dans [docs/app/api.md](../../../docs/app/api.md). Exemples :
[examples/alert.py](examples/alert.py), [examples/tool_template.py](examples/tool_template.py),
et les modules livrés dans [app/modules/](../../modules/).
