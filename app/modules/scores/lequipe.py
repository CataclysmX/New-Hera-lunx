"""The live feed of L'Équipe (https://dwh.lequipe.fr/api/live/lives), standard library only.

The feed nests listings: sport → competition → phase…, each with `items`; a `sport_event_widget`
holds an `event`: a match (`rencontre_sport_collectif`: home and away teams, score), a race
(`course`: a title), or a TV programme (`allo`, left out). Its `statut` says whether it is under
way (`type` « encours », or the « live-active » filter of the site), with a label (« 45' »,
« mi-temps », « 1e QT »…).
"""
import gzip
import json
import unicodedata
import urllib.request

URL = 'https://dwh.lequipe.fr/api/live/lives'
USER_AGENT = 'hsm01-scores/1.0 (new hera-nux)'


def fetch(url=URL, timeout=20):
    """The feed's JSON (gzip accepted)."""
    req = urllib.request.Request(url, headers={'User-Agent': USER_AGENT, 'Accept': 'application/json',
                                               'Accept-Encoding': 'gzip'})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        raw = r.read()
        if r.headers.get('Content-Encoding') == 'gzip':
            raw = gzip.decompress(raw)
    return json.loads(raw)


def _text(x):
    if isinstance(x, dict):
        return str(x.get('text') or x.get('nom') or '')
    return str(x or '')


def _team(side):
    t = (side or {}).get('equipe') or {}
    return str(t.get('nom') or ''), str(t.get('short_name') or t.get('nom') or '')


def events(doc):
    """Every match and race of the feed, in its order: dicts with id, kind ('match' or 'race'),
    sport, competition, phase, home, away, home_short, away_short, score ((home, away) texts or
    None), title (races), status (its label), live (under way), date."""
    out = []

    def walk(items, trail):
        for w in items or []:
            if not isinstance(w, dict):
                continue
            kind = w.get('__type')
            if kind == 'live_listing_widget':
                walk(w.get('items'), trail + [_text(w.get('title'))])
            elif kind == 'sport_event_widget':
                e = w.get('event') or {}
                etype = e.get('__type')
                if etype not in ('rencontre_sport_collectif', 'course'):
                    continue
                st = e.get('statut') or {}
                filters = (w.get('content_filters_matching') or {}).get('filters') or []
                live = st.get('type') == 'encours' or any(
                    f.get('code') == 'live-active' and 'is_live' in (f.get('values') or [])
                    for f in filters if isinstance(f, dict))
                sp = e.get('specifics') or {}
                (home, home_s), (away, away_s) = _team(sp.get('domicile')), _team(sp.get('exterieur'))
                score = sp.get('score') or None
                sport = _text(e.get('sport')) or (trail[0] if trail else '')
                names = [t for t in trail if t]
                out.append({
                    'id': str(e.get('id') or ''),
                    'kind': 'match' if etype == 'rencontre_sport_collectif' else 'race',
                    'sport': sport,
                    'competition': names[1] if len(names) > 1 else '',
                    'phase': names[2] if len(names) > 2 else '',
                    'home': home, 'away': away, 'home_short': home_s, 'away_short': away_s,
                    'score': (str(score.get('domicile', '')), str(score.get('exterieur', '')))
                    if isinstance(score, dict) else None,
                    'title': str(e.get('titre') or ''),
                    'status': str(st.get('libelle') or ''),
                    'live': bool(live),
                    'date': str(e.get('date') or ''),
                })

    for page in (doc or {}).get('data') or []:
        feed = ((page or {}).get('content') or {}).get('feed') or {}
        walk(feed.get('items'), [])
    return out


def norm(s):
    """Lower case, without accents (« Équipe » and « equipe » match)."""
    return ''.join(c for c in unicodedata.normalize('NFKD', str(s).casefold())
                   if not unicodedata.combining(c))


def wanted(ev, sports=(), competitions=(), teams=()):
    """Whether an event passes the user's filter: one of `sports` (all if empty), a competition
    containing one of `competitions`, a team (name or short name) containing one of `teams`
    (races: only without a team filter)."""
    if sports and norm(ev['sport']) not in {norm(s) for s in sports}:
        return False
    if competitions and not any(norm(c) in norm(ev['competition']) for c in competitions):
        return False
    if teams:
        names = [norm(n) for n in (ev['home'], ev['away'], ev['home_short'], ev['away_short']) if n]
        return any(norm(t) in n for t in teams for n in names)
    return True
