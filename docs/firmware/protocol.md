# Protocole « libre » du firmware custom (v2 = palier 2)

USB `06f8:e053`, bcdDevice `0xF0xx`, interface 0 classe vendor (0xFF) :
| EP | Type | Sens | Rôle |
|----|------|------|------|
| 0x01 | bulk 64 o | hôte → device | flux de commandes d'affichage |
| 0x81 | interrupt 64 o, 2 ms | device → hôte | état des entrées (report 24 o) |
Requêtes de contrôle vendor (device) : `C0 02` → `[minor, major]` (240.xx) ; `40 F2 wValue=0 wIndex=E053` → DFU.

## Flux de commandes (EP 0x01)
Chaque message : en-tête 8 o puis `len` octets de charge utile ; un message peut chevaucher
plusieurs paquets USB. Octet de synchro incorrect ⇒ ignoré (resynchronisation).
```
0  0xA5        synchro
1  op
2  0x00 0x00   réservé
4  len u32 LE  longueur de la charge utile
```
| op | Nom | Charge utile |
|----|-----|--------------|
| 0x01 | FILL | `x y w h` (u16 LE) + `color` RGB565 u16 |
| 0x02 | BLIT | `x y w h` + `w·h` pixels RGB565 u16 LE, ligne par ligne |
| 0x03 | BLIT_RLE | `x y w h` + runs `[count u8 ≥ 1][color u16]` jusqu'à `w·h` pixels |
| 0x04 | COPY (240.21+) | `x y w h to_x to_y` (u16 LE) : recopie le rectangle pour que son coin haut-gauche aille en `(to_x, to_y)` ; recouvrement permis (défilement), écrêté à l'écran |
| 0x10 | BACKLIGHT | `pct u8` (0 = rétroéclairage coupé) |
| 0x11 | LEDS | `pct u8 × 4` (LED 1..4, PWM 240 kHz) |
| 0x7F | NOP | ignorée |
Coordonnées hors écran (480×272) écrêtées. Pas de double buffer (déchirure possible).

## Report d'entrées (EP 0x81, 24 o, toutes les 10 ms)
```
0      0x01 (id)
1      seq u8
2..3   boutons u16 : bits 0-3 clics molettes 1-4, bits 4-7 boutons 1-4, bit 8 power (1 = appuyé)
4..19  ADC brut 8 × u16 (12 bits) : M1A PC0, M1B PC1, M2A PC2, M2B PC3, M3A PA0, M3B PA1, M4A PA2, M4B PC4
20     rétroéclairage % demandé par l'hôte
21     époque de l'écran (240.22+) : +1 à chaque fois que le firmware reprend l'écran à l'hôte
       (économiseur) ; quand elle change, l'hôte redessine tout
22..23 réservé
```
Le décodage des molettes (potentiomètres sans fin double curseur) est fait côté hôte.

## Présence de l'hôte (240.22+)
L'hôte est présent tant qu'il envoie des données de dessin ou **lit les reports** (moins de 3 s
sans l'un ni l'autre = absent). Ses données de dessin lui donnent l'écran ; absent, le firmware
affiche un économiseur (badge « SIGNAL NOT FOUND » mobile, rétroéclairage 1 %) puis éteint l'écran
après 5 min ; bus USB suspendu (PC en veille) : rétroéclairage et LEDs à 0 jusqu'à la reprise.
Un outil qui veut garder une image fixe doit donc lire les reports (EP 0x81) jusqu'à la fin
([stage2.md](stage2.md)).
