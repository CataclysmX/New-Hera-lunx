# Module « Live scores » / « Scores en direct » (2026-11-21)

Dossier `app/modules/scores/` : `main.py` (page, widget, pilules, rythme de lecture),
`lequipe.py` (lecture et tri du flux, sans dépendance), `module.toml` (formulaire),
`scores.toml` (réglages), `i18n/fr.toml` (textes en français).

## Source
Flux des directs de L'Équipe : `https://dwh.lequipe.fr/api/live/lives` (JSON, ~210 Ko, gzip
accepté ; en-tête `User-Agent: hsm01-scores/1.0 (new hera-nux)`). Structure : `data[0].content.
feed.items` = listes imbriquées `live_listing_widget` (`title` : sport → compétition → phase /
journée, `items`), feuilles `sport_event_widget` avec `event` :
- `rencontre_sport_collectif` : match ; `specifics.domicile` / `exterieur` → `equipe` (`nom`,
  `short_name`, `url_image`), `specifics.score` (`domicile`, `exterieur`, absent avant le match) ;
- `course` : course (F1…), `titre` ;
- `allo` : programme télé, ignoré.

`statut` : `type` (`avenir`, `encours`…), `libelle` (« 17h15 », « 45' », « mi-temps », « 1e QT »),
`periode`. **En cours** = `type` « encours », ou le filtre du site `content_filters_matching`
« live-active » / « is_live ». Seuls les événements en cours sont affichés.

## Réglages (onglet « Live scores » / « Scores en direct »)
`scores.toml` : `[[sport]]`, `[[competition]]`, `[[equipe]]` (chacun `nom`), `pilule_score`.
Listes vides : tout. Sport : nom exact (majuscules et accents ignorés) ; compétition et équipe :
une partie du nom suffit (nom long ou court de l'équipe). Les listes du formulaire viennent du
module lancé : sports, compétitions et équipes **vus dans le flux**, matchs à venir compris,
gardés dans `~/.cache/hsm01/scores/seen.json` (les listes restent remplies sans match du jour).

## Rythme de lecture
Après chaque lecture : **60 s** si au moins un match du filtre est en cours, sinon **300 s**
(erreur réseau : le rythme de la dernière lecture réussie). Un clic de la molette 1 sur la page,
ou la commande « Refresh the live scores » (onglet Commandes), relit tout de suite. Le Journal
du module dit à chaque lecture combien de matchs du filtre sont en cours et quand vient la
suivante.

## Affichage
- **Page** : par sport puis compétition (dans l'ordre du flux), une ligne d'en-tête « SPORT ·
  Compétition » puis une ligne par match : domicile (aligné à droite), score, extérieur, le
  libellé du moment sous le score (« 45' ») ; courses : leur titre. 6 lignes à la fois, molette 1
  pour défiler. En haut à droite « N live » ; en bas l'heure de la dernière lecture. Sans match :
  « No live match », le filtre, l'heure de la prochaine lecture.
- **Widget « Live scores »** (onglet Widgets → « module » → Live scores → montrer : ce widget) :
  360 px de large, 24 px par ligne (en-têtes compris, noms courts), sa hauteur suit ; « trop
  grand : défiler ↕ » conseillé.
- **Pilule** (`pilule_score`, activée par défaut) : un score qui change entre deux lectures,
  « NICE 2-0 PARIS-SG » (pas à la première lecture).

## Test
`tools/fake_lequipe.py` : faux flux au même format (Nice - Paris-SG en cours 1-0, Lens - Lille à
venir, Atlanta - Charlotte à la mi-temps, un programme télé), `GET /control/goal` fait marquer
Nice. Test de bout en bout (rythmes ramenés à 1 s / 3 s par `HSM01_SCORES_PERIODS`, flux par
`HSM01_SCORES_URL`) : `TMPDIR=… cargo test --release -- --ignored scores_module` → page triée,
match à venir absent, widget 360×96, listes du formulaire, lectures rapprochées avec un match
du filtre et espacées sans, pilule du but, filtre équipe + sport ; `$TMPDIR/hsm01-scores.png`.
Vérifié contre le vrai flux le 2026-11-21 : 88 événements dont 4 matchs NBA en cours.
