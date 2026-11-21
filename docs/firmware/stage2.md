# Firmware custom – palier 2 (écran libre)

Sources : `firmware/rust/` (version **240.xx** : `FW_VERSION` de `main.rs`, bcdDevice 0xF0xx qui la suit en BCD, 240.22 → 0xF022 ; produit « Stream 100 (hsm01-fw) »).
Contrat USB : [protocol.md](protocol.md). Côté hôte : `hsm01-proto::free`, `hsm01-usb::free::FreeDevice`,
CLI `hsm01 fw …` ([../driver/rust.md](../driver/rust.md)).

| Outil | Rôle |
|-------|------|
| `tools/hsm01_whoami.py` | quel firmware tourne (officiel / custom + version) |
| `tools/fw_smoke.py` | scène de test (FILL, BLIT dégradé, BLIT_RLE rayures, rétro 90 %, LEDs), 3 reports, débit |
| `tools/fw_bench.py [KiB]` | débit par type de message : NOP (USB seul) vs BLIT (USB + parseur) |
| `firmware/proto-tests/` | `cargo test` hôte du parseur (`#[path]` vers `firmware/rust/src/proto.rs`) |

## Architecture (embassy, un seul exécuteur)
| Tâche | Rôle |
|-------|------|
| `usb_task` | pile embassy-usb (OTG FS, HSI48 + CRS) |
| `rx_task` | lit EP 0x01 par 512 o → `proto::Parser` → framebuffer / PWM |
| `input_task` | report 24 o toutes les 10 ms sur EP 0x81 : boutons + 8 curseurs ADC bruts + rétro % |
| `guard_task` | requête DFU USB ou clics molettes 1+4 ~1 s → mot magique → reset → bootloader ROM |

Framebuffer unique 480×272 RGB565 en SRAM (255 Ko), lu en continu par la LTDC : pas de double
buffer, un BLIT partiel peut donc se voir en cours d'écriture.

## Débit (bulk OUT, xHCI Intel, port direct)
| Version | NOP (USB seul) | BLIT brut | Plein écran brut |
|---------|---------------:|----------:|-----------------:|
| 240.02 – driver embassy d'origine | 250 Kio/s | 248 Kio/s | 1,0 i/s |
| 240.03 – + anneau OUT (driver patché) | 814 Kio/s | 503 Kio/s | 1,9 i/s |
| 240.04 – + chemins rapides parseur | **936 Kio/s** | **936 Kio/s** | **3,7 i/s** |

Plafond théorique USB FS bulk ≈ 1,1 Mo/s (≈ 4 i/s en brut) → au-delà : BLIT_RLE / zones partielles.

**Le débit dépend des données** (bourrage de bits USB, `colour_bench`, 2026-11-20) : pixels sans
suite de 6 « 1 » (0x0841, rouge 0xF800, bleu 0x001F, noir) → 930 Kio/s ; vert 0x07E0 et blanc 0xFFFF
→ **625 Kio/s** (≈ 2/3 : un paquet de moins par micro-trame côté xHCI). Le bench C alterne R/V/B/blanc
→ ≈ 780 Kio/s ; C et Python sont équivalents à données égales. Taille des blocs bulk sans effet.

1. **NAK à chaque paquet** : `embassy-usb-synopsys-otg` 0.3.3 arme l'EP OUT pour **un** paquet puis
   NAK jusqu'au prochain `read()` ; le xHCI ne réessaie qu'environ toutes les 250 µs → 4 paquets/ms.
   Patch local `firmware/rust/vendor/embassy-usb-synopsys-otg` (`[patch.crates-io]`, repères
   `hsm01 patch`) : EP bulk OUT en **anneau de 16 paquets**, armé pour N paquets (`PKTCNT = place
   libre`, hystérésis ½ anneau), réarmé sur `XFRC` (masque `DOEPMSK.XFRCM` ajouté) ou par `read()`,
   qui rend un flux d'octets (frontières de paquets non conservées). EP0 et autres types inchangés.
2. **Parseur octet par octet** : chemins rapides dans `proto::Parser::feed` – BLIT : copie directe de
   segments de ligne (`gfx::span`, memcpy : RGB565 LE = format natif) ; RLE : `gfx::hline` par
   segment ; Skip : saut en bloc. Équivalence prouvée par `firmware/proto-tests` (flux aléatoires,
   découpages aléatoires vs octet par octet).

## Pièges rencontrés
- **ADC sans horloge** : `RCC.CCIPR.ADCSEL` vaut « aucune » au reset et `Adc::new()` (embassy 0.6, L4)
  n'y touche pas → la calibration boucle sur `ADCAL`, le firmware gèle avant l'USB (écran de boot
  affiché, rien sur le bus). Corrigé : `config.rcc.mux.adcsel = Adcsel::SYS` (80 MHz ≤ max L4+).
- Échantillonnage ADC 247,5 cycles (≈ 3 µs) : curseurs de potentiomètres = sources haute impédance.
- `dfu-util …:leave` fonctionne (ré-énumération ≈ 1 s) ; le premier échec venait du gel ADC.

## Démarrage à froid (2026-11-20)
| Démarrage | Version | Résultat |
|-----------|---------|----------|
| à froid, rebranché rapidement | 240.02 | ✔ |
| `:leave` (bootloader ROM → appli) | 240.03, 240.04, 240.06 | ✔ |
| à froid après ≈ 15 min débranché | 240.04 | ✘ descripteur device lu, puis BOS et config en `-71`, puis plus aucune réponse (pile USB morte) ; récupéré par la trappe molettes 1+4 |
| 240.05 (LSE + PLLSAI1 comme « l'officiel ») | 240.05 | ✘ bloqué sur `LSERDY` avant l'écran : la carte n'a pas de LSE actif (cf. hardware.md) ; trappe ✔ |
Cause du ✘ 240.04 non établie (1 occurrence) : horloge HSI48 pas encore recalée par le CRS au premier
transfert long ? blocage de la pile après une requête abandonnée par l'hôte ? → tests à froid répétés.

## Image cisaillée : `CFBLL` (corrigé en 240.07)
Photo du logo en 240.06 : image **cisaillée** (décalage cumulatif par ligne, rebouclage en diagonale).
embassy-stm32 0.6 `init_layer` écrit `CFBLL = octets_ligne + 7` hors F7/U5 ; le LTDC du L4+ attend
**`+ 3`** (firmware officiel, `LTDC_SetConfig` : `bpp × largeur + 3 | pas << 16`). Avec `+ 7`, chaque
ligne lit 2 pixels de trop, qui débordent sur la suivante. Correctif : `LxCFBLR` réécrit après
`init_layer` (`CFBP = 960`, `CFBLL = 963`). Les « restes d'image » signalés plus tôt étaient très
probablement cet artefact (toutes les versions 240.0x l'avaient), pas une rémanence de la dalle.
Mire de contrôle : `python3 -I tools/fw_geometry.py` (bordure 1 px, lignes verticales tous les
60 px, carrés de couleur dans les coins).

## Rémanence + lignes : horloge pixel 4× trop rapide (corrigé en 240.13)
**Cause** : `pac::RCC.ccipr2().set_ltdcdiv(2)` (÷8) était **ignoré** – `PLLSAI2DIVR` n'est modifiable
que PLLSAI2 arrêtée, et embassy l'a déjà démarrée → resté à ÷2 : **horloge pixel 40 MHz, 252 Hz** au
lieu de 10 MHz / 63 Hz, dans **toutes** les versions 240.01–240.12. Mesuré par `tools/fw_regs.py`
(requête vendor 0x03 : RCC/LTDC/GPIO/TIM2). Correctif : arrêter PLLSAI2, écrire ÷8, la relancer
(séquence de `RCCEx_PLLSAI2_Config` côté officiel) ; broches LTDC en vitesse LOW comme l'officiel.
**Symptômes** (dalle sur-cadencée, pixels mal chargés) : lignes dans les aplats, **rémanence en
2-3 min** (logo, grille), anciens fantômes (logo Hercules, des mois d'usage) révélés ; en vitesse GPIO
LOW plus d'image du tout (240.10), en MEDIUM des lignes (240.11).
**Démarche** : rémanence confirmée par photo (240.07) → aucune polarisation de dalle côté MCU dans
l'officiel (TIM2 CH2/PB3 configuré mais jamais démarré, VCOM matériel, callback de ligne LTDC = simple
échange de framebuffer) → test A/B : avec la 1.40, damier 3 min puis gris **sans aucun fantôme** et
gris net → pilotage fautif → incohérence « LOW marche chez l'officiel, pas chez nous » → registres lus.
Restent en place (inoffensifs, plus fidèles) : PB3 = TIM2 CH2 (240.08), dalle en veille (PA8/PA5/PB10
à 0) avant le saut vers la ROM DFU (240.05). Fausses pistes : PB3, vitesse GPIO.

## Test A/B rémanence
`python3 -I tools/mk_ab_images.py` → `re_work/ab/checker.png` (damier 80×68, bords hors de la grille
`fw_geometry`) et `grey.png`. Firmware officiel (restaurer `restore_first_256k.bin`) :
`hsm01 background re_work/ab/checker.png 180` puis `hsm01 background re_work/ab/grey.png 120` (le
`background` envoie maintenant `ScreenBrightness` et lit les reports, sinon l'officiel éteint l'écran
/ abandonne l'hôte). Firmware custom : `python3 -I tools/fw_ab.py checker|grey` (garde l'image
jusqu'à Ctrl+C : depuis 240.22, sans hôte qui lit les reports, l'économiseur la remplace en 3 s).

## Logo de démarrage (240.05, délai 240.10, animation 240.14)
**Animation « tracé néon » à 60 fps**, entièrement **précalculée par `build.rs`** (`bootanim.bin`) :
traits = pixels dont le canal le plus fort ≥ 190 (`STROKE_LUM`) ; chaque composante 8-connexe est
révélée depuis son pixel le plus à gauche selon la distance géodésique (BFS) à 4 px/image
(`PEN_PX_PER_FRAME`), départs en balayage gauche→droite sur 60 images (`SWEEP_FRAMES`). Format :
`[u16 images]` puis par image `[u16 runs]` + runs `[offset u24][len u8]` (positions seules). Le
firmware (`boot_task`, `Ticker` 60 Hz) trace les pixels de l'image en **#006C68** (`gfx::TRACE`
0x036D), pointe du stylo #007C78 (`PEN_TIP` 0x03EF), puis **allumage**
en 45 étapes (`gfx::glow`) : traits vert → vraie couleur, halo noir → vraie couleur, puis logo exact.
Logo actuel (240.16) : 173 + 45 images = 3,6 s, 17 Ko. Toute donnée de l'hôte
interrompt l'animation. Aperçus : `python3 -I tools/re/anim_preview.py <dir> 25 55 85 glow15`.
Depuis 240.10 : sans données de dessin de l'hôte pendant **10 s** (`LOGO_SECS`), le logo est remplacé
par du noir uniforme et le rétro-éclairage passe à 10 % (rien d'imprimable) ; la première commande
de l'hôte reprend la main. Source par défaut : le plus récent des `public/bootlogo*.{png,jpeg,jpg}`.
`build.rs` convertit `$HSM01_BOOTLOGO` ou ce fichier (crate `image`, côté hôte) :
ajusté dans 480×272 sur fond noir (Lanczos3), RGB565, RLE `[count u8][color u16]` → `include_bytes!`,
décodé dans le framebuffer par `gfx::boot_screen` (sinon écran de test). Logo actuel
`public/bootlogo_resized.jpeg` 480×272 : 149 763 o de RLE, firmware ≈ 178 Ko (44 pages). Aperçu :
`python3 -I tools/re/rle565_preview.py target/…/out/bootlogo.rle apercu.png`.

## Flash / retour arrière
```sh
python3 -I tools/hsm01_dfu_enter.py --yes          # ou `hsm01 fw dfu`, ou clics molettes 1+4 au branchement
dfu-util -d 0483:df11 -a 0 -s 0x08000000 -D firmware/rust/hsm01-fw.bin
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave     # démarre le firmware
# retour au firmware officiel 1.40 (240.05 avec logo : 22 pages, 0x08000000–0x08015FFF ; le fichier
# 256 Kio couvre tout firmware custom < 256 Kio) :
dfu-util -d 0483:df11 -a 0 -s 0x08000000:leave -D backups/stream100-fw1.40-2026-11-19/restore_first_256k.bin
```
Build : `cd firmware/rust && cargo build --release && llvm-objcopy -O binary
target/thumbv7em-none-eabihf/release/hsm01-fw hsm01-fw.bin`. Vérifier que
`__cortex_m_rt_main` commence par la trappe (délai MOVW `0x9c41`, magique MOVW `0xb007` + MOVT
`0xdf00`) avant `__aeabi_memclr` et `embassy_stm32::init`, et que l'image n'atteint pas la dernière
page : `python3 -I tools/re/fw_hatch_check.py firmware/rust/target/thumbv7em-none-eabihf/release/hsm01-fw`
(tout « OK », code de sortie 0). Après flash : `python3 -I
tools/fw_regs.py` doit afficher **pixel clock 10.000 MHz, 63.1 Hz**.

## Entrées sous charge (240.18 → 240.19, 2026-11-20)
La tâche de réception USB traitait chaque lecture d'un trait ; tant que l'hôte envoie, `ep.read` est
prête tout de suite et ne rend jamais la main : la tâche d'entrées ne tournait plus (0 rapport/s).
240.18 : `yield_now` après chaque lecture + remplissage par mots de 32 bits (2 pixels). 240.19 : le
parseur reçoit des tranches de 32 octets et la tâche cède la main tous les 16 000 pixels dessinés.
Mesure `python3 -I tools/fw_load.py 4` (app arrêtée ; l'écran clignote pendant le test) :

| Charge | 240.17 | 240.18 | 240.19 |
|--------|--------|--------|--------|
| petits FILL (200 × 8×4) | 0 rapport/s | 93/s, trou max 14 ms | 88/s, 14 ms |
| colonnes (48 × 8×240) | 0/s, trou 4 s | 59/s, 34 ms | 69/s, 18 ms |
| plein écran en continu | 0/s, 6,3 Mpx/s | 3/s, 366 ms, 11,2 Mpx/s | 25/s, 52 ms, 11,2 Mpx/s |

BLIT brut inchangé : 932 Kio/s (`tools/fw_bench.py`).

## Entrées figées (240.20, 2026-11-20)
Symptôme (240.19, après des heures) : plus aucun rapport d'entrée (molettes, boutons), l'affichage
marchant toujours ; ni une réinitialisation USB (`tools/fw_usb_reset.py`) ni la relance de l'app
n'y faisaient, seul un redémarrage du microcontrôleur (DFU puis `dfu-util …:leave`, rien d'écrit).
Cause : `input_task` n'envoie un rapport qu'une fois le précédent lu par l'hôte (`write` attend que
EPENA retombe) ; un transfert IN jamais terminé (hôte qui n'interroge plus, ou désactivation du
pilote qui pose EPDIS sans attendre EPDISD et laisse EPENA) la bloquait pour toujours. Correctif :
`Endpoint<In>::abort` dans le pilote vendu (NAK, attente INEPNE, EPDIS, attente EPDISD, vidage FIFO,
attentes bornées) et `with_timeout(250 ms)` autour de chaque `write` : rapport en attente jeté,
endpoint propre. Aussi : l'animation de démarrage ne rendait jamais la main (un `Ticker` en retard
est prêt tout de suite) → 6 rapports/s et aucun dessin hôte pendant ~2 s ; `yield_now` à chaque
image. Mesures 240.20 : 101 rapports/s pendant l'animation, rapports repris après réinitialisation
USB, charge identique à 240.19. Contrôle rapide : `python3 -I tools/fw_probe.py` (version, 5 rapports,
un FILL, app arrêtée).

## Résultats (2026-11-20)
| Test | Résultat |
|------|----------|
| Build 240.01→02 (ADC sans horloge) | ✘ gel avant USB ; récupéré par la trappe molettes 1+4 ✔ |
| Énumération / version 240.04 / relecture flash | ✔ |
| FILL / BLIT / BLIT_RLE / rétro / LEDs acceptés | ✔ (38 Ko en 75 ms) ; ◐ rendu à confirmer visuellement |
| Reports d'entrée | ✔ seq, boutons, 8 curseurs stables (±4), rétro 80→90 % après commande |
| Débit | ✔ 936 Kio/s, 3,7 i/s plein écran brut |
| Décodage molettes (hôte) | ✔ 512 pas/tour, sens = officiel, cf. [knobs.md](knobs.md) |

| Portage C (`hsm01-c fw …`) | ✔ info / fill / image PPM / backlight / leds / monitor / bench, tests gcc+clang `-Werror` |
| 240.07 (CFBLL +3) | ✔ mire géométrique parfaite (photo), logo droit |
| 240.08–240.12 (PB3, logo 480×272 + délai 10 s, vitesse GPIO, diag 0x03) | ✘ rémanence/lignes (horloge 40 MHz) |
| 240.13 (PLLSAI2DIVR ÷8 → 10 MHz / 63 Hz, GPIO LOW) | ✔ logo net, gris net (comme l'officiel) ; ✔ damier 3 min → **aucun fantôme** ; ✔ branchements à froid répétés (logo + énumération à chaque fois) |
| VU-mètres côté hôte (`tools/fw_vu_demo.py`) | ✔ 60 fps tenus, ≈ 520 o/image (30 Kio/s) |
| 240.14 (animation de tracé du logo à 60 fps, 6 px/image) | ✔ rendu validé (« un poil rapide ») |
| 240.15 (stylo à 4 px/image : 173 + 30 images = 3,4 s) | ✔ flashée |
| 240.16 (tracé vert DEVUS #00674F + allumage 45 images, 3,6 s) | ✘ « trop vert fluo » sur la dalle |
| 240.17 (tracé #006C68 choisi sur la dalle, pointe #007C78) | ✔ validée (« pas trop mal ») |

**Couleurs sur cette dalle** : un RGB pensé pour l'impression (#00674F sur blanc) rend « vert fluo »
sur le TFT rétroéclairé à fond noir ; les teintes sombres virent au gris. Choisir sur l'appareil :
`python3 -I tools/fw_swatches.py [#rrggbb …]` (12 pastilles, colonne = nombre de points), rétro 80 %
comme au démarrage. Ici : vert ≈ bleu, luminosité ≈ 0x6C (#006C68).

## Défilement matériel (240.21, 2026-11-20)
Nouvelle commande **COPY** (op 0x04, [protocol.md](protocol.md)) : `x y w h to_x to_y`, l'appareil
recopie un rectangle de son image ailleurs (`gfx::copy` : écrêtage identique des deux côtés,
lignes dans le bon ordre quand les zones se recouvrent, `ptr::copy` = memmove dans une ligne ;
comptée dans le budget de travail avant de céder la main). Côté app (`screen.rs`, firmware ≥
240.21 seulement) : avant le diff par tuiles, recherche des contenus déplacés verticalement puis
horizontalement (empreintes de segments de 16 px par bande, votes de décalage, puis plus longue
suite de lignes **vérifiée au pixel près**, hors zones de spectre) ; COPY appliquée aussi à
l'image de référence, puis seules les tuiles encore différentes partent. Une fausse piste ne peut
pas donner une image fausse. Mesure (simulateur d'appareil, tests `screen::tests`) : zone de
300×190 décalée de 5 à 12 px → plus de 4× moins d'octets qu'en tuiles seules. Flashée (version
lue `15f0`), trappe DFU vérifiée (192 Kio). Rendu sur l'écran : à confirmer à l'œil (menu
d'exemple de l'API, textes qui défilent).

## Sans hôte : économiseur, puis écran éteint (240.22, 2026-11-20)
`screen_task` (remplace `boot_task`) décide de ce que montre l'écran :
- **hôte présent** = données de dessin reçues ou report d'entrées **lu par l'hôte** depuis moins de
  3 s (`HOST_GONE_MS`). Un `write` de report qui aboutit prouve que le précédent a été lu (le
  driver attend la fin du transfert), sauf juste après un `abort` (échec 250 ms) : drapeau `queued`.
  L'app lit les reports en continu, même en veille : pas de fausse absence. App fermée, tuée
  (`kill -9`), plantée ou figée → absente 3 s après.
- **sans hôte** : logo de démarrage (10 s) puis **économiseur** : badge « SIGNAL NOT FOUND » (police
  pixel 5×7 ×2 dans un cadre, halo ; généré par `build.rs`, alpha 4 bits 222×42) qui rebondit sur
  les bords à 30 px/s (30 i/s, 1 px par axe ; sa marge transparente efface la trace), couleur
  suivante à chaque rebond, **rétroéclairage 1 %**, LEDs éteintes ; rien d'immobile, donc rien à
  imprimer. Au bout de **5 min** (`IDLE_OFF_SECS`) : noir, rétroéclairage 0. Aperçu animé, rejoué
  sur l'ordinateur avec le badge de la compilation et la même logique que `Saver` :
  `python3 -I tools/saver_gif.py target/…/build/hsm01-fw-*/out/saver.bin docs/images/signal-not-found.gif`.
- **retour de l'hôte** : ses premières données de dessin reprennent l'écran aussitôt (`rx_task`
  remet son rétroéclairage et ses LEDs). L'économiseur ayant dessiné par-dessus, le firmware
  incrémente `SCREEN_EPOCH` (octet 21 du report) à chaque reprise : l'app redessine tout quand il
  change (cas d'une app figée plus de 3 s puis repartie).
- **PC en veille** = bus USB suspendu depuis 500 ms (`Handler::suspended`, filtre la suspension vue
  avant l'énumération) : rétroéclairage et LEDs à 0, plus rien n'est dessiné. À la reprise (ou
  un reset USB) : valeurs de l'hôte remises (son image est intacte), l'hôte a de nouveau 3 s pour
  se manifester ; sans hôte, nouveau tour d'économiseur de 5 min.
Rétroéclairage et LEDs sont écrits directement dans les registres de comparaison (`set_duty`,
même formule que `SimplePwm`, objets `forget` après la mise en route). Les outils qui affichent une
image fixe et quittent (`fw_ab.py`, `fw_geometry.py`, `fw_swatches.py`, `hsm01 fw fill|image|
backlight|leds`) lisent maintenant les reports jusqu'à Ctrl+C, sinon l'économiseur reprendrait
l'écran 3 s après. Flashée (version lue 240.22), trappe DFU vérifiée (198 Kio), horloge pixel
10 MHz / 63,1 Hz. Mesuré sur l'appareil : lecture des reports 5 s → `SCREEN_EPOCH` inchangé ;
4,5 s de silence → +1 ; flash pendant que l'app tourne → l'app se reconnecte seule ; écran
éteint après 5 min sans app (constaté). Pas encore vérifiés : l'aspect à 1 %, la veille du PC.

## Suite
- Laisser s'estomper les fantômes créés par les versions à 40 MHz (gris/images changeantes).
