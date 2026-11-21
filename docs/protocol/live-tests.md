# Tests live (Linux, pyusb) – `tools/hsm01_live.py`

Device testé : s/n `6e c2 a7 07 07 9b 00 00`, **firmware 1.40** (`req 0x02` → `28 01`).
Accès : règle `packaging/udev/70-hercules-stream100.rules` (uaccess) installée.

## 2026-11-19
| Test | Résultat |
|------|----------|
| Init (set_config, claim 0/1, alt 1, req 0x52 ×2, 0x02) | ✔ |
| Keep-alive généré (`10…` + `11 00`) | ✔ accepté ; le device se met à émettre `sm` type 1 toutes les 16 ms |
| Ack `0x14` après type 1 | ✔ le type 1 cesse (confirme `0x11` = flag, `0x14` = ack) |
| Trame contenu `12…` + `31 01 00 64` + `30 i s` ×4 | ✔ `err=0`, `lastInv=0` ; octet d'état 0xa8→**0xe4** (écran on, 100 %) |
| Champ statut @26-27 (supposé usLedStates) | reste 0 après 0x30 (fw 1.40) – à revoir |
| Aspect visuel (écran / LEDs) | à confirmer par l'utilisateur |

Statut type 2 observé en live (octets depuis sm[7]) :
`00 00 | 23 00 | 00 00 | 06 00 | 02 6e c2 a7 07 07 9b 00 00 | 03 00 | 00 00 | 00 | 00 | 01 | 0b | ff`
= err, **dernier seq SM reçu**, ErrorCount, AvgSpeed, s/n(9), factoryCode=3, ledStates?, lastInvCmd,
renderMs, ?, splashMax, flashStatus(0xff = idle).

Encodeurs au repos : (23, 0, 0, 2) → valeurs absolues non nulles, seules les variations comptent.

Rejeu `tools/hsm01_replay.py re_work/captures/wireshark.tsv` : 600 trames de l'app officielle
(icônes 0x35, labels 0x36, VU 0x34/0x40/0x41, fond 0x38, réglages 0x32, LEDs 0x30) ré-encapsulées avec
notre framing → statut **err=0 / lastInv=0** sur toute la durée. Rendu visuel : à confirmer.

Rejeu et `hsm01 demo` (driver Rust) : **rendu validé visuellement par l'utilisateur** – icônes 32×32
(RLE alpha4/RGB565), labels 110×16, chenillard LEDs (0x30), VU animés (0x34/0x41), écran/luminosité.

Test contrôles (utilisateur : molettes 1→4, boutons 1→4, clics 1→4, power) : mapping complet confirmé,
voir [inputs.md](inputs.md). Remise à 0 des compteurs encodeurs observée au redémarrage de la session
(0,0,0,0 au départ). Power = cycle luminosité local 60→80→100→20→40.
