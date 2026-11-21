# 12 – Logo de démarrage, animation précalculée, choix des couleurs sur la dalle

## 1. Le logo

Déposer l'image dans `public/` sous un nom commençant par `bootlogo` (`.png`, `.jpg`, `.jpeg`) ; le
plus récent est pris (ou forcer avec `HSM01_BOOTLOGO=/chemin/image.png`). Idéalement en 480×272.

À la compilation, `firmware/rust/build.rs` (bibliothèque `image`) :
1. ajuste l'image dans 480×272 sur fond noir (Lanczos3) et la convertit en RGB565 ;
2. la compresse en RLE (`[nombre u8][couleur u16]`) → `bootlogo.rle`, inclus dans le firmware ;
3. calcule l'animation (ci-dessous) → `bootanim.bin`.

```sh
cd firmware/rust && cargo build --release && cd ../..
# aperçu exact de ce que le firmware affichera :
f=$(ls -t firmware/rust/target/thumbv7em-none-eabihf/release/build/hsm01-fw-*/out/bootlogo.rle | head -1)
python3 -I tools/re/rle565_preview.py $f /tmp/apercu_logo.png
```

## 2. L'animation « tracé néon », entièrement précalculée

Tout le calcul est fait **sur le PC à la compilation** ; le firmware ne fait que recopier des pixels.

1. **Traits** = pixels dont le canal le plus fort ≥ 190 (`STROKE_LUM`). Pour régler ce seuil :
   ```sh
   python3 -I tools/re/logo_anim_probe.py $f /tmp/anim     # histogramme + masques core_160/190/220.png
   ```
2. Chaque trait (zone connexe) est révélé **le long de sa forme** : distance géodésique (parcours en
   largeur) depuis son pixel le plus à gauche, `PEN_PX_PER_FRAME` = 4 px par image ; les traits
   démarrent en balayage gauche → droite sur `SWEEP_FRAMES` = 60 images.
3. Le fichier liste, image par image, les positions des pixels à allumer (17 Ko pour ce logo).
4. Le firmware (`boot_task`, cadence 60 Hz) trace ces pixels en `TRACE` (pointe du stylo en
   `PEN_TIP`), puis l'**allumage** : 45 étapes où les traits passent de la couleur de tracé à leur
   vraie couleur et où le halo s'allume ; enfin le logo exact, puis noir après 10 s sans logiciel.

```sh
python3 -I tools/re/anim_preview.py /tmp/anim 25 55 85 110 glow15 glow30   # images PNG de contrôle
```

Réglages : `PEN_PX_PER_FRAME`, `SWEEP_FRAMES`, `STROKE_LUM` dans `build.rs` ; `TRACE`, `PEN_TIP` dans
`src/gfx.rs` ; `GLOW_STEPS` dans `src/main.rs` (garder `tools/re/anim_preview.py` synchronisé).

## 3. Choisir une couleur **sur la dalle**

Une couleur pensée pour l'impression (logo sur fond blanc) ne rend pas pareil sur un écran
rétroéclairé à fond noir : le vert du logo (#00674F) sortait « fluo », les teintes sombres virent au
gris. Méthode : afficher des pastilles sur l'appareil et choisir à l'œil, en resserrant à chaque tour.

```sh
python3 -I tools/re/png_main_color.py logo.png          # couleur dominante d'un PNG (point de départ)
python3 -I tools/fw_swatches.py                          # 12 pastilles par défaut
python3 -I tools/fw_swatches.py 006c68 007c78 008c88 009c98 00aca8 00bcb8 \
                                006c70 007c80 008c90 009ca0 00acb0 00bcc0   # échelle de luminosité
```

Pastilles en 2 rangées × 6 ; le nombre de points blancs sous chaque pastille = n° de colonne ;
rétroéclairage 80 % comme au démarrage. Démarche suivie : trop vert → plus bleu → trop bleu → entre les
deux → trop gris (trop sombre) → échelle de luminosité → **#006C68** retenu. Rappel : l'écran est en
RGB565 (32 niveaux de rouge/bleu, 64 de vert), des écarts plus fins n'existent pas.

## 4. Après modification

Recompiler, vérifier la trappe, flasher (fichier 09 §2, fichier 07 §5), puis débrancher/rebrancher
pour revoir l'animation.
