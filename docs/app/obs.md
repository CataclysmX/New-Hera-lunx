# Module « OBS » (2026-11-20)

Ce que propose le logiciel Hercules (traductions `herculescreator_fr.ts`, classes `Obs*` de
`Stream-control.exe`) : démarrer / arrêter le stream, l'enregistrement, changer de scène,
activer / désactiver un filtre, afficher / masquer une source (« groupes et sources »), lire un
fichier audio, changer de profil, de collection de scènes ; par le serveur WebSocket d'OBS
(obs-websocket v5, cf. [integrations.md](integrations.md)). Le module livré `app/modules/obs/` fait
tout cela, plus le mixeur audio d'OBS (volumes, sourdines, vumètres) sur les molettes, la pause
de l'enregistrement, le replay, la caméra virtuelle et le mode studio.

## Mise en route (une fois)
1. OBS 28 ou plus : Outils → Paramètres du serveur WebSocket → « Activer le serveur WebSocket »
   → Appliquer (port 4455, mot de passe généré par OBS).
2. Onglet Modules → « ⟳ Rafraîchir les modules » si besoin → allumer « OBS » → Enregistrer.

Rien d'autre sur le même ordinateur : le module lit le port et le mot de passe dans les réglages
d'OBS (`~/.config/obs-studio/plugin_config/obs-websocket/config.json`, ou celui du Flatpak
`~/.var/app/com.obsproject.Studio/…`), relus à chaque tentative. OBS sur un autre ordinateur :
`adresse` et `port` dans le formulaire, mot de passe dans les données sensibles (`OBS_PASSWORD`,
`.env` 0600). Tant qu'OBS ne répond pas, la page le dit (« OBS n'est pas lancé », nouvel essai
toutes les 5 s ; « Serveur WebSocket d'OBS coupé » si ses réglages le disent ; « Mot de passe
refusé par OBS » : un clic réessaie).

## Page « OBS »
En tête : la scène diffusée, deux pastilles **DIRECT** et **REC** (rouges avec leur durée quand ils
tournent, orange en pause ou en reconnexion ; durée relue toutes les 2 s, interpolée entre deux).

| Entrée | Tourner | Cliquer / maintenir |
|--------|---------|---------------------|
| molette 1 | choisit une scène dans la liste (5 par écran, « scène 2/7 ») | l'affiche ; en mode studio : la met en aperçu, un second clic sur la scène en aperçu fait la transition |
| molettes 2-4 | volume d'une source audio, le long du curseur d'OBS (`pas` % du curseur par cran) | sourdine |
| boutons 1-4 | — | au choix (formulaire) ; par défaut direct, enregistrement, pause, replay |

Colonne des scènes : barre rouge = programme, verte = aperçu (mode studio), fond = sélection.
Colonnes 2-4 : nom de la source (rouge si coupée), vumètre (vert, orange au-dessus de -20 dB,
rouge au-dessus de -9 dB, -60 à 0 dB comme OBS, 10 images/s, retombée progressive) et curseur
(bleu, gris si coupée) ; en dessous le volume en dB (« -10,0 dB », « coupé »). Sources des molettes
par défaut : micro (`mic1`), audio du bureau (`desktop1`), puis les suivantes dans l'ordre d'OBS.

Boutons possibles : direct, enregistrement, pause (de l'enregistrement), replay (lance la mémoire
de replay si elle est arrêtée, sinon la sauvegarde ; « REPLAY DÉSACTIVÉ DANS OBS » si elle n'est
pas activée dans Paramètres → Sortie), caméra virtuelle, transition, mode studio, rien. Avec
`maintenir_pour_arreter` (par défaut), arrêter le direct ou l'enregistrement depuis la page demande
de **maintenir** le bouton (un appui court : pilule « MAINTENIR POUR ARRÊTER LE DIRECT ») ; démarrer
se fait d'un appui. LED vive quand l'état est actif (direct, enregistrement, pause…).

**Volume** : le curseur du mixeur d'OBS est logarithmique (`log_def` de libobs : -96 dB en bas,
0 dB en haut, `position()` / `multiplier()` dans `main.py`) ; les crans le déplacent comme la souris
dans OBS. Molette tournée vite : seule la dernière valeur part, et les échos plus anciens d'OBS
(`InputVolumeChanged`) sont ignorés 1 s.

**Pilules** (réglables) : changement de scène (aussi depuis OBS), source d'une molette coupée /
rétablie, « EN DIRECT », « DIRECT TERMINÉ », « DIRECT : RECONNEXION… », « ENREGISTREMENT EN PAUSE »,
« REPLAY SAUVEGARDÉ », « CAMÉRA VIRTUELLE : OUI »… ; un refus d'OBS en rouge (« PAUSE : PAS EN
COURS », « TRANSITION : MODE STUDIO INACTIF »…).

## Commandes (onglet Commandes, toutes les pages)
`direct`, `enregistrement` (démarrer / arrêter sans confirmation : à lier de préférence à un
maintien), `pause`, `replay`, `camera`, `transition`, `studio`, `scenes` (molette : scène suivante /
précédente) ; une par scène `scene:<nom>`, par source audio `couper:<nom>` et `volume:<nom>`
(molette ; texte « Mic/Aux -10,0 dB », valeur = position du curseur pour les jauges), par profil
`profil:<nom>`, par collection `collection:<nom>`, et par favori : `source:<scène>/<source>`
(afficher / masquer), `filtre:<source>/<filtre>`, `media:<source>` (relance un média depuis le
début ; il doit être dans la scène affichée pour s'entendre). LED : l'état (scène diffusée, source
coupée, source visible, filtre actif, média en lecture, profil courant…). La liste suit OBS (scène
ajoutée, renommée…) ; celle de la dernière connexion (`~/.cache/hsm01/obs/connu.json`) est proposée
dès le démarrage, pour que les liaisons marchent avant qu'OBS réponde.

## Widget « État d'OBS »
220 × 68 : la scène diffusée, le direct et l'enregistrement avec leur durée (onglet Widgets →
« module » → OBS → État d'OBS).

## Réglages (formulaire, `obs.toml`)
`[molettes]` `k2`…`k4` (sources audio, « (par défaut) » : automatique), `pas` (0,5 à 10 % du curseur
par cran, 2 par défaut), `vumetres`, `[boutons]` `b1`…`b4`, `maintenir_pour_arreter`, favoris
`[[source]]` (`scene`, `source` : listes données par le module), `[[filtre]]` (`source`, `filtre`),
`[[media]]` (`source`), `[pilules]` `scene`, `son`, `sorties`, `adresse`, `port`.

## Code et essai
`obs_ws.py` : client WebSocket (RFC 6455, masquage, fragments, ping, fermeture et son code) et
protocole obs-websocket v5 (Hello, Identify avec le mot de passe
`base64(sha256(base64(sha256(mot de passe + sel)) + défi))`, requêtes et réponses par identifiant,
évènements ; code 4009 : mot de passe refusé), bibliothèque standard seulement. `main.py` : fil
`obs` (connexion, lecture complète `read_all`, file d'actions — aucune requête pendant qu'un
gestionnaire tient le verrou ; relecture complète quand une scène, une source, un filtre ou la
collection change), évènements d'OBS (fil du client WebSocket), page, widget, commandes.
L'ordre des scènes : `GetSceneList` les donne à l'envers de la fenêtre d'OBS, triées par
`sceneIndex` décroissant ; idem pour les sources d'une scène (`sceneItemIndex`).

Faux OBS : `tools/fake_obs.py` (mot de passe, trois scènes, sources audio avec vumètres, caméra à
filtres, média, sorties, mode studio, profils, collections ; `--no-replay`). Test :
`TMPDIR=… cargo test -- --ignored obs_module` (port et mot de passe lus dans de faux réglages
d'OBS, page, molettes, commandes, arrêt par maintien, favoris, mode studio, widget, listes du
formulaire, mauvais mot de passe ; aperçu `hsm01-obs.png`). Pas encore essayé avec le vrai OBS
(32.2 installé ici, serveur WebSocket à activer).
