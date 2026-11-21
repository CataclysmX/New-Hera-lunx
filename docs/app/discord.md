# Intégration Discord (étude puis module, 2026-11-20)

Ce que fait le logiciel officiel (Stream Control 2.5.x : traductions `herculescreator_fr.ts`,
classes `Discord*Control` / `*CustomizationPage` de `Stream-control.exe`), et ce qui est faisable
ici. Vue d'ensemble des intégrations : [integrations.md](integrations.md).

## Les options du logiciel Hercules
| # | Action (Hercules) | Commande | Réglages (page de personnalisation) |
|---|-------------------|----------|-------------------------------------|
| 1 | **Me mettre en sourdine** (*Mute myself*) | bouton | — |
| 2 | **Mode sourd** (*Deafen myself*) | bouton | — |
| 3 | **Mon entrée** (*Own input* : volume du micro dans Discord) | molette | — |
| 4 | **Ma sortie** (*Own volume (output)*) | molette | — |
| 5 | **Se déconnecter** du salon vocal | bouton | — |
| 6 | **Rejoindre / quitter un salon** choisi | bouton | serveur + salon |
| 7 | **Volume d'un membre** du salon | molette ou bouton | membre ; mode *Ajuster* (pas) ou *Définir* (niveau) |
| 8 | **Couper un membre** (pour soi) | bouton | membre |
| 9 | **Push-to-talk ↔ détection de la voix** | bouton | — |

États montrés à l'écran : *Not connected*, *No channel*, *No user*, *User absent*, *No selection* ;
LED = état (micro coupé, sourd…). Connexion : « Autoriser Stream sur votre client Discord »
(fenêtre d'autorisation du client Discord), puis jeton OAuth.

## Comment Discord se pilote
Le client Discord ouvre un **RPC local** : socket Unix `$XDG_RUNTIME_DIR/discord-ipc-0` (paquet ou
AUR ; Flatpak : `app/com.discordapp.Discord/discord-ipc-0`), trames `[op u32][len u32][JSON]`.
Commandes : `AUTHORIZE` (le client affiche la demande d'autorisation) → code échangé contre un jeton
(`POST https://discord.com/api/oauth2/token`, avec l'**ID et le secret de l'application**) →
`AUTHENTICATE`. Ensuite : `GET/SET_VOICE_SETTINGS` (mute, deaf, volumes entrée/sortie, mode PTT /
voix), `SELECT_VOICE_CHANNEL` (rejoindre, `null` = quitter), `GET_GUILDS` / `GET_CHANNELS`,
`SET_USER_VOICE_SETTINGS` (volume, mute d'un membre), `SUBSCRIBE` à `VOICE_SETTINGS_UPDATE`,
`VOICE_STATE_CREATE/UPDATE/DELETE`, `SPEAKING_START/STOP`, `VOICE_CHANNEL_SELECT` (pour les LEDs et
l'écran).

**Limite** : ces commandes demandent le scope `rpc` (+ `rpc.voice.read/write`), que Discord ne
délivre qu'aux applications approuvées… **sauf à leur propriétaire** (et aux testeurs ajoutés dans le
portail). L'application de Hercules n'est pas réutilisable (son ID et son secret sont les siens).
Donc : **chacun crée sa propre application Discord** (portail développeur, 5 min : ID + secret +
une adresse de redirection), l'app fait le reste ; pas d'autre serveur que discord.com pour le jeton.
Vesktop / Vencord n'a pas ce RPC (son arRPC ne fait que l'activité affichée) : avec lui, seulement
la voie « sans RPC » ci-dessous.

## Faisable ici
| # | Avec le RPC (client officiel + ton application) | Sans RPC (aucun compte développeur) |
|---|-----------------------------------------------|-------------------------------------|
| 1, 2 | ✔ + LED / écran à jour (événements) | ◐ raccourci clavier de Discord (« Activer/désactiver le micro / le son ») par l'action clavier de l'app ; sous Wayland, Discord (XWayland) ne voit les raccourcis globaux que si KDE l'autorise (Paramètres → Prise en charge des applications X11 → lecture des frappes) ; pas d'état en retour ; ou micro coupé **pour tout le système** (déjà possible : cible « entrée par défaut ») |
| 3 | ✔ volume d'entrée de Discord (0–100 %) | ◐ volume du flux d'enregistrement de Discord dans PipeWire (nouvelle cible de molette « entrée d'une application ») |
| 4 | ✔ volume de sortie de Discord (0–200 %) | ✔ déjà possible : molette sur l'application `discord` (PipeWire) |
| 5, 6 | ✔ quitter ; rejoindre un salon choisi dans la fenêtre (liste des serveurs / salons) | ✘ |
| 7, 8 | ✔ membre choisi par son nom (liste du salon), volume 0–200 % par pas ou fixé, mute local | ✘ |
| 9 | ✔ | ◐ raccourci, s'il en existe un dans Discord |
| + | ✔ page « salon vocal » : membres (avatars ronds), qui parle en surbrillance, micro/son coupés | — |
| + | notifications (messages, mentions) : **déjà fait** (popup D-Bus) | idem |

## Projet open source : pas d'application Discord commune
Une application commune au projet ne marcherait pas : son **secret** (nécessaire pour obtenir le
jeton) serait publié avec le code, donc plus secret ; et surtout les commandes vocales du RPC ne
servent qu'au propriétaire de l'application et à ses testeurs déclarés, sauf approbation par
Discord (en pratique fermée). Un serveur du projet gardant le secret ne lèverait pas cette seconde
limite. Donc : **chaque utilisateur crée la sienne** et saisit dans l'onglet du module son **ID**
et son **secret** (champ masqué) — pas un jeton : le module l'obtient seul (fenêtre d'autorisation
du client Discord) et le garde dans son dossier (droits 0600), hors de `config.toml`. Un petit
guide dans l'onglet : créer l'application, ajouter l'adresse de redirection, copier ID et secret.
Sans application, la voie « sans RPC » reste disponible.

## Le module « Discord » (fait)
Module livré `app/modules/discord/` (Python, bibliothèque standard seulement) : `discord_ipc.py`
(RPC local : trames, `AUTHORIZE` / `AUTHENTICATE`, `SUBSCRIBE` ; jeton OAuth2 par `urllib`),
`main.py` (page, entrées, commandes), `module.toml` (`[[secret]]` ID et secret), `discord.toml`
(réglages, guide en tête). Les 9 options Hercules y sont, avec le RPC du **client officiel**.

**Mise en route** (une fois) : onglet Modules → allumer « Discord » ; son onglet « Discord »
apparaît. Créer son application sur https://discord.com/developers/applications (« New
Application »), OAuth2 → Redirects → ajouter `http://localhost` (= `redirection` de
`discord.toml`) et **Save Changes** — **une seule adresse** : la demande d'autorisation par le RPC
n'en accepte pas (« Redirect URI cannot be used in the RPC OAuth2 Authorization flow »),
discord.com prend donc celle de l'application ; sans adresse enregistrée (ou avec plusieurs),
Discord répond « Missing "redirect_uri" in request » après le clic sur « Autoriser », ce que
l'écran du module traduit en marche à suivre ; puis copier le **Client ID** et le **Client Secret** (« Reset Secret ») dans « Données
sensibles » → Enregistrer. Le module se relance, le client Discord demande d'autoriser
l'application : « Autoriser ». Le jeton obtenu (et son renouvellement) va dans le `.env` du module
(0600, `DISCORD_ACCESS_TOKEN`, `DISCORD_ACCESS_EXPIRES`, `DISCORD_REFRESH_TOKEN`) : plus de
demande aux lancements suivants ; un jeton refusé est oublié et l'autorisation redemandée.
Juste après une autorisation, Discord ferme les connexions de l'application (vu le 2026-11-20) :
le module se reconnecte seul en quelques secondes. Journal du module aussi dans
`~/.cache/hsm01/discord/journal.log` (étapes, erreurs de Discord mot pour mot ; jamais de jeton).
Diagnostic : testé en vrai le 2026-11-20 (client officiel, salon de 5 membres : autorisation,
abonnements, réglages voix, événements « qui parle »).

**Page « Discord »** (menu des modules) : le salon vocal où l'on est (nom, serveur), ses membres
en cases (avatar rond téléchargé dans `~/.cache/hsm01/discord/`, anneau vert quand il parle, icône
micro coupé / sourd, volume ou « coupé pour vous »), 8 par écran (`1/2` en haut à droite au-delà) ;
hors salon, les salons favoris. Entrées :

| Entrée | Tourner | Cliquer |
|--------|---------|---------|
| molette 1 | choisir un membre (un salon favori) | couper ce membre pour soi (#8) / rejoindre le salon |
| molette 2 | volume du membre choisi, 0–200 % (#7) | le remettre à 100 % |
| molette 3 | mon entrée, 0–100 % (#3) | couper le micro |
| molette 4 | ma sortie, 0–200 % (#4) | mode sourd |
| boutons 1-4 | — | micro (#1), sourd (#2), push-to-talk / voix (#9), quitter le salon (#5) |

Maintenir la molette 1 ouvre toujours le menu des modules ; power : retour. LEDs de la page :
micro coupé, sourd, push-to-talk (100 %, sinon 15 %), dans un salon (60 %). États à l'écran : à
configurer (pas d'ID ou de secret), Discord pas lancé (nouvel essai toutes les 5 s, ou au clic),
autorisation en attente, refusée (clic : redemander), erreur (clic : réessayer ; détail dans le
Journal), pas dans un salon.

**Pilules d'arrivée et de départ** (formulaire, section « Pilules ») : quelqu'un rejoint mon salon
vocal → bandeau vert « NOM A REJOINT LE SALON » (3 s, activé par défaut, `pilule_arrivee`) ;
quelqu'un le quitte → bandeau gris « NOM A QUITTÉ LE SALON » (désactivé par défaut,
`pilule_depart`). Pas pour moi-même, ni pour les présents quand j'arrive dans un salon.

**Widgets « Membres du salon vocal »** (onglet Widgets → « + Ajouter un widget » → « module (sa
page ou un de ses widgets) » → module Discord → montrer : l'un des deux) : par membre, avatar
rond, anneau vert quand il parle, nom, volume ou « coupé pour vous », icône micro coupé / sourd ;
hors salon ou déconnecté : une ligne d'état.
- **en colonne** (`membres`) : une ligne de 40 px par membre, 220 px de large ; sa hauteur suit le
  nombre de membres ; « trop grand : défiler ↕ » conseillé ;
- **en ligne** (`membres_ligne`) : une case de 150 px par membre, côte à côte, 40 px de haut ; sa
  largeur suit le nombre de membres ; « trop grand : défiler ↔ » conseillé (dans une bande de
  30 px de haut : réduit aux 3/4, défile de gauche à droite).

**Commandes** (onglet Commandes, sur toutes les pages, LEDs « état d'une commande de module ») :
`mute` « Couper le micro », `deafen` « Mode sourd », `input_volume` / `output_volume` (molettes ;
texte « Entrée 80 % », valeur pour les jauges), `disconnect`, `voice_mode` ; par salon favori
`salon:<serveur>/<salon>` « Rejoindre / quitter » (#6, LED allumée dans ce salon) ; par membre
suivi `volume:<nom>` (molette, #7 « Ajuster »), `couper:<nom>` (#8), et avec `niveau`
`niveau:<nom>` « … à N % » (#7 « Définir »). Membre absent du salon : commande grisée, « absent ».

**Réglages** (onglet « Discord », un **formulaire** ; enregistrer relance le module) : le guide de
mise en route en tête (lien vers le portail), les données sensibles, puis : pas des volumes (% par
cran) ; **salons favoris** à choisir dans des listes (vos serveurs, puis les salons vocaux du
serveur choisi, donnés par le module connecté) ; **membres suivis** à choisir parmi ceux croisés
dans vos salons (mémorisés dans `~/.cache/hsm01/discord/membres.json`), avec un niveau facultatif
(bouton « … à N % ») ; avancé : l'adresse de redirection. Fichier : `discord.toml` (`pas`,
`[[salon]] serveur = … salon = …`, `[[membre]] nom = … niveau = …`, `redirection`) ; un nom
introuvable : le Journal liste les serveurs ou salons vocaux existants.

**Essai sans Discord** : `tools/fake_discord.py` imite le client (RPC, jeton, avatars ; un serveur,
deux salons, trois membres, Alice parle) ; test complet : `TMPDIR=… cargo test -- --ignored
discord_module` (autorisation unique, `.env` en 0600, page, avatars, commandes, entrées, Discord
qui part et revient ; aperçus `hsm01-discord.png`, `hsm01-discord-sans-id.png`). À vérifier avec
le vrai client : noms exacts des erreurs, `force` de `SELECT_VOICE_CHANNEL`, changement de mode
push-to-talk sans raccourci défini.
