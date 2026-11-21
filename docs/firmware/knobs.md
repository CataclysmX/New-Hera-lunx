# Molettes : potentiomètres sans fin à double curseur

Matériel : 2 curseurs par molette sur ADC1 (brochage : [hardware.md](hardware.md)), relevés bruts
12 bits par le firmware custom (report EP 0x81, [protocol.md](protocol.md)).

## Signal (captures `re_work/knobs/turns1.csv`, `turns2.csv`, 2026-11-20)
- Deux **triangles en quadrature** (décalage ¼ de cycle), pleine échelle 0…4095 :
  le point (A, B) décrit un **losange** `|A−2048| + |B−2048| ≈ 2077 ± 30` (un cercle colle mal : ± 190).
- **≈ 1 cycle électrique par tour** mécanique ; bruit au repos ± 7 LSB.
- **Horaire = phase croissante = « + »** du firmware officiel (molette 1, aller-retour contrôlé).

## Algorithme du firmware officiel 1.42 (`FUN_0800e128`, appelé par la tâche entrées)
1. Valeurs **8 bits** filtrées par hystérésis ± 2 (`FUN_08000b2c`) ; état précédent par curseur.
2. Rien si A **ou** B n'a pas bougé.
3. Sens (vrai → décrément) selon (sens A, sens B) :
   A↓B↓ : `B < A` · A↑B↑ : `A < B` · A↑B↓ : `A > 0x7F ‖ B > 0x7F` · A↓B↑ : `A < 0x7F ‖ B < 0x7F`.
4. Amplitude : `|ΔA|` si `51 ≤ A ≤ 204` (zone linéaire 20–80 %, constantes `double` 0x0800e37c /
   0x0800e374), sinon `|ΔB|`.
5. Accumulateur i16 par molette (report officiel, octets 3-10) : **512 pas par cycle**.

## Décodeur hôte (`hsm01_proto::free::knob`)
Phase = position le long du losange, normalisée par `|x|+|y|` (insensible au gain), 16 bits par
cycle ; sortie en pas officiels (512/cycle, même signe), hystérésis d'un pas (bruit ≈ ± 0,4 pas).
Rejoué sur les captures (`tools/re/knob_analyze.py <csv> [molette]`), il donne les mêmes comptes que
l'algorithme officiel à ± 1 % sur chaque mouvement. Tests : cycle synthétique = ± 512, bruit = 0.
`hsm01 fw monitor` affiche `knobN ±pas (total …)` ; brut : `tools/fw_knobs.py [s] [csv]`.
