# 10 – Déboguer un firmware sans sonde : les pannes rencontrées et la méthode

Pas de sonde de debug (SWD) : on observe par l'USB, l'écran, et en relisant les registres de la puce.
La méthode générale :

1. **Symptôme précis** (ce qu'on voit, quand, sur quelle version) ;
2. **Comparer avec le firmware officiel décompilé** (il marche sur ce matériel : c'est la référence) ;
3. **Mesurer** plutôt que supposer (débit, registres, captures) ;
4. **Isoler par A/B** : changer une seule chose, ou comparer avec le firmware officiel ;
5. **Garder une trace** (`docs/firmware/stage2.md` : versions 240.01 → 240.17 et ce que chacune a appris).

## Panne 1 – Le firmware gèle avant l'USB (240.02)

- Symptôme : écran de démarrage affiché, mais **rien sur l'USB** (`lsusb` vide).
- Cause : `Adc::new()` (embassy) lance la calibration de l'ADC, qui attend un bit qui ne bascule
  jamais car **l'ADC n'a pas d'horloge** (`RCC.CCIPR.ADCSEL` vaut « aucune » au reset).
- Comment : lecture du code source de la bibliothèque (`~/.cargo/registry/src/*/embassy-stm32-*/src/adc/v3.rs`).
- Correctif : `config.rcc.mux.adcsel = Adcsel::SYS`. Récupération : trappe molettes 1+4.

## Panne 2 – Débit USB 4× trop faible (240.02 → 240.04)

```sh
python3 -I tools/fw_bench.py 256
```

- Mesure clé : un message que le parseur **ignore** (NOP) va aussi lentement qu'une image → ce n'est
  pas le parseur, c'est l'USB. 250 Kio/s = exactement 4 paquets de 64 o par milliseconde.
- Cause (lue dans le code de `embassy-usb-synopsys-otg`) : l'endpoint n'accepte **qu'un** paquet puis
  refuse (NAK) jusqu'au prochain `read()` ; le PC ne réessaie que ~250 µs plus tard.
- Correctif : copie locale du crate (`firmware/rust/vendor/…`, `[patch.crates-io]` dans `Cargo.toml`)
  avec un **anneau de 16 paquets** armé pour plusieurs paquets d'un coup → 814 Kio/s, puis chemins
  rapides dans le parseur → **936 Kio/s** (le maximum pratique de l'USB Full Speed).
- Bonus de mesure : le débit dépend des données (le « bit stuffing » USB ralentit les pixels blancs
  et vert pur à 625 Kio/s) – vérifié en envoyant des images unies de couleurs différentes.

## Panne 3 – Image cisaillée (240.06)

- Symptôme (photo) : chaque ligne décalée de 2 px par rapport à la précédente, image qui reboucle.
- Cause : la bibliothèque écrit la longueur de ligne du LTDC `CFBLL = octets + 7` ; sur ce
  microcontrôleur il faut `+ 3`.
- Comment : `grep -n "+ 3U" re_work/decomp/fw_hsm01_v1_40.c` → `LTDC_SetConfig` officiel utilise `+3`.
- Correctif : réécrire `LTDC.L1CFBLR` après l'init (CFBP 960, CFBLL 963). Mire de contrôle :
  `python3 -I tools/fw_geometry.py`.

## Panne 4 – Firmware bloqué au démarrage, écran noir (240.05)

- Essai : utiliser un quartz 32 kHz (LSE) pour l'horloge USB « comme l'officiel ».
- Faux : l'officiel **n'allume pas** le LSE (structure `RCC_OscInit` : `OscillatorType = 0x10` = MSI
  seul). Le firmware attendait indéfiniment le quartz.
- Leçon : lire **les valeurs réelles** des structures HAL dans le firmware, pas le nom des fonctions
  appelées (`HAL_RCCEx_EnableMSIPLLMode` y est appelé… sans effet).
- Récupération : trappe molettes 1+4 (exécutée avant l'init des horloges).

## Panne 5 – Rémanence de la dalle et lignes dans les aplats (240.01 → 240.13)

C'était la plus trompeuse : l'image « marquait » la dalle en 2-3 minutes.

1. Hypothèse « dalle fragile » → **test A/B avec le firmware officiel** :
   ```sh
   python3 -I tools/mk_ab_images.py                      # damier + gris
   # firmware officiel restauré (fichier 07), puis :
   driver/rust/target/release/hsm01 background re_work/ab/checker.png 180
   driver/rust/target/release/hsm01 background re_work/ab/grey.png 120
   ```
   Officiel : **aucun fantôme, gris net** → c'est notre pilotage de l'écran qui est fautif.
2. Recherche des différences avec l'officiel : timings, polarités, broches (`MX_LTDC_Init`,
   `HAL_LTDC_MspInit`, `MX_TIM2_Init`…) → une broche différente (PB3), la vitesse des sorties…
   Indice décisif : avec les sorties en vitesse LOW (comme l'officiel), **plus d'image du tout**.
   Incohérent… sauf si notre horloge pixel est plus rapide que prévu.
3. **Mesurer** : ajout d'une requête USB de diagnostic (`0x03`) qui renvoie les registres de la puce :
   ```sh
   python3 -I tools/fw_regs.py
   # RCC_CCIPR2 0x00000000 → diviseur LTDC /2 → pixel clock 40 MHz, 252 Hz au lieu de 10 MHz / 63 Hz
   ```
4. Cause : le diviseur LTDC (`PLLSAI2DIVR`) n'est **modifiable que PLL arrêtée** ; notre écriture était
   ignorée. Correctif : arrêter PLLSAI2, écrire ÷8, la relancer. Vérification :
   `python3 -I tools/fw_regs.py` → `pixel clock 10.000 MHz … 63.1 Hz`. Test A/B refait : plus de fantôme.

Leçons : (a) quand un réglage « devrait » être bon, **relire le registre** ; (b) un test A/B contre le
firmware officiel tranche entre « matériel » et « logiciel » ; (c) se méfier des corrélations (la
vitesse des sorties semblait jouer, mais ce n'était qu'un effet de l'horloge 4× trop rapide).

## Panne 6 – Échec au branchement à froid (240.04, une seule fois)

- Symptôme : descripteur lu, puis erreurs `-71` et appareil muet. Journal :
  ```sh
  journalctl -k --since "-10min" | grep "usb 1-1"
  ```
- Non reproduit ensuite (branchements répétés OK en 240.13). À surveiller.

## Outils de diagnostic à garder sous la main

```sh
lsusb | grep -iE '06f8|0483'                       # qui est là : firmware (06f8) ou DFU (0483)
journalctl -k -f | grep "usb 1-1"                  # énumération en direct
python3 -I tools/hsm01_whoami.py                   # version
python3 -I tools/fw_regs.py                        # registres horloge / écran / GPIO
python3 -I tools/fw_bench.py                       # débit
python3 -I tools/re/fw_word.py <fw.bin> <adresse>  # constantes du firmware officiel
```
