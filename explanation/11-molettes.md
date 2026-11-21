# 11 – Comprendre les molettes à double curseur

Les molettes sont des **potentiomètres sans fin à deux curseurs**. Le firmware custom remonte les 8
valeurs brutes (12 bits) ; il faut les transformer en « pas » (crans) côté PC.

## 1. Enregistrer des données

```sh
python3 -I tools/fw_knobs.py 75 re_work/knobs/turns1.csv     # tourner chaque molette, une à la fois
```

Pour la mesure de référence : chaque molette, **exactement un tour** horaire (repère visuel), pause,
un tour anti-horaire.

## 2. Analyser

```sh
python3 -I tools/re/knob_analyze.py re_work/knobs/turns1.csv        # fenêtres d'activité, forme, comptes
python3 -I tools/re/knob_analyze.py re_work/knobs/turns2.csv 1      # + phase cumulée de la molette 1
```

Le script trace le nuage (A, B) en caractères et teste deux modèles :
- `|A−2048| + |B−2048|` constant → **losange** → deux triangles en quadrature ;
- `√(…)` constant → cercle → deux sinusoïdes.

Résultat : losange (2077 ± 30, contre ± 190 pour le cercle) → triangles décalés d'un quart de cycle,
**≈ 1 cycle par tour**. Position = place le long du losange, normalisée par `|x|+|y|` (insensible au
gain).

## 3. Reprendre l'algorithme officiel (unités et sens identiques)

Dans le firmware officiel, `FUN_0800e128` lit les curseurs et met à jour les compteurs ; deux seuils
sont des `double` chargés depuis la flash :

```sh
python3 -I tools/re/fw_word.py re_work/fw/hsm01_v1_42_body.bin --f64 0x0800e374 0x0800e37c   # 204.0 et 51.0
```

Algorithme : valeurs 8 bits filtrées (hystérésis ±2), on prend le curseur dans sa zone linéaire
(51…204, soit 20–80 %), sens selon un tableau à 4 cas → **512 pas par tour**. Rejoué en Python
(`knob_analyze.py`, classe `StockKnob`) sur les captures, il donne les mêmes comptes que notre méthode
du losange : on garde le losange (plus fin, 12 bits) en sortant **les mêmes unités et le même signe**
que l'officiel. Code : `hsm01_proto::free::knob` (Rust) et `hsm01_knob_*` (C).

```sh
driver/rust/target/release/hsm01 fw monitor 20     # « knob1 +12 (total +340) », ~512 par tour
```
