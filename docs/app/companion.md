# Application compagnon `hsm01-app` (firmware custom 240.xx)

Sources : `app/` (workspace Rust séparé des drivers), dépend de `driver/rust/hsm01-proto` et
`hsm01-usb`. Firmware requis : custom (`firmware/rust`, version 240.xx) – le firmware officiel est
refusé avec un message (support prévu plus tard).

```sh
cd app && cargo build --release
./target/release/hsm01-app            # fenêtre + appareil, audio PipeWire (repli : simulation)
./target/release/hsm01-app --headless # sans fenêtre (service), --page N : page de départ
./target/release/hsm01-app --minimized # dans la barre système, sans fenêtre (démarrage de session)
./target/release/hsm01-app --sim      # audio simulé (sans PipeWire)
./target/release/hsm01-app --print-default-config
./target/release/hsm01-app --snapshot page.png --page 2 [--sim]   # rendu d'une page en PNG, sans appareil
./target/release/hsm01-app --keys ctrl+alt+t                      # essai d'un raccourci (clavier virtuel)
cargo test                            # tests hors appareil
cargo test -- --ignored --nocapture pulse_probe   # sonde PipeWire réelle (inventaire + crêtes)
```

## Architecture
| Module | Rôle |
|--------|------|
| `gui.rs` | fenêtre egui/eframe 0.37 (rendu glow) : aperçu en direct à gauche (sélection / déplacement des widgets à la souris, grille 16 px) ; à droite trois onglets : **Widgets** (page, liste, « + Ajouter un widget », inspecteur en sections Position et taille / Réglages), **Commandes** (une carte par molette, bouton, power : tourner / clic / maintien / LED), **Appareil** (langue, luminosité, sensibilité des molettes, veille) ; Enregistrer (Ctrl+S) / Annuler |
| `i18n.rs` | traductions : textes anglais du code, `i18n/fr.toml` intégré, fichiers de `~/.config/hsm01/i18n/`, `tr` / `tf!` / `tx` ([Langue](#langue)) |
| `config.rs` | modèle TOML `~/.config/hsm01/config.toml` (créé au 1er lancement) : pages, widgets libres, molettes, boutons |
| `engine.rs` | thread moteur : connexion (reconnexion auto), entrées (décodeur molettes 512 pas/tour, fronts boutons), actions, rendu 60 fps, LEDs, aperçu partagé pour l'interface |
| `render.rs` | rendu 480×272 tiny-skia + fontdue (polices Noto/DejaVu système) : texte à variables, rectangles arrondis, VU-mètres (zones vert/jaune/rouge + crête), jauges de volume, images PNG |
| `screen.rs` | n'envoie que les **tuiles 16×16 modifiées** (fusionnées par ligne, BLIT ou BLIT_RLE selon le plus petit) ; firmware 240.21+ : contenus déplacés (défilements) envoyés en COPY + ce qui apparaît |
| `api.rs` | **API ouverte** ([api.md](api.md)) : socket Unix + WebSocket local à jeton, JSON ; alertes, pages de widgets des programmes (après celles de la config), molettes / boutons envoyés en événements |
| `notify.rs` | notifications du bureau (D-Bus, mode moniteur) et alertes de l'API, file d'attente |
| `tools.rs` | **modules** : sous-dossiers de `~/.local/share/hsm01/modules/` (`module.toml` + programme, [modules.md](modules.md)) lus à la volée, modules livrés (`app/modules/`) copiés au premier lancement ; lancement, relance (arrêt ou code modifié), journal, modèle d'un nouveau module |
| `media.rs` | lecteurs MPRIS de la session : lecteur suivi, événement et requête `media` de l'API |
| `bin/hsm01ctl.rs` | commande shell de l'API (`hsm01ctl alert … --wait`, `send`) |
| `audio.rs` | interface `Audio` + `SimAudio` |
| `pulse.rs` | PipeWire via l'API client PulseAudio (comme pavucontrol) : inventaire par abonnement, volume/mute, flux de crête `PEAK_DETECT` 60 Hz par cible affichée (appli : `set_monitor_stream`), flux PCM 48 kHz pour le spectre |
| `visual.rs` | analyseur de spectre : FFT 2048 points (fenêtre de Hann), bandes log 35 Hz–16 kHz, pente +3 dB/octave, gain automatique (50 dB sous la bande la plus forte récente), retombée des barres et chapeaux de crête |
| `keys.rs` | raccourcis clavier : clavier virtuel uinput (thread dédié), combinaisons `ctrl+alt+t` |

## Langue
Textes de l'app (fenêtre, icône de la barre système, messages du terminal, textes dessinés sur
l'appareil) écrits **en anglais dans le code** ; `app/hsm01-app/i18n/fr.toml` les donne en français
(`"texte anglais" = "traduction"`, mêmes `{paramètres}`, inclus dans le binaire). Réglage
`language` (`en` par défaut, absent du fichier dans ce cas ; `fr`…), onglet **Appareil →
Language / Langue** : appliqué tout de suite (`i18n::set_language`, aussi au rechargement de la
config). Autre langue : copier `fr.toml` en `~/.config/hsm01/i18n/<code>.toml` (`name` = nom de la
langue, puis traduire) ; un fichier de ce dossier du même code qu'un fichier livré en complète ou
remplace les entrées. Un texte sans traduction reste en anglais. Code : `tr("…")`, `tf!("… {n}",
n = …)` (valeurs déjà formatées pour une précision), `tx("…")` dans un `const` (traduit à
l'affichage). Le test `i18n::tests::french_is_complete` vérifie que chaque texte du code a sa
traduction (mêmes paramètres) et qu'aucune entrée n'est en trop. Les modules suivent la même
langue avec leurs propres fichiers, `en.toml` compris ([modules.md](modules.md#langue-des-modules)).

## Configuration
```toml
language = "fr"            # langue (en par défaut)
brightness = 80            # rétroéclairage (%)
counts_per_percent = 8     # pas de molette par % de volume (512 pas/tour → 64 %/tour)

[[page]]
name = "Audio"
background = "#000000"

[[page.knobs]]             # molettes 1..4 : cible tournée, action au clic
target = { type = "default_sink" }        # default_sink | default_source | sink | source | app (name = "…")
click = { type = "mute", knob = 1 }

[[page.buttons]]           # boutons 1..4 : action + LED
action = { type = "command", cmd = "notify-send salut" }   # mute | command | url | keys | next_page | prev_page | page
led = { type = "muted", knob = 1 }                          # off | on | muted | unmuted

[[page.widget]]            # widgets placés librement (x, y, w, h en pixels)
kind = "text"              # text | rect | vu | volume | image | spectrum
x = 4
y = 30
w = 112
h = 22
text = "{name}"            # {name} {volume} {mute} (molette `knob`) ; {page}
knob = 1
size = 16.0

[[page.widget]]            # visualiseur (page « Visualiseur » de la config par défaut)
kind = "spectrum"
x = 0
y = 32
w = 480
h = 240
target = { type = "default_sink" }       # même syntaxe que les molettes
bars = 48
gap = 2
colors = ["#006c68", "#008c88", "#00b4a8", "#9ad8d0"]   # zones de bas en haut
peak = "#ffffff"           # chapeaux de crête (absent = sans)
background = "#000000"
```
Le spectre est envoyé en commandes FILL (seules les colonnes modifiées, quelques Kio par image) et
ses tuiles sont exclues du diff : garder x/y/w/h multiples de 16.

Raccourcis : `action = { type = "keys", keys = "ctrl+alt+t" }`. Noms : modificateurs `ctrl` `shift`
`alt` `altgr` `super`, lettres, chiffres, `f1`…`f24`, `enter` `esc` `tab` `space`…, multimédia
`play_pause` `next` `prev` `stop` `volume_up` `volume_down` `mute` `mic_mute`, ou tout code evdev
(`KEY_…`). Ce sont des **positions physiques QWERTY** : sur AZERTY, `a` est la touche marquée Q.
Accès : `/dev/uinput` accessible à l'utilisateur (`getfacl /dev/uinput`), sinon règle
`packaging/udev/71-hsm01-app-uinput.rules`. Le clavier virtuel est créé au premier raccourci ; ce
premier appui attend qu'udev ait traité le périphérique (~1,5 s sous KDE Wayland), les suivants sont
immédiats.

Interface : « Cadres des widgets » et la sélection (cadre jaune) sont dessinés **sur l'écran du
Stream 100** tant que la fenêtre est ouverte (et donc aussi dans l'aperçu) ; sur une zone de spectre
ils sont renvoyés en FILL à chaque image. Les cases « Cadres des widgets », « Aimants », « Grille
16 px » et « Lier rouge, vert et bleu » sont gardées d'un lancement à l'autre
(`~/.local/state/hsm01/window.toml`, écrit à chaque changement, hors config). Les modifications s'appliquent **en direct** à l'appareil ; « Enregistrer » met à jour le
fichier en place (`toml_edit` : commentaires et mise en forme conservés pour ce qui n'a pas changé ;
l'ancien fichier est aussi gardé en `config.toml.bak`). « Annuler » recharge le fichier. Le menu
« liste » à côté d'un nom de cible propose les appareils présents et les applications qui jouent du
son. Icône dans la barre système (`tray.rs`, StatusNotifierItem) : ouvrir la fenêtre, choisir la
page, quitter ; fermer la fenêtre laisse l'app tourner (sans barre système, fermer quitte). Fermer
avec des modifications non enregistrées demande quoi faire. **Une seule instance** (`single.rs`,
socket `$XDG_RUNTIME_DIR/hsm01-app.sock`) : relancer l'app (menu des applications) envoie
« show » à celle qui tourne, qui ouvre sa fenêtre ; `--minimized` démarre sans fenêtre (l'icône
ou un nouveau lancement l'ouvre ; sans barre système, l'app tourne cachée jusqu'au prochain
lancement) et ne fait rien si l'app tourne déjà.

**Installer** : `packaging/install.sh` (depuis les sources : compile l'app ; ou `./install.sh` de
l'archive d'une release) installe pour l'utilisateur dans `~/.local/share/new-hera-nux/`
(programme, modules livrés et bibliothèque Python à côté, `VERSION`), liens dans `~/.local/bin`,
entrée du menu et icône, démarrage réduit à l'ouverture de session (`~/.config/autostart/`,
`--minimized`, après le panneau), règle udev (sudo), puis démarre l'app. Relancé, il met à jour :
programme remplacé, app relancée, vos copies des modules livrés mises à jour (sauf leur fichier
de réglages s'il existe, et `.env`) ; options `--keep-modules`, `--no-autostart`, `--no-udev`,
`--no-start`, `--uninstall` (réglages et modules gardés).

Veille (`[standby]`, réglable dans « Appareil ») : après `after_secs` (300 par défaut, 10 minimum)
sans action (molette, clic, modification dans la fenêtre, changement de page), le rétroéclairage
descend à 0 en `fade_secs` (6 s ; courbe en puissance 2,2 pour une baisse régulière à l'œil), puis plus rien n'est dessiné ni envoyé et les LEDs s'éteignent ; la lecture des
entrées continue. Une action réveille l'écran ; elle n'est pas exécutée (molettes et clics ignorés
0,5 s). Une alerte (notification du bureau, alerte d'un module) le réveille aussi, selon le réglage
Notifications → réveil ou son propre `wake`.
**Pas de veille quand un module est affiché** : case « Veille : pas de veille quand il est affiché »
de sa fiche (onglet Modules ; `keep_awake = true` dans son `[[module]]`). Tant que sa page est
affichée, ou que sa page ou un de ses widgets est dans la page affichée, c'est comme une action à
chaque image : l'écran reste allumé ; sur une autre page, le délai de veille repart de zéro
(`engine::kept_awake`, avec `shown_tool` et les widgets `module` de la page).

Arrêt de l'app : l'appareil passe en veille (rétroéclairage à 0, LEDs éteintes ; plus rien ne
passe sur l'USB) jusqu'au prochain lancement, qui le rallume. Ça vaut pour « Quitter » (icône de
la barre système), la fermeture de la fenêtre sans icône, SIGTERM (fin de session, extinction du
PC, `kill`), SIGINT (Ctrl+C) et SIGHUP, et même un plantage du moteur (`Drop` de `Engine`).
Si la fenêtre ne répond pas (réduite), l'app sort quand même 3 s après (`quit_on_signals`,
`main.rs`). App tuée (`kill -9`), plantée, ou PC en veille : c'est le firmware 240.22 qui s'en
occupe ([stage2.md](../firmware/stage2.md), « Sans hôte ») : 3 s sans que l'app lise ses reports
→ badge « SIGNAL NOT FOUND » qui se promène à 1 % de rétroéclairage, écran éteint au bout de
5 min ; bus USB suspendu → rétroéclairage et LEDs éteints jusqu'au réveil. Si l'app, figée plus
de 3 s, repart : l'époque de l'écran (octet 21 du report) a changé, elle redessine tout
(`screen_epoch` / `redraw`, `engine.rs`).

Volume au-dessus de 100 % : à 100 %, tourner encore vers le haut n'augmente plus le volume mais
affiche sur l'appareil « Volume au-dessus de 100 % ? » (risque pour les haut-parleurs ou l'audition),
« Oui » / « Non » (« Non » par défaut). La même molette choisit (gauche « Oui », droite « Non ») et
son clic valide ; un autre bouton ou 15 s sans action répondent « Non ». Après « Oui », la cible
peut monter jusqu'à 150 % ; revenue sous 100 %, la question sera reposée. Pendant la question, le
spectre est figé (redessiné en entier ensuite).

Image de fond (`[page.wallpaper]`, onglet Widgets → Image de fond, bouton « Parcourir… » =
sélecteur de fichiers du bureau via `rfd`) : `path` (PNG, JPEG, WebP, GIF, animés ou non), `fit`
(`cover` remplir / `contain` contenir / `stretch` étirer / `center` taille réelle), `dim`
(assombrissement %, lisibilité des widgets), `fps` (cadence max des animations, 12). Chaque image
est décodée, cadrée et assombrie une fois (`wallpaper.rs`, 150 images max, au-delà une sur k).
Envoi : `Screen::update` envoie au plus `SEND_BUDGET` (12 Kio) par image de la boucle, le reste
attend l'image suivante ; l'horloge des animations n'avance que quand l'écran est à jour (vitesse
réelle, images sautées si l'USB ne suit pas : un fond plein écran qui change entièrement plafonne
vers 3-4 images/s). Mesure (GIF de test, `HSM01_STATS=1`) : 60 fps, ~48 Kio/s, 100 rapports/s.

Édition dans l'aperçu (`gui.rs` + `layout.rs`) : clic = sélectionner, Maj+clic = ajouter / retirer,
glisser dans le vide = cadre de sélection (Maj : ajoute), glisser une sélection = la déplacer en bloc
(aimants / grille sur son cadre englobant), 8 poignées sur un widget seul = redimensionner (bords
aimantés, 4 px minimum, curseur adapté). Plusieurs widgets sélectionnés (onglet Widgets) : aligner
gauche / centre / droite / haut / milieu / bas (sur le cadre englobant), répartir (3 au moins),
même largeur / hauteur que le dernier choisi, dupliquer, supprimer. Clavier (hors champ de texte,
onglet Widgets) : flèches 1 px (Maj 10 px), Suppr / Retour arrière, Ctrl+D, Ctrl+A, Échap. Les cadres
jaunes de toute la sélection sont aussi dessinés sur l'appareil.

Défaire / refaire (fenêtre) : Ctrl+Z, Ctrl+Maj+Z ou Ctrl+Y, boutons « Défaire » / « Refaire » de la
barre d'état ; 100 pas. Un geste continu (glisser un widget, tirer un curseur, une saisie suivie
d'une pause de 0,4 s, souris relâchée) compte pour un pas. Dans un champ de texte, Ctrl+Z reste
celui du texte. « Recharger » (ex-« Annuler ») revient au fichier ; Ctrl+Z annule ce retour.

Aimants (aperçu, case « Aimants ») : en glissant un widget, son bord gauche / centre / bord droit
(et haut / milieu / bas) s'aligne à 6 px près sur ceux des autres widgets et de l'écran, avec une
ligne de repère magenta ; Alt maintenu = déplacement libre ; « Grille 16 px » prioritaire.

Nom de l'application : « new hera-nux » (`APP_NAME` : titre de la fenêtre, barre système).

Cible « application en plein écran » (`target = { type = "fullscreen" }`, pour les jeux) :
au démarrage, l'app charge dans KWin un script (`fullscreen.rs`, D-Bus `org.kde.KWin /Scripting
loadScript`, fichier `$XDG_RUNTIME_DIR/hsm01-fullscreen.js`, retiré à la fermeture) qui signale à
son service D-Bus `org.hsm01.App` (`Fullscreen(pid, classe, fichier desktop, titre)`) la fenêtre en
plein écran (`fullScreen`, ou sans bordure couvrant tout son écran) ; la dernière est gardée tant
qu'elle reste en plein écran, même sans le focus. Flux audio associés : `application.process.id`
égal au PID de la fenêtre ou descendant de lui (`/proc/<pid>/stat`), sinon nom / binaire contenant
la classe. Nom affiché : titre de la fenêtre ; icône : `Icon=` de son fichier `.desktop` (ou celui
dont `StartupWMClass` est la classe), `steam_icon_N` pour `steam_app_N` (jeux Steam / Proton), sinon
la classe ; jeux Steam / Proton (`steam_app_N`) : `steam_icon_N` du thème, sinon l'icône du client
(`steam/games/<hash>.ico` jusqu'à 256 px, hash lu dans `appcache/appinfo.vdf`), sinon l'icône 32×32
ou l'en-tête de `appcache/librarycache/N/` (`icons::steam_game_icon` ; test
`steam_icons`). La barre d'état indique « plein écran : … ». KDE Plasma uniquement.

Icônes des sources (widget `icon`, `knob = N`) : icône de la cible de la molette, d'après
`application.icon_name` (flux ; à défaut le binaire, sinon le nom de l'application — Spotify
n'annonce ni icône ni binaire) ou `device.icon_name` (appareils) de PipeWire, cherchée dans le thème
d'icônes du bureau (`kdeglobals` `[Icons] Theme`, Breeze par défaut, variante `-dark` d'abord, puis
thèmes hérités, hicolor, Adwaita, `/usr/share/pixmaps` ; nom raccourci si besoin :
`audio-card-analog-pci` → `audio-card` ; `icons.rs`). Style `auto` (défaut) : appareils en
icône symbolique teintée (`tint`, gris clair ; les illustrations Breeze sont sombres sur l'écran
noir), applications en couleur ; `color` / `symbolic` pour tout. Grisée (30 %) quand la cible est
muette. Aperçu : `TMPDIR=… cargo test --release -- --ignored gradient_snapshot`
(`hsm01-icons-sources.png`).

Étalonnage (`[calibration]`, onglet Appareil → « Étalonnage des couleurs » → Régler) : par canal
rouge / vert / bleu, `gain` (balance des blancs, 0,3–1,5) et `gamma` (tons moyens, > 1 éclaircit),
`sortie = gain × entrée^(1/gamma)`, appliqués à tout ce qui part vers l'écran (tuiles, remplissages
du spectre, cadres ; `render::Lut`). Section ouverte : le Stream 100 affiche une mire corrigée
(rampes gris / R / V / B en 16 pas, nuancier, quasi-noirs et quasi-blancs) et l'aperçu la même mire
non corrigée, pour comparer avec l'écran du PC. « Lier rouge, vert et bleu » déplace les trois
canaux ensemble. Le logo de démarrage (firmware) n'est pas corrigé.

Cadres décoratifs (`frame` de n'importe quel widget, onglet Widgets → case « Cadre ») :
`frame = { border = "#00b4a8", width = 2.0, fill = "#101818", radius = 8.0, padding = 4 }` —
bordure (couleur ou dégradé, `width` 0 = sans), fond facultatif (couleur ou dégradé), coins
arrondis, marge : le contenu se dessine en retrait (`Widget::inner`, marge + épaisseur) et la
bordure par-dessus. Spectre encadré : zone envoyée en FILL = l'intérieur, le cadre part avec les
tuiles. Aperçu : `gauge_snapshot` (`hsm01-frames.png`).
Bords affichés : `sides = { top = true, right = false, bottom = true, left = false }` (tous par
défaut, non écrit). Un coin est arrondi entre deux bords affichés (ou deux masqués) ; là où la
bordure s'arrête, le bord va jusqu'au bout et le fond y est à angle droit. Le retrait du contenu ne
compte l'épaisseur que des bords affichés. Fenêtre : petit schéma du cadre, un clic sur un bord
l'affiche ou le masque (au centre : tous / aucun), + préréglages (tous, haut et bas, gauche et
droite, souligné).

Image (`kind = "image"`, `path` d'un PNG, mis à l'échelle du cadre) : `radius` arrondit ses coins
(onglet Widgets → arrondi) ; la moitié du cadre ou plus en fait un disque (avatars du module
Discord). Code : `draw_image` (`render.rs`, l'image comme motif d'un rectangle arrondi ou d'un
ovale).

Icône par nom (`kind = "theme_icon"`, `name = "audio-input-microphone"` ou chemin d'un fichier
SVG / PNG, `tint` facultatif) : une icône fixe, sans lien avec une molette. Recherche dans le
thème : apps, devices, status, places, categories, mimetypes puis **actions** (précédent, lecture,
suivant, go-previous…) et emblems. Les icônes monochromes (style `ColorScheme-Text` de Breeze,
#232629 ; gris en dur des icônes « -symbolic » GNOME, #2e3436…) sont dessinées dans `tint`, ou
en clair (#e8eeee) sans teinte : sinon elles disparaissent sur les fonds sombres de l'écran.

Séparateur (`kind = "separator"`) : ligne centrée dans son cadre, `thickness` (px), `vertical`,
`style = "solid" | "dashed" | "dotted"` (tirets 3× et points 1× l'épaisseur, écartés pour que les
deux bouts soient pleins), `fade = true` (extrémités en fondu sur un quart chacune) ; `color` uni ou
dégradé, le long de la ligne. Aperçu : `TMPDIR=… cargo test --release -- --ignored
lines_snapshot` (`hsm01-lines.png`, `hsm01-notice.png`).

Module (`kind = "module"`, `module = "Menu de raccourcis"`, `page` facultatif, `inputs`, `overflow`) :
la page d'un module lancé (onglet Modules), ou **un widget qu'il propose** (API `page` avec
`widget: true` et sa taille : « Membres du salon vocal » du module Discord…), dessiné dans le cadre
et à jour en direct ; rien tant que le module ne tourne pas. `page` : identifiant de la page ou du
widget (vide : sa première page, « sa page « … » » dans l'éditeur ; liste « montrer » des pages et
widgets du module lancé ; choisir un widget décoche les entrées). `overflow` (« trop grand ») : `shrink` (défaut,
réduit en entier, proportions gardées, centré ; tel quel s'il tient) ou **`scroll`** (largeur
ajustée au cadre, 1:1 si assez étroit ; ce qui dépasse en hauteur défile seul : 2,5 s en haut,
20 px/s vers le bas, 2,5 s en bas, retour ; `Renderer::module_view`, `scroll_offset`) ou
**`scroll_sideways`** (« défiler ↔ », pour une liste en ligne : hauteur ajustée, ce qui dépasse en
largeur défile de gauche à droite et retour, mêmes temps ; collé à gauche, centré en hauteur). `inputs` (défaut
`["knob4", "button1", "button2", "button3", "button4"]`, cases à cocher molettes / boutons 1-4) :
quand cette page est affichée, ces entrées vont au module comme sur sa propre page (`mine` vrai,
`page` = l'identifiant de sa page) au lieu des réglages de la page (Commandes) ; les autres restent
à la page. Les pages de module ne contiennent pas d'autre widget `module`. Code : `embedded` / `routes`
(`engine.rs`), `WidgetKind::Tool` (`render.rs`). Aperçu avec le vrai menu : `TMPDIR=… cargo test
-- --ignored shipped_modules_run` (`hsm01-deck-widget.png`).

Stream deck (`kind = "deck"`, `knob`, `vertical`, `color`, cases `[[page.widget.tile]]` : `title`,
`subtitle`, `icon`, `action`) : une piste de cases sans fin, la sélectionnée au milieu (couleur de
la piste, plus claire), les voisines avant et après (case `i mod n`). Sur sa page, la molette
`knob` lui appartient : tourner déplace la sélection d'une case tous les 40 comptes (~13 par
tour, `DECK_STEP`) avec un glissement de 160 ms, cliquer lance l'action de la case (éclair de
250 ms) ; ses réglages de l'onglet Commandes ne jouent plus sur cette page, les autres molettes
les gardent. Un seul deck par molette (le premier). Colonne (défaut, 112×248 juste au-dessus de
sa molette) ou ligne (480×72) ; changer le sens remet la forme. Éditeur (onglet Widgets) :
molette, sens, couleur, cases repliables (titre, sous-titre, icône, action, ↑ ↓ supprimer),
« + case », « + média » (4 cases). Ajouté : la 1re molette sans deck, colonne au-dessus d'elle.
Marche aussi sur une page de module (il passe avant le module ; une molette prise par `grab` reste au
programme). Code : `deck_inputs` / `deck_views` / `DeckState` (`engine.rs`), `Renderer::deck`
(`render.rs`). Aperçu : `TMPDIR=… cargo test -- --ignored deck_snapshot` (`hsm01-deck.png`,
`hsm01-deck-row.png`). Action `media` (`play_pause`, `next`, `previous`, `stop`) : le lecteur qui
joue, partout (boutons compris), appel lancé hors du moteur.

Styles de jauge (`gauge` des widgets `vu` et `volume`, onglet Widgets → style ; `gauge.rs`) :
`bar` (défaut, non écrit), `{ style = "segments", count = 16, gap = 2 }` (blocs LED colorés selon
leur position, crête = segment blanc), `{ style = "arc", start = -135, end = 135, thickness = 10 }`
(arc, dégradé le long de l'arc), `{ style = "needle", start, end, thickness = 4 }` (cadran : échelle
colorée, graduations tous les 10 %, aiguille, crête à l'extérieur), `{ style = "image", track =
"piste.png", fill = "remplissage.png", sweep = false, start, end }` (forme libre : piste entière,
remplissage révélé le long du widget ou, `sweep = true`, par angle autour du centre). Angles en
degrés, 0 = haut, sens horaire. Passer à `arc` / `needle` rend le cadre carré. Aperçu :
`TMPDIR=… cargo test --release -- --ignored gauge_snapshot`.

Couleurs et dégradés (`color` des textes, rectangles, jauges `volume` et VU-mètres `vu`) :
`"#rrggbb"` (uni), `["#…", "#…", …]` (dégradé régulier, début → fin) ou
`{ colors = [...], stops = [0.0, 0.7, …], hard = true, vertical = true }` (positions 0..1 ;
`hard` = zones nettes ; `vertical` = de haut en bas pour textes et rectangles). Jauges et VU-mètres :
le dégradé suit l'axe, du vide vers le plein (la couleur d'un niveau ne dépend pas du remplissage) ;
VU par défaut : vert < 70 % < jaune < 90 % < rouge. Texte : le dégradé couvre le cadre (zone
visible) et reste fixe quand le texte défile. Aperçu : `TMPDIR=… cargo test --release -- --ignored
gradient_snapshot`.

Notifications du bureau (`[notifications]`, onglet Appareil ; `notify.rs`) : l'app observe le bus
D-Bus de session en mode moniteur (`BecomeMonitor` sur `org.freedesktop.DBus.Monitoring`, règle
`interface='org.freedesktop.Notifications', member='Notify'`, comme `dbus-monitor`) : elle voit les
notifications envoyées au serveur du bureau, qui les affiche toujours ; rien n'est envoyé sur cette
connexion. Filtre : `apps = ["discord", "vesktop"]` (partie du nom d'application ou de
`desktop-entry`, sans casse ; vide = toutes). Image : indice `image-data`, puis `image-path`, puis
`app_icon` fichier (avatar, recadré en disque 56 px), sinon icône nommée (thème), sinon initiale.
Texte : balises et entités retirées, 300 caractères au plus, titre 1 ligne + message 2 lignes
(« … »). Affichage : fenêtre en haut de l'écran (`NOTICE_BOX`), barre de temps restant,
`duration_secs = 6` (alertes des modules : style `card` ainsi, ou **`pill`** : bandeau d'une
ligne centré **en bas**, aussi large que son texte, contour `border` avec halo, un point `dot` à
chaque bout dont le halo respire (1,4 s), texte `color` en gras ; il monte du bas de l'écran en
0,25 s (ralenti à l'arrivée) et redescend avant la fin ; zone `PILL_BAND`, 3 s par défaut ;
« Tester une pilule » le montre ; code `Renderer::pill`, `NoticeStyle`) ; un clic (bouton,
molette, power) la ferme sans rien déclencher, tourner une molette agit normalement ; file de 4
au plus, **une seule par application** (`app`, sinon le programme qui l'envoie ; casse ignorée) :
une nouvelle remplace celle qui attend, et celle qui est affichée tout de suite (morceaux
enchaînés sur Spotify : seul le dernier ; `notify::push`, `next_notice`). `wake = true` : rallume l'écran en veille (sinon
la notification est ignorée). Les spectres continuent autour : la zone de la popup (arrondie aux
tuiles de 16 px) est retirée de leurs FILL et part en tuiles. Le contenu n'est jamais écrit dans
les journaux. Tests : `markup_is_removed_but_not_text`, `application_filter`, et sur le vrai bus
`cargo test --release -- --ignored live_notification` (envoie une notification avec `notify-send`).

Appui long (« maintien », `hold` des molettes et boutons, `power_hold` de la page) : maintenu
0,5 s → action de maintien ; s'il y en a une, le clic part au relâchement d'un appui court (sinon dès
l'appui, sans délai).

Textes trop longs pour leur cadre : défilement aller-retour (`scroll = true`, par défaut ; 30 px/s
par pas de 2 px, pauses de 1,5 s), sinon coupés avec « … ».
**Emoji** (partout : textes, alertes, pilules, noms Discord) : un pictogramme (U+1F000 et au-delà,
même si la police de texte en a un en noir et blanc, comme DejaVu Sans) ou un caractère absent de
la police de texte est pris en **couleur** dans Noto Color Emoji (l'image PNG de son glyphe, lue par
`ttf-parser`, à 1,15 × la taille du texte, posée sur la ligne de base ; une police d'emoji sans
images, COLR vectoriel, n'est pas retenue), sinon dans une police de secours
(DejaVu Sans, Noto Sans Symbols 2 / Symbols : flèches, coches, notes…) ; variantes (U+FE0F), liants
(ZWJ) et couleurs de peau ne sont pas composés : l'emoji de base s'affiche seul (familles et
drapeaux en morceaux). Code : `Renderer::piece` / `advance`, `emoji_png`. Aperçu : `TMPDIR=… cargo
test -- --ignored emoji_snapshot` (`hsm01-emoji.png`).

La configuration est **rechargée à chaud** (vérification chaque seconde) : modifier le fichier suffit.
Sauts de page (boutons, clics, maintiens, power) : `page` (`index = 2`), `module` (`name = "Lecteur
média"` : page d'un module de l'onglet Modules ; ancien nom `tool`), `back` (« retour » : vers la
page d'où vient le dernier saut, sinon, sur une page de module, la dernière page de la config
affichée, sinon page suivante ; c'est le power des pages de modules : **power quitte le module**).
`next_page` / `prev_page` et `page` ne passent que par les pages de la config : les pages de module
n'y sont plus (depuis une page de module, le cycle repart de la dernière page de config affichée).
Menu des modules (`module_menu`, « menu des modules » ; **par défaut maintien de la molette 1**) :
un bandeau monte du bas de l'écran en 200 ms (`MENU_BOX`, 480 × 92 en bas), un élément par module
qui a une page (nom du module, titre de sa page ; triés par nom), celui de la page affichée
présélectionné. Molette 1 : choisir (sans boucler) ; **clic de la molette 1 : valider** (montre
la page, power y fait « retour » vers la page d'où l'on a ouvert le menu) ; autre clic (power
compris) ou 10 s sans rien : fermer (descente 150 ms). Ouvert, il prend toutes les entrées (rien
d'autre ne réagit, programmes compris). Le maintien qui l'ouvre reste à l'app **sur une page de
module** aussi (celui de la dernière page de config affichée ; le module reçoit `press` / `hold`,
un appui court lui reste un clic), et sous un widget `module` ou `deck` qui prend la molette : le
deck lance alors sa case au relâchement d'un appui court. Sauf si un programme a pris la molette
par `grab`. Code : `ModuleMenu` / `menu_entries` / `menu_inputs` / `menu_tick`, `menu_keys` /
`keep_menu_keys` / `deck_fires` (`engine.rs`), `Renderer::module_menu` (`render.rs`). Aperçu : `TMPDIR=… cargo test
-- --ignored menu_snapshot` (`hsm01-menu.png`, `hsm01-menu-half.png`).
Actions de luminosité (boutons, clics, power) : `brightness_set` (`level = 50` : niveau fixe,
0 = éteint),
`brightness_cycle` (`levels = [100, 60, 30, 10]` :
niveau suivant après le plus proche de l'actuel, 0 permis), `brightness_up` / `brightness_down`
(`step = 10`, 5 % minimum). Le niveau choisi ainsi n'est pas enregistré : il vaut jusqu'à ce que la luminosité de
la configuration change (ou au redémarrage). Instance unique : relancer l'app alors qu'elle tourne
affiche sa fenêtre (socket `$XDG_RUNTIME_DIR/hsm01-app.sock`). Icônes (`icon.rs`, resvg, intégrées
à la compilation, monochromes `currentColor`) : `public/tray.svg` (dessin détaillé : fenêtre, en
vert) et `public/tray-symbolic.svg` (version simplifiée, traits épais : barre système 16-64 px, dans
la couleur de texte du thème KDE lue dans `kdeglobals`). Planche d'aperçu : `TMPDIR=… cargo test
--release -- --ignored icon_sheet`. VU-mètre / jauge : cocher ou décocher « vertical » fait pivoter le
cadre (largeur ↔ hauteur).
Bouton power : action par page (`[page.power]`, « page suivante » par défaut) ; sur une page de
module, « retour » (quitter le module). Cibles `sink`/`source`/`app` : correspondance partielle insensible à la
casse (nom, description, nom d'application ou binaire).

## Entrées et performances
Diagnostic : `HSM01_STATS=1 ./target/release/hsm01-app --page 2` affiche chaque seconde fps, Kio/s,
rapports d'entrée reçus (et trous de numéro de séquence), temps passé à envoyer, pire image.

Ratés de clics / molettes sur le visualiseur (corrigé 2026-11-20), deux causes mesurées :
| Cause | Mesure avant | Correctif | Après |
|-------|--------------|-----------|-------|
| lecture des rapports depuis la boucle 60 Hz, 1 essai de 1 ms par image (EP 0x81, bInterval 2 ms) | ~20 rapports/s sur ~100 | thread `hsm01-input` en lecture bloquante, la boucle vide la file | ~100/s |
| spectre envoyé en colonnes entières (~90 000 px/image) : le firmware remplit à ~4 Mpx/s et sa tâche d'entrées ne tourne plus | 40 fps, envoi 950 ms/s, **0 rapport/s** | `column_delta` : seules les lignes qui changent | 60 fps, envoi 80–150 ms/s, 100 rapports/s |

Config illisible après un enregistrement (corrigé 2026-11-20) : la fusion `toml_edit` recopiait les
nouvelles tables avec leur position dans le document généré ; un VU-mètre sans `color` dans le
fichier recevait `[page.widget.color]` sous le widget suivant (« duplicate key »). Les tables
copiées perdent maintenant leur position (rangées après leur parent), une valeur devenant table
reste en ligne, et le texte fusionné est relu avant écriture : s'il ne redonne pas la même config,
le fichier est réécrit entièrement. Tests : `saving_places_new_tables_correctly`,
`saving_keeps_comments_of_a_hand_written_file`.

Plantage (SIGSEGV) à la mise en veille (corrigé 2026-11-20) : fermer un flux d'enregistrement
libérait sa fonction de lecture côté Rust alors que libpulse pouvait encore l'appeler jusqu'à la
confirmation de la déconnexion par le serveur. La veille fermant tous les flux d'un coup pendant la
lecture, la course devenait probable. `close_stream` retire la fonction avant `disconnect`. Test de
stress : `cargo test --release -- --ignored --nocapture pulse_stream_churn` (150 ouvertures /
fermetures ; plantait avant, passe après).

Côté firmware (240.19, `docs/firmware/stage2.md`) : la réception USB cède la main après chaque
lecture et tous les 16 000 pixels dessinés ; même sous un flot continu de remplissages plein écran,
les rapports d'entrée continuent (25/s au pire, ~70-90/s en charge réaliste).

## État (2026-11-20)
| Étape | État |
|-------|------|
| 1 squelette, config, rendu, tuiles, entrées | ✔ sur l'appareil (240.17), ~5 % CPU |
| 2 audio PipeWire | ✔ sur l'appareil : sortie/entrée par défaut, applis (Vivaldi, Spotify) |
| 2b visualiseur (spectre) | ✔ sur l'appareil (page 2, bouton power) |
| 3 actions | ✔ commande, URL, mute, pages ; raccourcis uinput essayés (touches volume, Verr. Maj) |
| 4 interface egui (aperçu, éditeur) | ✔ première version sur l'appareil ; retours en cours |
| suite | garde-fou des entrées sous forte charge (app + firmware 240.18), étalonnage des couleurs |
