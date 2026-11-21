# Commandes SM (hôte → device)

Sources : dispatcher firmware `FUN_080099d0` (fw v1.42, **référence**) + encodeurs DLL
(`FUN_180004ce0`, `FUN_180004540`). Longueurs validées sur 3305 trames capturées
(`tools/re/hsm01_proto.py decode` : 0 erreur, fin exacte). ✔ = observé en capture.
Opcode inconnu ⇒ firmware arrête le parse, `Error |= 0x40`, `ucLastInvCmd = op` (statut).

| Op | Long. | Format | Effet firmware / API source |
|----|-------|--------|-----------------------------|
| 0x00 | 1 | – | fin de liste (sinon arrêt à la longueur SM) ✔ |
| 0x10 | 24 | `10 01 [os 4×u16][sdkMin sdkMaj][12×00]` | HOST_INFO, 23 o stockés ✔ |
| 0x11 | 2 | `11 v` | flag "hôte présent" (=1) + v ; keep-alive ✔ |
| 0x12 | 5 | `12 00 00 00 00` | **écran ON** (rétroéclairage = luminosité mémorisée) ✔ |
| 0x13 | 5 | `13 00 00 00 00` | **écran OFF** (veille, rétro 60) |
| 0x14 | 1 | – | efface le flag 0x11 (ack des infos device type 1) ✔ |
| 0x15 / 0x16 | 1 | – | no-op |
| 0x17 | 1 | – | reset d'un état d'affichage (envoyé si rafraîchissement forcé) |
| 0x20 | 2 | `20 v` | usine : mode factory (v=1) |
| 0x21 | 4+n | `21 09 [sum16] 02 sn[8]` | usine : écrit n° série (si factory) |
| 0x22 | 3 | `22 [u16]` | usine : code factory (si factory) |
| 0x30 | 3 | `30 led state` led<4, state 0 off / 1 on / 2 clignote | `HSM_SetLedStyle` ✔ |
| 0x31 | 3/4 | `31 flags 00 [pct si flags&1]` pct ≤ 100 | **luminosité écran** (`HSM_SetDisplaySettings`, b×100) ✔ |
| 0x39 | 3/4 | `39 flags 00 [v si flags&1]` v ≤ 100 | **intensité LEDs** (`HSM_SetLedIntensity` : 50 / 90) |
| 0x32 | 26/18 | `32 00` + 4 canaux × 6 o (si octet1≠0 : 16 o ignorés) | réglages d'affichage par canal (bit0 o2/o3 → flags) ✔ |
| 0x33 | 7+2n | `33 sel ? off [n u16] 00 [n × RGB565]` | **palette** du fond custom (entrées off..off+n) |
| 0x34 | 9+2n | `34 [ch \| sub<<7] [c0][c1][c2] n [n × u16]` RGB565 | **style VU-mètre** (8 = 4 canaux × 2) ✔ |
| 0x35 | 5+n | `35 bank elt [n u16] RLE` | **icône 32×32** du canal `elt` (0..3), banque 0/1 ✔ |
| 0x36 | 5+n | `36 bank elt [n u16] RLE` | **label 110×16** du canal `elt`, banque 0/1 ✔ |
| 0x37 | 9+4080 | `37 01 [u32 tile&0x1f] 00 [n u16] 4080 o` | **tuile de fond** 8 bits indexés (68×60 o, étirés selon table) |
| 0x38 | 16 | `38 screenType bkgType [darkRGB u32][lightRGB u32] azStyle [azRGB u32]` | **config écran/fond** (`HSM_SetScreenConfiguration`) ✔ |
| 0x40 | 6/10 | `40 [ch \| wide<<6 \| sub<<7] v0 v1 v2 v3` (u8×0x101 ou u16) | géométrie/échelle des VU (positions barres) ✔ |
| 0x41 | 6 | `41 [ch \| clip<<5 \| clip<<6 \| sub<<7] [lvl u16][peak u16]` | **niveau VU** : y = (0xffff-lvl)·119/0xffff + 80 ; clip ⇒ rouge ✔ |
| 0x50 | 2 | `50 80` / `50 40` | flash splash : image custom / restaure défaut (`HSM_FlashWrite_*`) |

Notes
- VU inactif : `41 ch c000 c000`. La DLL n'émet 0x30/0x34/0x40/0x41 que sur changement.
- 0x38 `azStyle` 0..5 transmis tel quel (sinon 0). Capturé : `38 00 04 ffffffff ffffffff 04 ffffffff`. `bkgType = 0xFF` ⇒ fond custom (palette 0x33 + tuiles 0x37).
  Struct API (cb=0x28) : screenType, bkgType, hsmBkgId, bgDarkestRGB, bgLightestRGB, actionZoneStyle,
  actionZoneRGB, pageId, modeId.
- 0x32 capturé : `00` + 4 × `81 01 07 07 00 64` ; sémantique fine encore TBD (`FUN_180003a20`).
- Ordre DLL : [0x17] [0x12|0x13] [0x14] 0x30* 0x31 0x39 [0x20 0x22 0x21] 0x32 0x38, par VU (×8)
  0x34 0x40 0x41, [0x50 40], [0x33 0x37 | 0x50 80], éléments 0x35/0x36, 0x00.
- Keep-alive DLL (500 ms) : `10 …` + `11 00`, sans 0x00 final.

## Fond custom & logo de démarrage (0x33 / 0x37 / 0x50)
- Image 480×272 en **8 bits indexés** + palette **RGB565** (≤ 256), quantifiée par l'hôte.
- `33 00 00 <offset> [n u16] 00 [n × RGB565]` : entrées de palette offset..offset+n.
- `37 01 [tile u32] 00 [0x0ff0 u16] <4080 o>` : tuile `t` (0..31) = échantillons
  `(8·col + dx, 4·row + dy)` pour 68 lignes × 60 colonnes ; le fw remplit un bloc `w×h`
  (entrelacement progressif, table `(dx,dy,w,h)` dans `hsm01-proto/src/background.rs::TILES`).
  Le fw tient un masque des tuiles reçues (remis à 0 par la tuile 0).
- Affichage : `0x38` avec `bkgType = 0xFF`. La DLL envoie palette + tuile 0 dans la même trame,
  puis une tuile une trame sur deux.
- `50 80` : écrit le fond courant en flash comme **logo de boot** (refusé, erreur statut 0x100, si les
  32 tuiles ne sont pas toutes reçues) ; `50 40` : restaure le logo d'usine. Progression dans l'octet
  `FlashProgrammingStatus` du statut : 0..100 %, 100 = terminé, 0xFE = erreur, 0xFF = inactif.
- CLI Rust : `hsm01 background <img>` (aperçu RAM), `hsm01 splash <img|default> --yes`.

## RLE (0x35/0x36 ; encodeur DLL `FUN_180004900`, décodeur fw)
Par run : `octet = (len << 4) | alpha4` puis, si `alpha4 != 0`, **RGB565 LE** (2 o).
`len` 1..15 ; `len = 0` ⇒ jusqu'à la fin de la ligne (32 px icône / 110 px label) – utilisé dès
qu'un run atteint la fin de ligne, même > 15. Runs comparés sur (alpha4, couleur).
`alpha4` = A >> 4 (0 = transparent, 15 = opaque) ; couleur **pré-multipliée** par l'alpha
(`FUN_1800046a0` : bits 3..0 de alpha4 → ×(1/2 + 1/4 + 1/8 + 1/16)) ; cas spécial A ∈ [0x80,0x84] → 7.
Filtres DLL `FUN_180004790` : ×0.6 (grisé), gris 0.3R+0.59G+0.11B.
Banque (octet 1) : la DLL envoie les 4 combinaisons {icône,label}×{banque 0,1} par canal ;
fw : banque 0 → buffer A (flag 0x01/0x02), banque 1 → buffer B (flag 0x10/0x20).
