# Tableau de bord de jeu : modules « Motor Town » et « SimHub » (2026-11-20)

Deux modules livrés, une même base : `hsm01/dashboard.py` (bibliothèque Python). Chaque module
traduit les données de son jeu en un **échantillon** (dictionnaire décrit en tête de
`dashboard.py` : vitesse, régime, rapport, pédales, carburant, températures, voyants, course,
roues…) et le donne à `Dashboard.feed` ; le tableau de bord le dessine 20 fois par seconde
(seulement ce qui change), ne montre pas ce que le jeu ne donne pas (absent, 0 qui veut dire
« indisponible », valeur non finie) et, sans échantillon depuis 1,5 s, affiche quoi faire.

## La page « tableau »
En tête : le véhicule, les clignotants (verts, au rythme du jeu), l'écran ; dessous le
**compte-tours** en 20 segments (vert, orange à 60 %, rouge à 85 % du régime maximal ; bleu
clignotant au rupteur). Trois écrans, à la **molette 1** (tourner ou cliquer ; maintien : menu des
modules) :

| Écran | Contenu |
|-------|---------|
| Conduite | vitesse, rapport (N, R, R2…, rouge au rupteur), régime, distance à la destination, panneau de limitation (rond rouge, fond rosé au-delà de +3 km/h), régulateur (« RÉG 90 ») ou pilote automatique (« AUTO »), pédales (accélérateur, frein, embrayage), carburant (barre, litres ou kWh), température moteur (rouge en surchauffe), turbo |
| Course | position (« P2 sur 8 »), tour (« tour 2/3 »), point de passage, temps du tour en gros (« ARRIVÉE », « DISQUALIFIÉ »), dernier, meilleur, total |
| Véhicule | les 4 roues (avant / arrière, gauche / droite d'après leur position) : pneu °C (fond rouge au-dessus de 100), frein, usure, suspension ; moteur (kW, N·m, usure), compteur, poids et charge, remorques, air |

Voyants en bas : ABS, TC, RÉG, LIM (limiteur), FEUX, PLEINS (phares), FREIN (à main), TEMP
(surchauffe). Ligne du bas : heure du jeu, destination, argent ; ou, si des **boutons** sont
réglés, leurs noms au-dessus des boutons 1-4, qui envoient leurs touches au jeu (action « raccourci
clavier » : le jeu doit avoir le focus). L'écran Course vient seul quand une course commence
(`ecran_course`).

**LEDs** (`leds_regime`) : compte-tours, les 4 LEDs s'allument à 70, 78, 86 et 93 % du régime
maximal et clignotent au rupteur. **Pilules** : « MOTEUR EN SURCHAUFFE », « RÉSERVE : FAIRE LE
PLEIN » (sous 10 %, à nouveau après être repassé au-dessus de 15 %), « SENS INTERDIT » ; en course
« TOUR 2 : 1:23.456 » (violet et « (MEILLEUR) » si c'est le meilleur), « ARRIVÉE : P2 »,
« DISQUALIFIÉ ». **Widget** « Tableau de bord » (220 × 68 : rapport, vitesse, régime) pour
n'importe quelle page. Case « Veille » de la fiche du module (onglet Modules) : pas de veille tant
que le tableau est affiché. Case « Chargement : charger uniquement quand affiché » (cochée par
défaut) : le module n'est lancé que quand sa page ou son widget s'affiche, gelé 3 s après qu'ils
ont quitté l'écran et arrêté 30 s après (0 % de processeur ; mesuré à environ 1 % d'un cœur sinon,
jeu lancé, page masquée) ; mais hors de l'écran : ni pilules ni relais ([modules.md](modules.md)).

## Réception : IP, ports, relais (`hsm01/udp.py`, les deux modules)
- **IP d'écoute** (`ecoute`, 0.0.0.0 par défaut : ce PC et le réseau ; 127.0.0.1 : ce PC seul ;
  ou une adresse de ce PC) et **port d'écoute** (`port`, UDP 1024 à 65535 comme dans les jeux :
  33330 Motor Town, 8888 SimHub). Une valeur fausse (« 300.1.1.1 », port 80…) : la valeur par
  défaut, et le problème écrit sur la page en attente (« Réglage : … ») et au journal ; port pris
  par un autre programme : « port indisponible ».
- La page en attente dit où le jeu doit envoyer : « adresse 127.0.0.1 (ce PC) ou 192.168.x.y
  (autre PC) » (l'adresse de ce PC sur le réseau, celle de la route par défaut) et le port.
- **Relais** (`[[relais]]` : `ip`, `port` ; 127.0.0.1:9999 tant que le formulaire n'est pas
  changé) : chaque datagramme reçu est renvoyé tel quel à ces adresses. Un jeu n'envoie qu'à une
  destination : ainsi le programme SimHub, un autre PC, un autre outil (ou l'autre module, s'il lit
  ce format) le reçoivent aussi. Un relais vers l'écoute du module lui-même (boucle) est refusé.
- Module SimHub, source « programme SimHub » : `ip_programme` (IP ou nom d'ordinateur, 127.0.0.1)
  et `port_programme` (TCP, 8888) ; une ancienne `adresse` en URL est encore lue.

## Motor Town
Télémétrie du jeu (`Documentation/Telemetry/MotorTownTelemetryProtocol.md` de son dossier,
protocole 1.2, révision 3) : UDP, un datagramme = un échantillon, 60 par seconde, rien en retour.
Dans le jeu : Options → Gameplay → « Enable Telemetry Output », format **Motor Town v1**, adresse
**127.0.0.1** (l'adresse du PC qui reçoit : 0.0.0.0 est une adresse d'écoute, pas de destination),
port **33330** ; s'applique sans relancer. Rien n'est envoyé à pied ni en passager. Le module
écoute sur `ecoute` (0.0.0.0 par défaut) et `port` (ancienne clé `adresse` encore lue).

`mt_telemetry.py` : `decode` (en-tête `MTTM`, version majeure 1, toute version mineure, longueur
= 8 + `payload_size` ; v1.0 153 octets, v1.1 849, v1.2 858 ; petit-boutiste sans bourrage) et
`sample`. Règles du document appliquées : un 0 peut vouloir dire « non disponible » (régime maximal
0 = pas de moteur : ni compte-tours ni rapport ; liquide 0 °C, limitation 0, capacité 0 : non
affichés) ; destination -1 = aucune ; `race_source` non nul mais rien qui court (tous les temps à
0, aucun point franchi : c'est un contre-la-montre **à proximité**, vu en jeu le 2026-11-20) = pas
de course ; drapeaux inconnus ignorés ; poids de l'attelage entier (v1.2) ; énergie en kWh pour
l'électrique. Le **format compatible Forza** (311 octets, et la variante Forza Horizon de 324)
est aussi lu, avec moins de valeurs (`IsRaceOn = 0` : pause ou pas de véhicule, rien d'affiché).
Le décodeur redonne exactement les fichiers d'exemple du jeu (`Examples/NativeV1*`, `V11*`,
`V12*` : 265 champs, 0 écart) ; essayé en jeu le 2026-11-20 (v1.2, 858 octets depuis 127.0.0.1).

## SimHub
Deux sources (`source` du formulaire) :

- **flux du jeu** (par défaut) : le module reçoit lui-même ce que le jeu enverrait à SimHub, sa
  sortie « compatible SimHub » / « Forza Data Out » en UDP, port **8888** (`port`, écoute sur
  `ecoute`, 0.0.0.0 par défaut). Pas besoin du programme SimHub. Format lu par `hsm01/forza.py`
  (commun avec Motor Town) : « Sled » 232 octets (régime, mouvement), « Dash » 311 (plus vitesse,
  puissance, couple, pneus en °F, turbo en psi, carburant 0..1, tours, position, pédales 0-255,
  rapport 0 = marche arrière, 11 = point mort), Forza Horizon 324 (les mêmes, 12 octets plus loin) ;
  `IsRaceOn = 0` (menu, pause, à pied) : rien d'affiché, « Jeu en pause, ou pas au volant ».
  Essayé avec Motor Town réglé en « compatible SimHub » vers le port 8888 (2026-11-20 : 311 octets
  depuis 127.0.0.1, ~53 par seconde). Pas de nom de véhicule dans ce format.
- **programme SimHub** : le programme SimHub (Windows, ou Wine / Proton) et son serveur web
  (Dash Studio, port 8888, `adresse`) : `GET /api/getproperty/<propriété>`, une valeur par
  requête, sur une connexion gardée ouverte (`simhub_api.py`) ; `frequence` fois par seconde (10)
  vitesse, régime, rapport et pédales (`DataCorePlugin.GameData.SpeedKmh`, `Rpms`, `Gear`,
  `Throttle`, `Brake`, `Clutch`), une fois sur cinq le reste (régime maximal, carburant,
  température d'eau, ABS, TC, limiteur des stands, voiture, tour, position, adversaires, temps,
  températures des pneus et des freins), avec `DataCorePlugin.GameRunning` et `CurrentGame`.
  Réponse lue sans supposer sa forme (JSON, nombre, True / False, durée `hh:mm:ss.fffffff`) ;
  propriété inconnue (404) laissée de côté, redemandée toutes les 60 s. Pas encore essayé avec le
  vrai programme (pas installé ici : seulement l'installateur dans les téléchargements). Son port
  TCP 8888 est aussi celui de la redirection du module Spotify pendant son autorisation (une fois).

## Essai
Faux jeux : `tools/fake_motortown.py --port N [--race S] [--forza]` (datagrammes v1.2 écrits champ
par champ d'après le document, course après S s ; `--forza` : flux compatible SimHub, 311 octets), `tools/fake_simhub.py [--no-game]` (API web).
Test : `TMPDIR=… cargo test -- --ignored telemetry_modules` (attente, écrans Conduite, Course venu
seul, Véhicule au clic, bouton → touches ; SimHub : flux compatible, puis programme : conduite, course, propriété inconnue ; aperçus
`hsm01-motortown.png`, `hsm01-motortown-course.png`, `hsm01-simhub.png`).
