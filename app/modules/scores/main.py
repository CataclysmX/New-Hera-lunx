#!/usr/bin/env python3
"""Live scores for new hera-nux: the matches under way in L'Équipe's live feed (lequipe.py),
grouped by sport and competition, on a page and in a widget; a pill when a score changes.

The feed is asked once a minute while a match of the user's filter (scores.toml: sports,
competitions, teams; « Live scores » tab of the window) is under way, otherwise every
5 minutes; a click of knob 1 on the page, or the « Refresh » command, asks at once.
Page: knob 1 scrolls. Widget « Live scores »: one line per match, its height follows.
"""
import argparse
import json
import os
import threading
import time
import tomllib

from hsm01 import Client
from hsm01 import widgets as W
from hsm01.i18n import tr

import lequipe

PAGE, LIST = 'scores', 'scores_liste'
HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'scores')
# names met in the feed (sports, competitions, teams): the lists of the settings form
SEEN = os.path.join(CACHE, 'seen.json')
URL = os.environ.get('HSM01_SCORES_URL') or lequipe.URL


def periods():
    """Seconds between two asks: a match of the filter under way / none (60, 300; the tests
    give theirs in HSM01_SCORES_PERIODS)."""
    try:
        fast, slow = (float(x) for x in os.environ['HSM01_SCORES_PERIODS'].split(','))
        return fast, slow
    except (KeyError, ValueError):
        return 60.0, 300.0


BG, TEXT, GREY, DIM, ACCENT, GREEN = '#0c0f12', '#e8eef0', '#8a9696', '#3a4448', '#00b4a8', '#3ddc84'
LIVE = '#f05a4a'
ROWS, ROW_H, TOP = 6, 34, 36  # page: lines shown, height of a line, top of the first
WIDE, LINE = 360, 24          # widget: width, height of a line


def log(text):
    print(text, flush=True)


def load_settings(path):
    """scores.toml: [[sport]], [[competition]], [[equipe]] (each `nom`), pilule_score."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        log(f'{path}: {e}')
        data = {}

    def names(key):
        return [str(x.get('nom', '')).strip() for x in data.get(key, [])
                if isinstance(x, dict) and str(x.get('nom', '')).strip()]
    return {'sports': names('sport'), 'competitions': names('competition'),
            'teams': names('equipe'), 'pill': bool(data.get('pilule_score', True))}


def lines(live):
    """What is shown, in the feed's order grouped by sport then competition: ('head', text) and
    ('event', event) lines."""
    groups = {}
    for ev in live:
        groups.setdefault(ev['sport'], {}).setdefault(ev['competition'], []).append(ev)
    out = []
    for sport, comps in groups.items():
        for comp, evs in comps.items():
            out.append(('head', ' · '.join(x for x in (sport.upper(), comp) if x)))
            out += [('event', ev) for ev in evs]
    return out


def score_text(ev):
    if ev['kind'] == 'race' or not ev['score']:
        return '—' if ev['kind'] == 'match' else ''
    return f'{ev["score"][0]} - {ev["score"][1]}'


class Scores:
    def __init__(self, client, args):
        self.c = client
        self.settings = load_settings(os.path.join(HERE, 'scores.toml'))
        self.fast, self.slow = periods()
        self.lock = threading.Lock()
        self.wake = threading.Event()
        self.live, self.lines, self.offset = [], [], 0
        self.scores = None  # id → score at the last ask (None: not asked yet)
        self.error, self.updated, self.next_at = '', '', ''
        self.sent, self.list_n, self.list_sent = {}, -1, {}
        self.seen = {'sports': set(), 'competitions': set(), 'teams': set()}
        try:
            with open(SEEN, encoding='utf-8') as f:
                for k, v in json.load(f).items():
                    if k in self.seen:
                        self.seen[k] = {str(x) for x in v}
        except (OSError, ValueError):
            pass
        client.page(PAGE, self.widgets(), name=tr('Live scores'), background=BG, show=args.show)
        self.offer_choices()
        with client.lock:
            client.on('turn', self.on_turn)
            client.on('click', self.on_click)
            client.command('refresh', tr('Refresh the live scores'),
                           description=tr('asks the feed at once'))(lambda ev: self.wake.set())
            self.refresh()
        threading.Thread(target=self.poll, name='scores', daemon=True).start()

    # --- the page -------------------------------------------------------------------------
    @staticmethod
    def widgets():
        out = [W.text('title', 12, 4, 300, 26, tr('Live scores'), size=15, align='left', color=GREY),
               W.text('count', 300, 4, 168, 26, '', size=13, align='right', color=LIVE)]
        for i in range(ROWS):
            y = TOP + i * ROW_H
            out += [W.text(f'h{i}', 12, y + 10, 456, 18, '', size=12, bold=True, align='left',
                           color=ACCENT),
                    W.text(f'a{i}', 8, y + 3, 170, 22, '', size=15, align='right', color=TEXT),
                    W.text(f's{i}', 182, y, 116, 24, '', size=18, bold=True, color=TEXT),
                    W.text(f'p{i}', 182, y + 21, 116, 13, '', size=10, color=LIVE),
                    W.text(f'b{i}', 302, y + 3, 170, 22, '', size=15, align='left', color=TEXT)]
        out += [W.text('msg1', 0, 96, 480, 30, '', size=20, bold=True, color=TEXT),
                W.text('msg2', 0, 134, 480, 22, '', size=14, color=GREY),
                W.text('msg3', 0, 162, 480, 22, '', size=13, color=GREY),
                W.text('foot', 8, 250, 464, 18, '', size=11, color=DIM)]
        return out

    def filter_text(self):
        s = self.settings
        parts = s['sports'] + s['competitions'] + s['teams']
        return ' · '.join(parts) if parts else tr('all sports')

    def refresh(self):
        """The page and the widget from the last ask (only what changed is sent)."""
        with self.lock:
            shown = self.lines[self.offset:self.offset + ROWS]
            want = {'count': {'text': tr('{n} live', n=len(self.live)) if self.live else ''}}
            for i in range(ROWS):
                kind, x = shown[i] if i < len(shown) else ('', None)
                ev = x if kind == 'event' else None
                want[f'h{i}'] = {'text': x if kind == 'head' else ''}
                want[f'a{i}'] = {'text': (ev['home'] if ev['kind'] == 'match' else ev['title']) if ev else ''}
                want[f's{i}'] = {'text': score_text(ev) if ev else ''}
                want[f'p{i}'] = {'text': ev['status'] if ev else ''}
                want[f'b{i}'] = {'text': ev['away'] if ev and ev['kind'] == 'match' else ''}
            empty = not self.live
            if self.scores is None and not self.error:
                msg = (tr('Asking L\'Équipe…'), '', '')
            elif empty:
                msg = (tr('No live match'), self.error or tr('filter: {filter}', filter=self.filter_text()),
                       tr('next check at {time}', time=self.next_at) if self.next_at else '')
            else:
                msg = ('', '', '')
            for k, t in zip(('msg1', 'msg2', 'msg3'), msg):
                want[k] = {'text': t}
            more = len(self.lines) > ROWS
            foot = [tr('knob 1: scroll') if more else '', tr('click: refresh'),
                    tr('updated {time}', time=self.updated) if self.updated else '']
            if self.error and not empty:
                foot.append(self.error)
            want['foot'] = {'text': ' · '.join(x for x in foot if x)}
            changes = {k: v for k, v in want.items() if self.sent.get(k) != v}
            self.sent.update(changes)
            rows = list(self.lines)
        self.c.update(PAGE, changes)
        self.refresh_list(rows)

    # --- the widget -----------------------------------------------------------------------
    @staticmethod
    def list_widgets(n):
        out = []
        for i in range(n):
            y = i * LINE
            out += [W.text(f'wh{i}', 6, y + 4, WIDE - 12, 16, '', size=11, bold=True, align='left',
                           color=ACCENT),
                    W.text(f'wa{i}', 0, y + 3, 126, 18, '', size=13, align='right', color=TEXT),
                    W.text(f'ws{i}', 128, y + 1, 64, 22, '', size=15, bold=True, color=TEXT),
                    W.text(f'wb{i}', 194, y + 3, 126, 18, '', size=13, align='left', color=TEXT),
                    W.text(f'wp{i}', 322, y + 5, 38, 14, '', size=10, align='left', color=LIVE)]
        return out

    def refresh_list(self, rows):
        """The « Live scores » widget: made again when its number of lines changes."""
        if not rows:
            rows = [('head', tr('No live match'))]
        if len(rows) != self.list_n:
            self.list_n, self.list_sent = len(rows), {}
            self.c.widget(LIST, WIDE, len(rows) * LINE, self.list_widgets(len(rows)),
                          name=tr('Live scores'), background=BG)
        changes = {}
        for i, (kind, x) in enumerate(rows):
            ev = x if kind == 'event' else None
            match = ev and ev['kind'] == 'match'
            for wid, fields in ((f'wh{i}', {'text': x if kind == 'head' else ''}),
                                (f'wa{i}', {'text': (ev['home_short'] if match else ev['title']) if ev else ''}),
                                (f'ws{i}', {'text': score_text(ev) if ev else ''}),
                                (f'wb{i}', {'text': ev['away_short'] if match else ''}),
                                (f'wp{i}', {'text': ev['status'] if ev else ''})):
                if self.list_sent.get(wid) != fields:
                    self.list_sent[wid] = fields
                    changes[wid] = fields
        self.c.update(LIST, changes)

    # --- inputs ---------------------------------------------------------------------------
    def on_turn(self, ev):
        if not (ev.get('mine') and ev.get('page') == PAGE and ev.get('knob') == 1):
            return
        with self.lock:
            top = max(0, len(self.lines) - ROWS)
            self.offset = max(0, min(top, self.offset + (1 if ev.get('delta', 0) > 0 else -1)))
        self.refresh()

    def on_click(self, ev):
        if ev.get('mine') and ev.get('page') == PAGE and ev.get('button') == 'knob1':
            self.wake.set()

    # --- the feed -------------------------------------------------------------------------
    def offer_choices(self):
        self.c.choices(**{k: sorted(v, key=str.casefold) for k, v in
                          (('sports', self.seen['sports']), ('competitions', self.seen['competitions']),
                           ('equipes', self.seen['teams']))})

    def remember(self, events):
        """Names of the feed (all its events, not only the live ones) for the settings form."""
        before = {k: len(v) for k, v in self.seen.items()}
        for e in events:
            self.seen['sports'].add(e['sport'])
            if e['competition']:
                self.seen['competitions'].add(e['competition'])
            for n in (e['home'], e['away']):
                if n:
                    self.seen['teams'].add(n)
        for v in self.seen.values():
            v.discard('')
        if {k: len(v) for k, v in self.seen.items()} != before:
            self.offer_choices()
            try:
                os.makedirs(CACHE, exist_ok=True)
                with open(SEEN, 'w', encoding='utf-8') as f:
                    json.dump({k: sorted(v) for k, v in self.seen.items()}, f, ensure_ascii=False)
            except OSError as e:
                log(f'{SEEN}: {e}')

    def poll(self):
        """Ask the feed, show it, wait: `fast` while a match of the filter is under way, `slow`
        otherwise (or until a refresh is asked)."""
        while True:
            self.wake.clear()
            s = self.settings
            try:
                events = lequipe.events(lequipe.fetch(URL))
                live = [e for e in events if e['live']
                        and lequipe.wanted(e, s['sports'], s['competitions'], s['teams'])]
                self.remember(events)
                error = ''
            except Exception as e:  # network, HTTP, JSON: shown, asked again later
                live, error = None, tr('L\'Équipe unreachable: {e}', e=e)
                log(error)
            wait = self.fast if (live if live is not None else self.live) else self.slow
            pills = []
            with self.lock:
                if live is not None:
                    pills = self.changed_scores(live)
                    self.live, self.lines = live, lines(live)
                    self.offset = min(self.offset, max(0, len(self.lines) - ROWS))
                    self.updated = time.strftime('%H:%M')
                self.error = error
                self.next_at = time.strftime('%H:%M', time.localtime(time.time() + wait))
            if live is not None:
                log(tr('{n} live match(es) of the filter, next check in {secs} s', n=len(live),
                       secs=f'{wait:.0f}'))
            for text in pills:
                self.c.pill(text, border=GREEN, duration=3, app=tr('Live scores'))
            self.refresh()
            self.wake.wait(wait)

    def changed_scores(self, live):
        """The pills of the scores that changed since the last ask (none at the first one)."""
        now = {e['id']: e['score'] for e in live if e['kind'] == 'match'}
        out = []
        if self.scores is not None and self.settings['pill']:
            for e in live:
                old = self.scores.get(e['id'])
                if e['kind'] == 'match' and old is not None and e['score'] and old != e['score']:
                    out.append(f'{e["home_short"].upper()} {e["score"][0]}-{e["score"][1]} '
                               f'{e["away_short"].upper()}')
        self.scores = now
        return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client('scores')
    Scores(client, args)
    client.run()


if __name__ == '__main__':
    main()
