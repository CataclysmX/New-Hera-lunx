# Créer un module pour new hera-nux

Guide de la bibliothèque Python (toutes ses fonctions, en français et en anglais) :
[app/clients/python/README_FR.md](../../app/clients/python/README_FR.md).

Un **module** est un programme qui parle à l'app par l'[API ouverte](api.md) : il affiche une page
sur le Stream 100, reçoit molettes et boutons, envoie des alertes, pilote le lecteur média…
**Tous les modules sont des sous-dossiers d'un dossier externe**, `~/.local/share/hsm01/modules/`
(`$XDG_DATA_HOME/hsm01/modules`), lus à la volée : rien n'est compilé dans l'app. L'onglet
« Modules » montre un module par sous-dossier ; **⟳ Rafraîchir les modules** relit le dossier (modules
ajoutés, modifiés, supprimés). La fenêtre relit aussi seule un `module.toml` modifié (toutes les 2 s : un module mis à jour
montre son nouveau formulaire sans rafraîchir). L'app lance ceux dont l'interrupteur est allumé, les relance s'ils
s'arrêtent **ou si un de leurs fichiers de code change**, et les arrête avec elle.

Les modules livrés (Lecteur média, Système, Menu de raccourcis, Stream deck, Discord, Spotify, OBS, Motor Town, SimHub ; sources dans `app/modules/`) sont
**copiés dans ce dossier au premier lancement** : on peut les modifier comme les siens.
« Réinstaller les modules livrés » remet leur version d'origine (les changements faits dans leurs
dossiers sont perdus). « + Commande » lance un programme déjà installé (`sh -c`) sans dossier.

## Le plus rapide : depuis la fenêtre
Onglet **Modules** → taper un nom → **Créer un module**. L'app crée
`~/.local/share/hsm01/modules/<nom>/` avec `module.toml` et `main.py` (modèle : une page « Mon module »,
la molette 1 compte, le bouton 1 envoie une alerte), l'allume et ouvre son dossier. Ensuite :
1. modifier `main.py` et enregistrer le fichier : **le module se relance tout seul** ;
2. lire le **Journal** du module (ce que le programme affiche, et ses erreurs Python) ;
3. **Enregistrer** dans la fenêtre pour garder l'interrupteur allumé au prochain lancement.

Un dossier copié à la main (module reçu, téléchargé) apparaît après « Rafraîchir les modules ».

Pour montrer la page du module : **maintenir la molette 1** ouvre le menu des modules (un bandeau
qui monte du bas de l'écran, listant les modules qui ont une page), tourner la molette 1 choisit,
son **clic** valide ; un autre clic ou 10 s sans rien le ferme. C'est l'action « menu des modules »
(`type = "module_menu"`), liable à n'importe quel bouton, clic ou maintien. Pour un module précis
d'un appui : onglet Commandes → un bouton → « afficher un module ».
Sur la page du module, **power quitte le module** (retour à la page d'où l'on vient). Les pages de
module ne sont pas dans le cycle « page suivante / précédente », qui ne passe que par celles de la
config.

Pour la montrer **dans une de ses pages** : onglet Widgets → « + Ajouter un widget » → « module (sa
page) », choisir le module, placer et dimensionner le cadre (la page y est réduite), cocher les
entrées qui vont au module sur cette page (par défaut molette 4 et boutons 1-4). Le module reçoit
alors ces entrées comme sur sa propre page ([companion.md](companion.md), widget Module).

**Widgets de modules** : un module peut aussi proposer des widgets, à placer sur ses pages — par
exemple Discord : « Membres du salon vocal, en colonne » et « …, en ligne ». Onglet Widgets →
« module » → le module → **montrer** : le widget ; **trop grand** : « réduire » (tout tient, plus
petit), « défiler ↕ » (taille gardée, la liste défile de haut en bas toute seule) ou « défiler ↔ »
(pour une liste en ligne : elle défile de gauche à droite). En Python :
```python
c.widget('membres', 220, 40 * len(noms), [W.text(f'n{i}', 8, 40 * i, 204, 40, n) for i, n in
                                          enumerate(noms)], name='Membres')
c.update('membres', {'n0': {'color': '#23a55a'}})   # comme une page ; autre taille : widget()
```

## Le dossier d'un module
```
~/.local/share/hsm01/modules/        ($XDG_DATA_HOME/hsm01/modules)
└── minuteur/                      ← identifiant du module (nom du dossier)
    ├── module.toml                  ← description
    ├── main.py                    ← le programme
    ├── .env                       ← ses données sensibles (jetons…), à vous seul (0600)
    ├── i18n/en.toml, fr.toml      ← ses textes en anglais et en français (voir « Langue des modules »)
    └── …                          ← ce qu'il veut (images, données)
```
`module.toml` :
| Clé | Sens |
|-----|------|
| `name` | nom affiché dans l'onglet Modules (et pour l'action « afficher un module ») |
| `description` | texte sous le module |
| `file` | programme Python, relatif au dossier (lancé par `python3`) |
| `command` | ou n'importe quel programme : liste `["node", "index.js"]`, `["./mon-module"]` (chemins relatifs au dossier, nom seul = programme du `PATH`) |
| `args` | arguments par défaut (modifiables ensuite dans l'onglet Modules) |
| `[settings]` | facultatif : `file` (fichier de réglages du dossier) et `editor` (`deck` : éditeur de pistes du Stream deck ; `texte`, par défaut : le TOML) — tant que le module est activé, la fenêtre lui ajoute **un onglet à son nom** pour ce fichier |
| `[[secret]]` | facultatif, un par donnée sensible : `name` (variable d'environnement), `label`, `description`, `hidden` (`true` : affichée en points) — champs de son onglet, gardés dans son `.env` ([plus bas](#données-sensibles-env)) |

Un ancien dossier `~/.local/share/hsm01/tools/` est renommé `modules/` au lancement, et `tool.toml`
en `module.toml` (encore lu tel quel s'il reste).

```toml
name = "Minuteur"
description = "Compte à rebours : molette 1 règle, bouton 1 démarre, alerte à la fin."
file = "main.py"
args = ["--minutes", "25"]
```
Le programme démarre **dans son dossier** (chemins relatifs possibles). Relance automatique : un
fichier de code changé (`.py`, `.js`, `.mjs`, `.ts`, `.sh`, `.lua`, `.rb`, `.pl`, `.php`, `.toml`,
y compris dans les sous-dossiers ; pas `__pycache__`, `node_modules` ni les dossiers cachés) relance
le module dans la seconde ; les fichiers de données qu'il écrit (`.json`, images…) ne le relancent
pas. Les réglages de l'interrupteur et des arguments sont dans la config de l'app (`[[module]]`,
`module` = nom du dossier), pas dans le dossier du module.

Onglet de réglages (`[settings]`) : après « Modules », un onglet par module activé qui en déclare un
(il disparaît quand on coupe le module). Éditeur `texte` : le fichier, vérifié (TOML valide) avant
« Enregistrer » ; `deck` : disposition, pistes, cases et leurs actions, écrit dans le fichier en
gardant ses commentaires. Enregistrer change un `.toml` du dossier : le module se relance et relit
ses réglages. Le fichier modifié à la main pendant que l'onglet est ouvert (sans modification en
cours) est relu.

**Formulaire** (`editor = "formulaire"`, conseillé) : le module décrit ses champs, la fenêtre affiche
de vrais contrôles et écrit le fichier en gardant ses commentaires (modules Discord et Système) :
```toml
[settings]
file = "reglages.toml"
editor = "formulaire"
guide = """
Un paragraphe par ligne ; les adresses https://… deviennent des liens.
"""

[[settings.field]]
kind = "section"                # un titre
label = "Alertes"

[[settings.field]]
key = "alertes.seuil"            # clé pointée : tables imbriquées du fichier
label = "Seuil"
kind = "number"                  # number (min, max, step, unit, optional), text, bool
min = 40
max = 110
unit = "°C"
default = 90                     # montré tant que la clé n'est pas dans le fichier
help = "au survol"

[[settings.field]]
key = "salon"                    # list : tableau de tables [[salon]], ses champs dessous
kind = "list"
label = "Salons favoris"
add = "+ Ajouter un salon"

  [[settings.field.field]]
  key = "serveur"
  kind = "choice"                # choices = ["a", "b"], ou source : liste donnée par le module
  source = "serveurs"

  [[settings.field.field]]
  key = "salon"
  kind = "choice"
  source = "salons:{serveur}"    # {clé} : la valeur d'un champ voisin
```
Les listes `source` viennent du module lancé : `c.choices(serveurs=[…], **{'salons:A': […]})` en
Python (requête `choices`, [api.md](api.md)) ; module arrêté : un champ texte à la place. Les clés
que le formulaire ne montre pas restent telles quelles dans le fichier.

## Ce que l'app donne au programme
| Variable | Contenu |
|----------|---------|
| `HSM01_API_SOCKET` | chemin du socket de l'API |
| `HSM01_MODULE` | nom du module dans l'onglet Modules : à donner comme `name` dans `hello` (la bibliothèque Python le fait seule) ; c'est ainsi que « afficher un module » trouve sa page (`HSM01_TOOL` : même valeur, pour les anciens modules) |
| `HSM01_MODULE_DIR` | son dossier |
| `HSM01_LANG` | la langue choisie dans l'app (`en` par défaut, `fr`…) ; le module est relancé quand elle change |
| les lignes de son `.env` | ses données sensibles, une variable par ligne `NOM=valeur` |
| `PYTHONPATH`, `HSM01_PYLIB` | dossier de la bibliothèque Python : `from hsm01 import Client` marche tel quel |
| `PYTHONUNBUFFERED=1` | les `print` arrivent tout de suite dans le Journal |

Vie du module : lancé quand son interrupteur est allumé ; relancé s'il s'arrête (1 s, 2 s, 4 s…
30 s au plus) ou si son code change ; tué quand on le coupe, le retire ou quitte l'app. Il doit **s'arrêter quand la
connexion se ferme** (`Client.run()` le fait) : l'app relancée le relancera.
Sa fiche (onglet Modules) a aussi la case **Veille : pas de veille quand il est affiché**
(Discord pendant les appels, OBS en direct, un tableau de bord de jeu…) : l'écran ne s'éteint
pas tant que la page du module (ou son widget dans la page) est affichée ; sur une autre page,
la veille revient normalement (`keep_awake` dans `[[module]]`).

## Langue des modules
Les textes d'un module s'écrivent en anglais ; `i18n/<code>.toml` dans son dossier les donne dans une
langue, au même format que les fichiers de l'app. **`en.toml`** contient les textes anglais
eux-mêmes (`"texte du code" = "texte affiché"`) : changer une valeur reformule l'anglais, et c'est
le fichier à copier pour commencer une autre langue (sans `en.toml`, le texte du code s'affiche).
Exemple de `fr.toml` :
```toml
name = "Français"

[text]
"Not in a voice channel" = "Pas dans un salon vocal"
"{n} members" = "{n} membres"        # mêmes {paramètres}
```
- **Dans le programme** (Python) : `from hsm01.i18n import tr` puis `tr('Not in a voice channel')`,
  `tr('{n} members', n=3)` ; `point('2.4 km')` écrit les décimales à la façon de la langue
  (« 2,4 km » en français). La langue vient de `HSM01_LANG` ; la bibliothèque a son propre fichier
  (`hsm01/i18n/en.toml`, `fr.toml`) pour ses textes (tableau de bord, UDP…), celui du module passe
  par-dessus.
  Autre langage : lire `HSM01_LANG` et le même fichier TOML.
- **Dans `module.toml`**, l'app traduit avec le même fichier : `name`, `description`, le `guide`
  ligne par ligne, et pour chaque champ du formulaire `label`, `help`, `unit`, `add`,
  `choice_labels` ; pour chaque `[[secret]]`, `label` et `description`.
- **Choix d'une liste** : la valeur enregistrée reste celle de `choices` (des identifiants, qui ne
  changent pas avec la langue) ; `choice_labels` (même ordre) donne le texte affiché, traduit.
- Changer de langue dans l'app **relance les modules** (nouvelle `HSM01_LANG`) et relit leurs
  `module.toml`. Le modèle « Créer un module » est livré avec son `i18n/en.toml` et `fr.toml`.
- Vérifier les fichiers des modules livrés, de la bibliothèque et du modèle :
  `python3 -I tools/i18n_check.py fr` (ou `en` ; textes de `tr(…)` et de `module.toml` sans
  traduction, paramètres différents, entrées en trop). Après avoir ajouté ou changé des textes :
  `python3 -I tools/i18n_check.py en --fill` réécrit les `en.toml` (tous les textes, les valeurs
  déjà là gardées, les entrées plus utilisées retirées), puis compléter les `fr.toml`.

## Ajouter des commandes à l'app
Un module lancé peut proposer ses **commandes** : elles apparaissent alors dans l'onglet Commandes
(sur toutes les pages) et disparaissent quand on le coupe. Bouton (ou clic de molette, power, case de
deck) : action « commande de module » ; molette : cible « commande de module » (les crans vont au
module, pas au son) ; LED : « état d'une commande de module ». Une liaison faite reste dans la
config (`{ type = "module_command", module = "Mon module", command = "zero" }`, cible
`{ type = "module", module = …, command = … }`, LED pareil) ; module arrêté, elle est marquée
« (arrêté) » et ne fait rien. En Python :
```python
@client.command('zero', 'Remettre à zéro')            # bouton
def zero(ev):
    set_count(0)

@client.command('compteur', 'Compteur', kind='knob')  # molette : ev['delta'] = crans
def knob(ev):
    set_count(count + ev['delta'])

client.command_state('compteur', text='Compteur 3', value=0.03)  # widgets de la molette
client.command_state('zero', active=True)                         # LED
```
Une liste qui change (scènes d'OBS, listes de lecture…) : `client.commands([(id, nom, fonction),
(id, nom, fonction, 'knob')…], replace=True)` envoie tout d'un coup et retire les commandes qui n'y
sont plus (leurs liaisons restent, marquées « (arrêté) » jusqu'à leur retour).
Autres langages : requêtes `commands` / `command_state`, événement `command` ([api.md](api.md)).
Le modèle (« Créer un module ») en contient deux.

## Données sensibles (`.env`)
Jetons, mots de passe, clés d'API : **jamais** dans `config.toml`, le code du module ni son
journal, mais dans le `.env` de son dossier. Le module les déclare dans `module.toml` :
```toml
[[secret]]
name = "DISCORD_CLIENT_ID"
label = "ID de l'application"
description = "Portail développeur Discord → votre application"

[[secret]]
name = "DISCORD_CLIENT_SECRET"
label = "Secret"
hidden = true
```
Module activé, son onglet (à son nom, après « Modules ») a une section **Données sensibles** : un
champ par `[[secret]]` (les `hidden` en points, « afficher » pour relire) ; **Enregistrer** écrit le
`.env` (droits 0600 : vous seul) et relance le module. Le programme les reçoit comme variables
d'environnement (`os.environ['DISCORD_CLIENT_ID']`).

Format du `.env` : une ligne `NOM=valeur` (ou `export NOM=valeur`) ; valeur nue, entre `'…'`, ou
entre `"…"` avec `\"`, `\\`, `\n` ; lignes `#` et lignes invalides ignorées. Une écriture
(fenêtre ou `secrets.save`) garde les autres variables du fichier, pas ses commentaires. Une valeur obtenue en marchant (jeton OAuth
renouvelé…) se garde pour les lancements suivants **sans relancer** le module :
```python
from hsm01 import secrets
token = secrets.get('DISCORD_REFRESH_TOKEN')   # .env relu (ou variable d'environnement)
secrets.save('DISCORD_REFRESH_TOKEN', new)     # écrit le .env (0600) ; '' efface la ligne
```
Le `.env` n'est jamais copié (modules livrés, « Réinstaller ») : **ne pas le joindre** en partageant
un module.

## Écrire le programme en Python
```python
from hsm01 import Client, widgets as W

PAGE = 'main'
c = Client('minuteur')            # nom remplacé par $HSM01_MODULE quand l'app le lance
c.page(PAGE, [W.text('t', 0, 100, 480, 60, '25:00', size=48, bold=True)], name='Minuteur')

@c.on('turn')
def turn(ev):
    if ev['mine'] and ev['page'] == PAGE:   # nos entrées seulement (voir api.md)
        c.update(PAGE, {'t': {'text': f'{ev["delta"]}'}})

c.run(tick=lambda dt: None, fps=2)  # tick : animations, horloges
```
À savoir :
- **Toutes** les entrées arrivent à tous les programmes ; `mine` dit si elle est pour vous (votre
  page est affichée, ou vous l'avez prise avec `grab`), `page` dit quelle page est affichée.
- Mettre à jour seulement ce qui change (`update` avec quelques champs) : l'app n'envoie à
  l'appareil que les zones modifiées, et transforme les déplacements en défilements matériels.
- Widgets : `text`, `rect`, `icon` (icône du thème par nom, ou fichier), `image` (`radius` :
  avatar rond), `separator`,
  cadres `frame` (`app/clients/python/hsm01/widgets.py`) ; menus prêts : `hsm01.menu.GridMenu`.
- Alertes : `c.alert(titre, texte, icon=…, led=1, duration=6)` (une seule par application : la
  plus récente remplace les autres) ; bandeau court « pilule », en bas, animé :
  `c.pill('ACTION ACTIVÉE : EN DIRECT', border='#3ddc84', dot='#ffffff', color='#ffffff')`
  (contour, points et texte chacun sa couleur ; 3 s) ; lecteur média :
  `c.call({'type': 'media'})`, `c.post({'type': 'media', 'action': 'next'})` ; entrées sur toutes
  les pages : `c.grab('knob4')`.
- Exemples complets : les modules livrés, dans leurs dossiers (`media`, `system`, `deck` ;
  `discord` : connexion à un autre programme dans un fil à part, `[[secret]]`, jeton gardé par
  `hsm01.secrets`, page mise à jour par différences).

## Dans un autre langage
Se connecter au socket Unix `$HSM01_API_SOCKET`, échanger des objets JSON (un par ligne), dire
bonjour avec le nom du module, puis réagir aux événements. Node.js :
```js
const net = require('net');
const s = net.createConnection(process.env.HSM01_API_SOCKET);
const send = (m) => s.write(JSON.stringify(m) + '\n');
send({type: 'hello', name: process.env.HSM01_MODULE || 'mon-module'});
send({type: 'page', page: 'main', name: 'Node', widgets: [
  {id: 't', x: 0, y: 100, w: 480, h: 40, kind: 'text', text: 'Bonjour', size: 28}]});
let buf = '';
s.on('data', (d) => {
  buf += d;
  for (let i; (i = buf.indexOf('\n')) >= 0; buf = buf.slice(i + 1)) {
    const ev = JSON.parse(buf.slice(0, i));
    if (ev.event === 'click' && ev.mine) send({type: 'alert', title: 'Clic', text: ev.button});
  }
});
s.on('close', () => process.exit(0));
```
`module.toml` : `command = ["node", "index.js"]`. Une page web passe par le WebSocket ([api.md](api.md)).

## Essayer sans l'onglet Modules
Pendant que l'app tourne : `PYTHONPATH=app/clients/python python3 main.py` (dans le dossier du
module). `app/target/release/hsm01ctl events` montre les événements qui passent.

## Partager un module
Compresser son dossier, **sans son `.env`** ; l'autre personne le décompresse dans `~/.local/share/hsm01/modules/`, puis
« Rafraîchir les modules » et allume son interrupteur. Un module tourne **avec vos droits** (comme
n'importe quel programme) : n'installer que des modules de confiance, après avoir lu leur code.
