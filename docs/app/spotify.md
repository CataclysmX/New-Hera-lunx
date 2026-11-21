# Module « Spotify » (2026-11-20)

Ce que propose le logiciel Hercules (traductions `herculescreator_fr.ts`, classes `Spotify*` de
`Stream-control.exe`) : lecture / pause, morceau suivant / précédent, « J'aime », sourdine,
aléatoire, répétition (titre / tout / non), lancer une liste de lecture, y ajouter ou en retirer
le morceau ; par l'API web de Spotify avec une application Spotify Developer (ID + secret) et un
compte **Premium** pour piloter la lecture. Le module livré `app/modules/spotify/` fait de même,
plus une radio. Sans application Spotify, le module Lecteur média pilote déjà Spotify (MPRIS :
lecture, suivant, position, volume du lecteur), sans « J'aime » ni listes.

## Mise en route (une fois)
1. Onglet Modules → « ⟳ Rafraîchir les modules » si besoin → allumer « Spotify » → Enregistrer ;
   son onglet « Spotify » apparaît (guide en tête, lien vers le tableau de bord).
2. https://developer.spotify.com/dashboard → « Create app » (API : Web API) ; Settings →
   Redirect URIs : `http://127.0.0.1:8888/callback` (= `redirection` du formulaire ; Spotify
   n'accepte plus `localhost`, il faut l'adresse 127.0.0.1) → Save.
3. Client ID et Client secret dans « Données sensibles » → Enregistrer : le module ouvre le
   navigateur sur la page d'accord de Spotify ; « Accepter » renvoie vers le module (petit
   serveur local d'une requête sur 127.0.0.1:8888), qui échange le code contre un jeton gardé
   dans son `.env` (0600 ; renouvelé seul ensuite).

## Page « Spotify »
Pochette (téléchargée dans `~/.cache/hsm01/spotify/`), titre, artiste (ou émission), album,
avancement (relu toutes les 2 s, interpolé entre deux), appareil et état en tête.

| Entrée | Tourner | Cliquer / maintenir |
|--------|---------|---------------------|
| molette 1 | position, 5 s par cran | lecture / pause |
| molette 2 | — | aléatoire |
| molette 3 | — | répétition : non → tout → titre |
| molette 4 | volume de Spotify (`pas` % par cran) | sourdine (le volume revient au clic suivant) |
| boutons 1-4 | — | au choix (formulaire, section « Boutons de la page ») ; par défaut précédent, lecture / pause, suivant (**maintien : radio du titre**), J'aime |

Chaque bouton : précédent, lecture / pause, suivant, J'aime, **radio du titre**, aléatoire,
répétition, sourdine ou rien ; icône et libellé à l'écran, LED vive quand son option est active
(lecture, aimé, aléatoire, répétition, sourdine). Maintenir le bouton « suivant » ou « radio du
titre » lance la radio. Chaque action dit ce qu'elle a fait par une pilule verte (« AJOUTÉ AUX
TITRES LIKÉS », « ALÉATOIRE : OUI »…) ; un refus de Spotify en rouge (« SPOTIFY PREMIUM REQUIS… »,
« AUCUN APPAREIL SPOTIFY ACTIF… »). Sans appareil actif, la commande est refaite sur le Spotify de
cet ordinateur (`/me/player/devices`, type Computer).

**Ce que Spotify interdit ici** (`actions.disallows` de `/me/player`) : un titre joué seul (sans
liste, album ni artiste : lancé depuis la recherche…) ne peut ni passer en aléatoire ni se
répéter. Spotify n'annonce que « tout » comme interdit mais refuse aussi « titre »
(`Restriction violated`, vu le 2026-11-20). La répétition saute les modes annoncés interdits ; une
action interdite ou refusée ainsi donne une pilule grise « … : IMPOSSIBLE ICI » au lieu d'une
erreur rouge. Lecture / pause se fie à ce que Spotify autorise
(`pausing` interdit : ça ne joue pas) et à ce qui vient d'être demandé (2 s : Spotify met un
moment à le dire ; deux clics rapprochés alternent bien).

**Radio** : les recommandations de Spotify à partir du morceau (`/recommendations`) ; elles sont
fermées aux applications créées depuis fin 2024 (404) : le module lance alors le catalogue de
l'artiste du morceau, puis l'aléatoire (dans cet ordre : sans artiste en cours, Spotify refuse
l'aléatoire), ce qui s'en approche le plus.

## Commandes (onglet Commandes, toutes les pages)
`play_pause`, `next`, `previous`, `like`, `shuffle`, `repeat`, `mute`, `radio` (boutons, LEDs :
lecture, aimé, aléatoire, répétition, sourdine), `volume` (molette ; texte « Spotify 55 % »,
valeur pour les jauges) ; par liste favorite : `lancer:<nom>` (LED allumée quand elle joue),
`ajouter:<nom>` (« déjà dedans » s'il y est), `retirer:<nom>`.

## Réglages (formulaire, `spotify.toml`)
`pas` (volume, % par cran), **listes de lecture favorites** choisies dans la liste des vôtres
(donnée par le module connecté, relue toutes les 5 min), `[boutons]` `b1`…`b4` (ce que fait
chaque bouton de la page ; valeur inconnue : celle par défaut), `redirection`.

## Code et essai
`spotify_api.py` (OAuth : `authorize_url`, `wait_code`, `token` ; `call` et ses erreurs : statut,
`reason`, `Retry-After` ; `where` : l'appel refusé, écrit au journal), `main.py` (fil `spotify` :
jeton, lecture de l'état, file d'actions — aucun appel réseau pendant qu'un gestionnaire
d'événement tient le verrou ; page ; commandes).

**API web de Spotify en 2026** (constaté le 2026-11-20 avec une application récente) :
- « J'aime » : `/me/tracks` (`ids`) répond 403 ; c'est `/me/library` (`PUT` / `DELETE`, `uris`,
  titres et épisodes) et `GET /me/library/contains?uris=…` ;
- listes : `/playlists/{id}/tracks` répond 403 ; c'est `/playlists/{id}/items` (`GET` : chaque
  élément a un champ `item`, plus `track` ; `POST` `{"uris": […]}` ; `DELETE`
  `{"items": [{"uri": …}]}`) ;
- les commandes du lecteur (lecture, suivant…) répondent 200 avec un texte qui n'est pas du JSON :
  une réponse réussie illisible est donc prise pour « fait ».
Faux Spotify : `tools/fake_spotify.py` (autorisation qui renvoie aussitôt le code, jetons, lecteur
de trois morceaux, J'aime, listes « Ma liste » et « Sport », recommandations fermées, adresses de
2026 et 403 sur les anciennes, commandes du lecteur qui répondent du texte ; `--free` : Premium
requis). Test : `TMPDIR=… cargo test -- --ignored spotify_module` (autorisation par `curl -sL` en
guise de navigateur, `.env` en 0600, page, commandes, entrées, liste, radio ; aperçu
`hsm01-spotify.png`). Essayé avec le vrai Spotify (2026-11-20) : autorisation, état du lecteur,
J'aime relu, lecture / pause ; ajout et retrait dans une liste vérifiés sur leur format seulement.
