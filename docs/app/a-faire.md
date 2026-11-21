# À faire – new hera-nux (note du 2026-11-20)

État : firmware custom **240.21** flashé (COPY pour les défilements ; entrées qui ne se figent plus depuis 240.20, voir `firmware/stage2.md`) ; app `hsm01-app` (« new hera-nux ») complète sur les
4 étapes prévues + les ajouts listés dans [companion.md](companion.md). Relancer l'app :
`cd app && ./target/release/hsm01-app` (diagnostic : `HSM01_STATS=1 … --headless`).

## À tester sur l'appareil (fait par le code, pas encore vu en vrai)
- [ ] **Plein écran** : molette sur « application en plein écran », lancer un jeu (natif et Proton) :
  le son suit-il ? l'**icône du jeu Proton** s'affiche-t-elle (ICO Steam / icône 32 px / en-tête) ?
- [ ] **Spotify** : icône visible (repli sur le nom de l'application).
- [ ] **Image de fond** : PNG / JPEG / GIF animé sur une page ; lisibilité avec « Assombrir » ;
  fluidité des molettes pendant un GIF plein écran.
- [ ] **Aimants** dans l'aperçu (alignement de deux icônes à la même hauteur, Alt = libre).
- [ ] **Étalonnage** avec la mire (onglet Appareil → Étalonnage des couleurs → Régler).
- [ ] Dégradés (texte, jauges, VU), « maintien » des boutons, confirmation > 100 %, réveil de veille,
  luminosité 0 % via « définir » / « cycle ».
- [ ] **Styles de jauge** (onglet Widgets → VU-mètre / jauge → style) : segments, arc, aiguille,
  images (piste + remplissage) ; rendu sur l'écran et fluidité.
- [ ] **Ctrl+Z / Ctrl+Maj+Z** dans la fenêtre (gestes regroupés, champs de texte non concernés).
- [ ] Molettes / boutons toujours réactifs après des heures (240.20) ; sinon `tools/fw_probe.py`.
- [ ] Icône de la barre système (version simplifiée `public/tray-symbolic.svg`) : lisible ?
- [ ] **Notifications** : onglet Appareil → Notifications → « Tester », puis un vrai message
  Discord (avatar rond ? texte coupé proprement ?) ; réveil de veille ; un clic ferme sans agir ;
  page visualiseur : le spectre continue autour de la popup.
- [ ] **Séparateur** (plein / tirets / points / fondu, vertical) et **bords des cadres** (schéma
  cliquable, préréglages) sur l'écran.
- [ ] **API** (relancer l'app d'abord) : module « Menu de raccourcis » (onglet Modules) → page
  « Menu » : défilement des rangées fluide (COPY 240.21) sans traces, bouton k = case k, dossier
  Médias + Retour, LED des boutons ; `app/target/release/hsm01ctl alert "Test" --led 2 --wait` ;
  page web `app/clients/web/alert.html` avec l'adresse copiée (Appareil → API ouverte).
- [ ] Textes qui défilent (noms longs) : toujours nets avec la détection de défilement.
- [ ] **Modules** (onglet « Modules » → + Ajouter) : « Lecteur média » (page Média : pochette
  Spotify / navigateur, boutons 1-2-3, molettes 1 et 4, `--alert`) et « Système » (page, alertes) ;
  couper / rallumer l'interrupteur, Journal ; ne pas oublier Enregistrer.
- [ ] **Modules en dossiers** : au premier lancement, `~/.local/share/hsm01/modules/` reçoit media,
  system, deck ; « Rafraîchir les modules » ; « Créer un module » (nom → dossier ouvert, page « Mon
  module », molette 1, bouton 1) ; modifier `main.py` → le module se relance seul ([modules.md](modules.md)).
- [ ] **Widget « module »** : activer « Menu de raccourcis » (onglet Modules ; il ne doit plus sauter
  sur sa page au lancement), page 2 → + Ajouter un widget → « module (sa page) » : menu réduit dans
  le cadre, molette 4 = rangées, boutons 1-4 = cases, molettes 1-3 toujours au volume.
- [ ] **Stream deck** : onglet Widgets → + Ajouter → « stream deck (une molette) » (colonne
  au-dessus de la 1re molette libre) : tourner = case suivante avec glissement, clic = action
  (média par défaut), « ↔ ligne ». Module « Stream deck » (onglet Modules, arrivé tout seul) :
  bouton « afficher un module » vers lui, 4 pistes ; son onglet « Stream deck » (module activé) :
  disposition lignes/colonnes, pistes, cases → Enregistrer → le module se relance avec.
- [ ] **Arrêt → veille** : « Quitter » (barre système) ou Ctrl+C dans un terminal → écran et LEDs
  éteints ; relancer l'app → rallumés, page redessinée.
- [ ] **Sans app (firmware 240.22)** : 3 s après l'arrêt (ou `kill -9`) → badge « SIGNAL NOT FOUND »
  qui rebondit à 1 % (lisible ? couleur qui change à chaque bord) ; écran noir après 5 min ;
  relancer l'app → page normale. **PC en veille** → écran et LEDs éteints, au réveil → page de
  l'app intacte.
- [ ] **Alertes des modules en veille** : `--alert` du Lecteur média rallume l'écran (réglage
  Notifications → réveil) ; alertes du module Système aussi.
- [ ] **Menu des modules** : maintenir la molette 1 → bandeau qui monte du bas (transition fluide ?),
  molette 1 = choisir, clic molette 1 = page du module, power = retour à la page d'avant ; autre
  clic ou 10 s → il redescend. Power et page suivante ne passent plus par les pages de modules.
- [ ] **Renommage « modules »** : au lancement, `~/.local/share/hsm01/tools/` devient `modules/`
  (fichiers `module.toml`), la config garde ses modules, onglets et textes disent « module ».
- [ ] **Données sensibles** : un module avec `[[secret]]` (le module Discord) → son onglet,
  section « Données sensibles » → Enregistrer → `.env` en 0600, module relancé avec ses variables.
- [ ] **Alerte « pilule »** : onglet Appareil → Notifications → « Tester une pilule » : en bas,
  monte en glissant, points qui « respirent », redescend au bout de 3 s ; fluide sur l'appareil ?
- [ ] **Une alerte par application** : changer plusieurs fois de morceau sur Spotify (Lecteur média
  `--alert`) → seule la dernière s'affiche, tout de suite.
- [ ] **Module arrêté pendant qu'on est sur sa page** (onglet Modules : le couper) → retour à la page
  de la config d'où on venait.
- [ ] **Module Système** : l'allumer (onglet Modules), son onglet « Système » (`systeme.toml`) ; en
  jeu, plus d'alerte de carte graphique tant que `edge` reste sous 90 °C ; Journal : liste des
  sondes ; essayer `style = "pilule"`.
- [ ] **Discord, pilule d'arrivée** : quelqu'un rejoint ton salon vocal → « NOM A REJOINT LE SALON » en bas ; départ (à activer dans le formulaire) → pilule grise.
- [ ] **Module Spotify** avec le vrai Spotify ([spotify.md](spotify.md)) : application Spotify
  Developer, redirection `http://127.0.0.1:8888/callback`, ID + secret → navigateur → Accepter ;
  page (pochette, molettes, boutons, maintien du bouton 3 = radio), pilules, J'aime, une liste
  favorite (lancer / ajouter / retirer) ; sans Premium : le message « Premium requis ».
  Fait le 2026-11-20 : autorisation, état, lecture / pause, J'aime relu (adresses de 2026) ;
  reste : ajouter / retirer dans une liste, boutons et clics réglables, radio sur le clic molette 1.
- [ ] **Module OBS** avec le vrai OBS ([obs.md](obs.md)) : Outils → Paramètres du serveur WebSocket →
  Activer ; allumer le module OBS (Rafraîchir les modules) ; page (scènes, molettes 2-4 = mixeur,
  vumètres), direct / enregistrement (arrêt en maintenant le bouton), widget « État d'OBS ».
- [ ] **Motor Town** ([telemetrie.md](telemetrie.md)) : allumer le module (Rafraîchir les modules) ;
  en jeu, adresse 127.0.0.1 port 33330, format Motor Town v1, au volant ; écrans Conduite /
  Course / Véhicule (molette 1), LEDs compte-tours, pilules. **SimHub** : le flux compatible SimHub
  du jeu (UDP 8888) marche ; la source « programme SimHub » reste à essayer avec le vrai SimHub
  (serveur web port 8888) : noms des propriétés et forme des réponses supposés.
- [ ] **Veille bloquée par un module** : case « Veille » de sa fiche (onglet Modules) → l'écran ne
  s'éteint pas tant que sa page est affichée ; une autre page → veille après le délai habituel.
- [ ] **Page Système qui défile** (sans `--no-page`) : plus de 6 lignes (disques, batteries) →
  une molette fait défiler, barre à droite, « 1-6 sur N » en tête.
- [ ] **Emoji** : un pseudo Discord ou un titre Spotify avec emoji → en couleur sur l'appareil ;
  notification Discord avec emoji ; flèches / coches (police de secours).
- [ ] **Réglages en formulaire** : onglets « Discord » (guide + lien, salons favoris à choisir dans
  les listes de vos serveurs / salons, membres croisés, niveau) et « Système » (sondes de la
  machine dans une liste, seuils, durées) ; Enregistrer → le module se relance avec.
- [ ] **Widget de module** : onglet Widgets → « module » → Discord → montrer « Membres du salon
  vocal, en colonne », cadre plus petit que la liste, « trop grand : défiler ↕ » → la liste défile
  seule ; « …, en ligne » dans une bande, « défiler ↔ » → elle défile de gauche à droite.
- [ ] **Module relancé sur sa page** (Enregistrer ses réglages pendant que sa page est affichée) →
  l'écran principal un instant, puis sa page revient.
- [ ] **Discord, molette 2** : sur la page Discord, le clic remet le membre choisi à 100 % (pilule
  de confirmation) ; la sélection démarre sur un autre membre que vous.
- [ ] **Menu des modules depuis une page de module** : sur la page Média ou Stream deck, maintenir
  la molette 1 → le menu ; un appui court reste au module (Stream deck : la case part au
  relâchement).
- [ ] **Module Discord** avec le vrai client ([discord.md](discord.md), mise en route) : créer
  l'application, ID + secret dans l'onglet « Discord », « Autoriser » dans Discord ; dans un salon :
  avatars, anneau quand on parle, molettes 1-4, boutons 1-4 (micro, sourd, push-to-talk,
  quitter), LEDs ; un salon favori (`discord.toml`) → commande « Rejoindre / quitter » sur un
  bouton d'une page ; relancer l'app → pas de nouvelle autorisation.

## Reste à faire
1. ~~Styles de jauge~~ : faits (barre, segments LED, arc, aiguille, images libres) — à tester.
2. ~~Notifications~~ (Discord…) : faites (écoute D-Bus, filtre, réveil, clic pour fermer) — à
   tester. Pas encore : clignotement d'une LED à l'arrivée.
3. Optionnel : placer les **icônes des sources** sur la page Audio (petite icône à gauche des noms).
4. ~~Ergonomie de l'éditeur~~ : faite (poignées, sélection multiple, aligner / répartir, clavier)
   — à tester.
5. ~~Séparateur~~, ~~bords des cadres au choix~~ : faits — à tester.
6. **API ouverte** (choisie : Python, noms anglais, WebSocket en plus) : première version faite,
   [api.md](api.md) ; modules lancés par l'app + modules Média et Système faits. Suite possible
   ([api-externe.md](api-externe.md)) : surfaces en mémoire partagée, menus déclaratifs côté
   serveur, cibles audio des molettes sur les pages d'API, autres modules (Home Assistant… ;
   Discord, Spotify, OBS : faits, [discord.md](discord.md), [spotify.md](spotify.md), [obs.md](obs.md)), menu de raccourcis réglable sans code, bibliothèques Rust / JS, paquet.
   Scripts Lua intégrés ([scripts.md](scripts.md)) : mis de côté.

7. **Compatibilité firmware d'origine** : étude faite ([firmware-officiel.md](firmware-officiel.md)) ;
   option conseillée : hybride (éléments natifs + décor cuit dans le fond) et proposition de
   passer au firmware custom ; d'abord les vérifications sur l'appareil de la section 6.

## Points connus
- Un fond plein écran qui change entièrement à chaque image plafonne vers 3-4 images/s (USB) ;
  l'horloge d'animation saute des images plutôt que ralentir.
- Le logo de démarrage (firmware) n'est pas corrigé par l'étalonnage.
- L'enregistrement depuis la fenêtre garde les commentaires du fichier ; s'il ne peut pas les garder
  sans risque, il réécrit le fichier entier (copie dans `config.toml.bak`).
