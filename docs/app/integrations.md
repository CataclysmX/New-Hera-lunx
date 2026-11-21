# Intégrations du logiciel officiel (Stream Control 2.5.x) et équivalents Linux

Détail par intégration : [Discord](discord.md), [Spotify](spotify.md), [OBS](obs.md) (modules livrés : faits).

Source : chaînes et décompilation de `Stream-control.exe` (classes `herculescreator::*Connector`,
`*ActionHandler`, `*DataManager`, `*Stepper`). Principe : chaque molette/bouton d'une **page** est lié à
une **action** d'un plugin ; les LEDs et l'écran reflètent l'état (stream on, mute, niveau VU…).

| Intégration | Transport / protocole (Windows) | Fonctions vues | Équivalent Linux |
|-------------|--------------------------------|----------------|------------------|
| **Audio** (volumes, VU) | Windows Core Audio / WASAPI (`IAudioSessionControl`, `IAudioEndpointVolume`, sessions par app, mètres de crête) | volume app / entrée / sortie, mute, « App Loop », « Foreground App », VU par app | **PipeWire/PulseAudio** (`libpulse`/`pw` : sink-inputs par app, sources, sinks, moniteurs de crête) ✔ |
| **OBS Studio** | obs-websocket **v5** `ws://127.0.0.1:4455`, auth challenge/salt SHA-256, `op`/`d` JSON | `GetSceneList`, `SetCurrentProgramScene`, `ToggleStream`, `ToggleRecord`, `Get/SetInputMute`, `Get/SetInputVolume`, `SetSceneItemEnabled`, `SetSourceFilterEnabled`, `TriggerMediaInputAction`, profils/collections, évènements d'état | identique (OBS + websocket existent sous Linux) ✔ |
| **Streamlabs Desktop** | SockJS JSON-RPC `ws://127.0.0.1:59650/api/…/websocket` + token | scènes, stream, enregistrement | ✘ Streamlabs n'existe pas sous Linux |
| **Twitch** | OAuth `id.twitch.tv` + API **Helix** | pub (`channels/commercial`), marqueurs, clips, annonces, messages/paramètres de chat, shield mode, raids, EventSub | identique (HTTP) ✔ – app Twitch à enregistrer |
| **Discord** | RPC local `discord-ipc-N` (named pipe) : `AUTHORIZE`/`AUTHENTICATE`, `SUBSCRIBE VOICE_*`, réglages voix ; OAuth `discord.com/api/oauth2/token` | mute/sourdine soi-même, mute/volume membre, rejoindre/quitter un salon | socket Unix `$XDG_RUNTIME_DIR/discord-ipc-0`, même protocole ◐ (scope `rpc` réservé aux apps approuvées par Discord) |
| **Spotify** | Web API + OAuth (callback `http://127.0.0.1:8888/callback`), scopes playback/library/playlist | play/pause, suivant/précédent, volume, shuffle/repeat, like, playlists | Web API identique ✔, ou **MPRIS (D-Bus)** sans compte pour la lecture |
| **Philips Hue** | découverte `discovery.meethue.com`, appairage bouton du pont, API v1 `/api/<user>/config` + **CLIP v2** (`resource/light`, `grouped_light`, `room`, `zone`, `scene`) | on/off, couleur, température, luminosité, scénario, boucle de couleurs | identique (HTTPS LAN) ✔ |
| **Voicemod** | websocket local port 59129 | changement de voix | ✘ Windows uniquement |
| **SteelSeries Sonar** | cartes son virtuelles détectées par nom | mixage | ✘ Windows uniquement |
| **Raccourcis clavier** | `SendInput` | combinaisons de touches | **uinput** (clavier virtuel) ✔ |
| **Lancer fichier / page web** | `QDesktopServices` | ouvrir un fichier/URL | `xdg-open` (Qt fait pareil) ✔ |
| **Pages** | interne | page suivante/précédente/aller à | interne ✔ |

Télémétrie : l'app envoie des évènements à **Google Analytics 4** (Measurement Protocol, `measurement_id`
+ `api_secret` embarqués) et des rapports de crash **BugSplat** – à ne pas reproduire.
Check de MAJ : manifeste `ts.hercules.com` (cf. [../re/firmware-update.md](../re/firmware-update.md)).
