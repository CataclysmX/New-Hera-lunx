#!/usr/bin/env python3
"""Module « Spotify » de new hera-nux: the Hercules Spotify options through Spotify's Web API and
YOUR Spotify Developer app (Client ID and secret: « Données sensibles » of the module's tab; the
browser asks once, the token is kept in the module's .env).

Page « Spotify »: cover, title, artist, album, progress; knob 1: position (5 s a step, click: play
/ pause), knob 2 click: shuffle, knob 3 click: repeat (off, all, track), knob 4: volume (click:
mute); buttons (spotify.toml [boutons]; by default previous, play / pause, next - held: the
track's radio -, like) one of: those, the track's radio, shuffle, repeat, mute. Actions Spotify
forbids where the music comes from (actions.disallows: shuffle and « repeat all » without a
playlist or album…) are skipped with a pill. Commands (tab Commandes, any page): the same, and per
favourite playlist (spotify.toml): start it, add / remove the track playing.
Controlling playback needs Spotify Premium (like and playlists work without). Option: --show.
"""
import argparse
import hashlib
import os
import queue
import secrets as tokens
import shlex
import subprocess
import threading
import time
import tomllib
import urllib.request

from hsm01 import Client, secrets
from hsm01 import widgets as W
from hsm01.i18n import tr

import spotify_api as api

PAGE = 'spotify'
HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'spotify')
BG, TEXT, GREY, GREEN, RED, TRACK = '#0b0f0c', '#e8eeea', '#8a968e', '#1db954', '#f04848', '#1c2620'
KEY, KEY_DOWN, KEY_ON = '#131a15', '#2a3a2e', '#174d2a'
BAR_X, BAR_W = 180, 288
POLL = 2.0
TOKENS = ('SPOTIFY_ACCESS_TOKEN', 'SPOTIFY_ACCESS_EXPIRES', 'SPOTIFY_REFRESH_TOKEN')
REPEAT = ['off', 'context', 'track']
REPEAT_TEXT = {'off': tr('off'), 'context': tr('all'), 'track': tr('track')}
# what the page's buttons 1-4 can do (spotify.toml [boutons] b1..b4, the form's choices)
KEYS = ['précédent', 'lecture / pause', 'suivant', 'j\'aime', 'radio du titre', 'aléatoire',
        'répétition', 'sourdine', 'rien']
DEFAULT_KEYS = ['précédent', 'lecture / pause', 'suivant', 'j\'aime']
# the knobs' clicks (spotify.toml [clics] k1..k4): the same choices
DEFAULT_CLICKS = ['lecture / pause', 'aléatoire', 'répétition', 'sourdine']


def log(text):
    """A line in the Journal and in ~/.cache/hsm01/spotify/journal.log (never a token)."""
    print(f'Spotify: {text}', flush=True)
    try:
        os.makedirs(CACHE, exist_ok=True)
        with open(os.path.join(CACHE, 'journal.log'), 'a', encoding='utf-8') as f:
            f.write(f'{time.strftime("%Y-%m-%d %H:%M:%S")} {text}\n')
    except OSError:
        pass


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def clock(ms):
    s = max(0, int(ms // 1000))
    return f'{s // 60}:{s % 60:02d}'


def load_settings(path):
    """spotify.toml: volume step, favourite playlists (by name), redirect address."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        log(f'{path}: {e}')
        data = {}
    names = [str(p['nom']).strip() for p in data.get('playlist', []) if p.get('nom')]

    def chosen(table, prefix, defaults):  # a wrong value: the default
        t = data.get(table) if isinstance(data.get(table), dict) else {}
        got = [str(t.get(f'{prefix}{i + 1}') or d) for i, d in enumerate(defaults)]
        return [k if k in KEYS else d for k, d in zip(got, defaults)]
    return {'step': clamp(int(data.get('pas', 5)), 1, 20), 'playlists': names,
            'keys': chosen('boutons', 'b', DEFAULT_KEYS),
            'clicks': chosen('clics', 'k', DEFAULT_CLICKS),
            'redirect': str(data.get('redirection') or 'http://127.0.0.1:8888/callback')}


def says(e):
    """Spotify's refusal in words for the screen."""
    if e.reason == 'PREMIUM_REQUIRED' or e.status == 403 and 'remium' in str(e):
        return tr('Spotify Premium is needed to control playback')
    if e.reason == 'NO_ACTIVE_DEVICE' or e.status == 404:
        return tr('no active Spotify device: start Spotify')
    if e.status == 429:
        return tr('too many requests: new try in {secs} s', secs=e.retry)
    if 'Restriction violated' in str(e):
        return tr('Spotify refuses this action here')
    return str(e)


class Spotify:
    def __init__(self, client, args):
        self.c, self.args = client, args
        self.settings = load_settings(os.path.join(HERE, 'spotify.toml'))
        self.client_id = (secrets.get('SPOTIFY_CLIENT_ID') or '').strip()
        self.secret = (secrets.get('SPOTIFY_CLIENT_SECRET') or '').strip()
        self.status, self.message = 'start', ''
        self.player, self.read_at = None, time.monotonic()
        self.liked, self.liked_id = None, None
        self.playlists = {}  # name -> (id, uri)
        self.volume_target, self.unmuted = None, None
        self.play_asked = None  # (playing, until): the last play / pause asked
        self.sent, self.states, self.art, self.leds_sent, self.down = {}, {}, {}, None, set()
        self.actions, self.wake = queue.Queue(), threading.Event()
        client.page(PAGE, self.widgets(), name='Spotify', background=BG, show=args.show)
        with client.lock:
            client.on('turn', self.on_turn)
            client.on('click', self.on_click)
            client.on('hold', self.on_hold)
            client.on('press', lambda ev: self.key(ev, True))
            client.on('release', lambda ev: self.key(ev, False))
            self.offer_commands()
            self.refresh()
        threading.Thread(target=self.run_loop, name='spotify', daemon=True).start()

    # --- the page --------------------------------------------------------------------------
    @staticmethod
    def widgets():
        out = [W.icon('logo', 8, 6, 22, 22, 'spotify-client'),
               W.text('player', 36, 4, 436, 26, 'Spotify', size=14, align='left', color=GREY),
               W.rect('cover_bg', 12, 36, 156, 156, '#151c17', radius=10),
               W.icon('cover', 15, 39, 150, 150, 'spotify-client'),
               W.text('title', BAR_X, 40, BAR_W, 30, '', size=20, bold=True, align='left', color=TEXT),
               W.text('artist', BAR_X, 74, BAR_W, 24, '', size=15, align='left', color='#c8d2cc'),
               W.text('album', BAR_X, 100, BAR_W, 22, '', size=13, align='left', color=GREY),
               W.rect('bar_bg', BAR_X, 142, BAR_W, 6, TRACK, radius=3),
               W.rect('bar', BAR_X, 142, 1, 6, GREEN, radius=3),
               W.text('time', BAR_X, 152, BAR_W, 20, '', size=12, align='left', color=GREY),
               W.text('msg1', 16, 84, 448, 30, '', size=17, color=TEXT),
               W.text('msg2', 16, 120, 448, 24, '', size=13, color=GREY)]
        edge = W.frame(border='#26332a', width=1.0, radius=10, padding=0)
        for k in range(4):
            cx = 60 + k * 120
            out += [W.text(f'k{k + 1}', k * 120, 199, 120, 16, '', size=12, color=GREY),
                    W.rect(f'b{k + 1}', cx - 44, 222, 88, 46, KEY, radius=10, frame=edge),
                    W.icon(f'bi{k + 1}', cx - 12, 225, 24, 24, '', tint=TEXT),
                    W.text(f'bt{k + 1}', cx - 44, 251, 88, 14, '', size=11, color=GREY)]
        return out

    def header(self):
        """(top line, message, its second line) while not ready."""
        return {
            'config': (tr('Spotify · to set up'), tr('Client ID and secret of your Spotify application'),
                       tr('window, "Spotify" tab (guide at the top of the tab)')),
            'auth': ('Spotify', tr('Allow in the browser'), tr('the Spotify page that has just opened')),
            'error': ('Spotify', self.message, tr('click: try again (details: Log)')),
        }.get(self.status, ('Spotify', tr('Connecting to Spotify…'), ''))

    def item(self):
        return (self.player or {}).get('item') or {}

    def refresh(self):
        """Send what changed on the page, the LEDs and the commands' states."""
        ready, p, it = self.status == 'ready', self.player or {}, self.item()
        want = {}
        if ready:
            dev = (p.get('device') or {}).get('name', '')
            state = tr('playing') if self.playing() else tr('paused') if it else tr('nothing playing')
            want['player'] = {'text': ' · '.join(x for x in ('Spotify', state, dev) if x)}
            want['msg1'], want['msg2'] = {'text': ''}, {'text': ''}
            # a track's artists, or a podcast episode's show
            names = [a.get('name', '') for a in it.get('artists') or []]
            names = names or [(it.get('show') or {}).get('name', '')]
            want['title'] = {'text': it.get('name') or '—'}
            want['artist'] = {'text': ', '.join(n for n in names if n)}
            want['album'] = {'text': (it.get('album') or {}).get('name', '')}
            want['cover'] = {'name': self.cover(it)}
        else:
            top, m1, m2 = self.header()
            want['player'], want['msg1'], want['msg2'] = {'text': top}, {'text': m1}, {'text': m2}
            for w in ('title', 'artist', 'album', 'time'):
                want[w] = {'text': ''}
            want['cover'] = {'name': 'spotify-client'}
        want.update(self.controls())
        changes = {}
        for wid, fields in want.items():
            old = self.sent.setdefault(wid, {})
            diff = {k: v for k, v in fields.items() if old.get(k) != v}
            if diff:
                old.update(diff)
                changes[wid] = diff
        self.c.update(PAGE, changes)
        self.progress()
        leds = [self.key_led(k) for k in self.settings['keys']] if ready else [0] * 4
        if leds != self.leds_sent:
            self.leds_sent = leds
            self.c.leds(PAGE, leds)
        self.push_states()

    def controls(self):
        """Knob labels and the keys above buttons 1-4."""
        ready, p = self.status == 'ready', self.player or {}
        vol = self.volume()
        rep = REPEAT_TEXT.get(p.get('repeat_state', 'off'), 'non')

        def click(what):  # knobs 2 and 3 only click: what their click does
            return {'aléatoire': tr('shuffle: {state}', state=tr('yes') if p.get('shuffle_state') else tr('no')),
                    'répétition': tr('repeat: {mode}', mode=rep), 'rien': ''}.get(what, self.key_look(what)[1])
        clicks = self.settings['clicks']
        out = {'k1': {'text': 'position' if ready else ''},
               'k2': {'text': click(clicks[1]) if ready else ''},
               'k3': {'text': click(clicks[2]) if ready else ''},
               'k4': {'text': (tr('volume {pct} %', pct=vol) if vol is not None else 'volume') if ready else ''}}
        for k, what in enumerate(self.settings['keys'], 1):
            icon, label, on = self.key_look(what)
            color = KEY_DOWN if f'button{k}' in self.down else KEY_ON if on else KEY
            out[f'b{k}'] = {'color': color if what != 'rien' else BG}
            out[f'bi{k}'] = {'name': icon if ready else ''}
            out[f'bt{k}'] = {'text': label if ready else ''}
        return out

    def key_look(self, what):
        """(icon, label, lit) of a page button doing `what` (lit: an option that is on)."""
        p = self.player or {}
        playing, rep = self.playing(), p.get('repeat_state', 'off')
        return {
            'précédent': ('media-skip-backward', tr('previous'), False),
            'lecture / pause': ('media-playback-pause' if playing else 'media-playback-start',
                                tr('pause') if playing else tr('play'), False),
            'suivant': ('media-skip-forward', tr('next'), False),
            'j\'aime': ('emblem-favorite', tr('liked') if self.liked else tr('like'), bool(self.liked)),
            'radio du titre': ('radio', tr('track radio'), False),
            'aléatoire': ('media-playlist-shuffle', tr('shuffle'), bool(p.get('shuffle_state'))),
            'répétition': ('media-playlist-repeat', tr('repeat: {mode}', mode=REPEAT_TEXT.get(rep, tr('off'))),
                           rep != 'off'),
            'sourdine': ('audio-volume-muted', tr('mute'), self.volume() == 0),
        }.get(what, ('', '', False))

    def key_led(self, what):
        """LED of a page button: bright when its option is on (playing, liked…)."""
        if what == 'rien':
            return 0
        if what == 'lecture / pause':
            return 100 if self.playing() else 40
        if what in ('j\'aime', 'aléatoire', 'répétition', 'sourdine'):
            return 100 if self.key_look(what)[2] else 15
        return 40

    def key_action(self, what):
        return {'précédent': lambda: self.skip('previous'), 'lecture / pause': self.play_pause,
                'suivant': lambda: self.skip('next'), 'j\'aime': self.toggle_like,
                'radio du titre': self.radio, 'aléatoire': self.toggle_shuffle,
                'répétition': self.cycle_repeat, 'sourdine': self.toggle_mute}.get(what)

    def progress(self):
        """The progress bar, moved on between two reads while it plays."""
        p, it = self.player or {}, self.item()
        length, pos = it.get('duration_ms') or 0, p.get('progress_ms') or 0
        if p.get('is_playing'):
            pos += (time.monotonic() - self.read_at) * 1000
        frac = clamp(pos / length, 0.0, 1.0) if length else 0.0
        shown = (max(1, round(BAR_W * frac)), f'{clock(pos)} / {clock(length)}' if length else '')
        if self.status == 'ready' and shown != getattr(self, 'shown', None):
            self.shown = shown
            self.c.update(PAGE, {'bar': {'w': shown[0]}, 'time': {'text': shown[1]}})

    def volume(self):
        if self.volume_target is not None:
            return self.volume_target
        return ((self.player or {}).get('device') or {}).get('volume_percent')

    def cover(self, it):
        """Local file of the cover (downloaded in the background), else Spotify's icon."""
        images = (it.get('album') or {}).get('images') or it.get('images') or []
        url = images[0].get('url') if images else ''
        if not url:
            return 'spotify-client'
        path = os.path.join(CACHE, hashlib.sha1(url.encode()).hexdigest() + '.img')
        if os.path.exists(path):
            return path
        if url not in self.art:
            self.art[url] = None
            threading.Thread(target=self.download, args=(url, path), daemon=True).start()
        return 'spotify-client'

    def download(self, url, path):
        try:
            os.makedirs(CACHE, exist_ok=True)
            req = urllib.request.Request(url, headers={'User-Agent': api.USER_AGENT})
            with urllib.request.urlopen(req, timeout=8) as r:
                data = r.read(4 << 20)
            with open(path + '.part', 'wb') as f:
                f.write(data)
            os.replace(path + '.part', path)
        except OSError as e:
            log(tr('cover: {e}', e=e))
            return
        with self.c.lock:
            self.refresh()

    def pill(self, text, color=GREEN):
        self.c.pill(text, border=color, dot=color, duration=1.8, app='Spotify')

    # --- connection, polling, actions (thread `spotify`) -----------------------------------
    def set_status(self, status, message=''):
        with self.c.lock:
            self.status, self.message = status, message
            self.refresh()

    def run_loop(self):
        while not self.c.closed:
            if not (self.client_id and self.secret):
                return self.set_status('config')  # the window restarts the module once typed
            try:
                tok = self.access()
                if tok is None:
                    self.authorize()
                    continue
                if self.status != 'ready' or time.monotonic() - getattr(self, 'lists_at', 0) > 300:
                    self.read_playlists(tok)
                    self.set_status('ready')
                self.do_actions(tok)
                self.poll(tok)
            except api.ApiError as e:
                log(f'{e.where}: {e}' if e.where else f'{e}')
                if e.status == 401:
                    secrets.save('SPOTIFY_ACCESS_TOKEN', '')  # refreshed at the next turn
                elif e.status == 429:
                    self.wake.wait(max(1, e.retry))
                elif self.status != 'ready':
                    self.set_status('error', says(e))
                    self.wake.wait(30)
            self.wake.wait(POLL)
            self.wake.clear()

    def access(self):
        """A valid access token (refreshed when needed); None: an authorization is needed."""
        tok = secrets.get('SPOTIFY_ACCESS_TOKEN')
        if tok and float(secrets.get('SPOTIFY_ACCESS_EXPIRES') or 0) - time.time() > 60:
            return tok
        refresh = secrets.get('SPOTIFY_REFRESH_TOKEN')
        if not refresh:
            return None
        try:
            t = api.token(self.client_id, self.secret, refresh_token=refresh)
        except api.ApiError as e:
            if e.status not in (400, 401):
                raise
            log(tr('renewal refused ({e}): new authorization', e=e))
            for name in TOKENS:
                secrets.save(name, '')
            return None
        self.keep(t)
        return t['access_token']

    @staticmethod
    def keep(t):
        secrets.save('SPOTIFY_ACCESS_TOKEN', t['access_token'])
        secrets.save('SPOTIFY_ACCESS_EXPIRES', str(int(time.time() + t.get('expires_in', 3600))))
        if t.get('refresh_token'):
            secrets.save('SPOTIFY_REFRESH_TOKEN', t['refresh_token'])

    def authorize(self):
        """The browser on Spotify's consent page, the code at the redirect address, the token."""
        state, redirect = tokens.token_urlsafe(16), self.settings['redirect']
        url = api.authorize_url(self.client_id, redirect, state)
        self.set_status('auth')
        log(tr('authorization: browser opened'))
        try:
            code = api.wait_code(redirect, state, on_ready=lambda: self.open_browser(url))
        except OSError as e:
            self.set_status('error', tr('redirect address {redirect}: {e}', redirect=redirect, e=e))
            self.wake.wait(60)
            return
        if not code:
            self.set_status('error', tr('authorization refused or no answer'))
            self.wake.wait()
            self.wake.clear()
            return
        self.keep(api.token(self.client_id, self.secret, code=code, redirect_uri=redirect))
        log(tr('authorized: token obtained'))

    @staticmethod
    def open_browser(url):
        cmd = shlex.split(os.environ.get('HSM01_SPOTIFY_BROWSER') or 'xdg-open') + [url]
        subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         start_new_session=True)

    def read_playlists(self, tok):
        """My playlists (the settings form offers their names)."""
        lists, path = {}, '/me/playlists'
        for offset in range(0, 500, 50):
            got = api.call(tok, 'GET', path, {'limit': 50, 'offset': offset}) or {}
            for p in got.get('items') or []:
                if p and p.get('name'):
                    lists.setdefault(p['name'], (p['id'], p['uri']))
            if not got.get('next'):
                break
        self.playlists, self.lists_at = lists, time.monotonic()
        self.c.choices(playlists=sorted(lists, key=str.casefold))

    def poll(self, tok):
        p = api.call(tok, 'GET', '/me/player', {'additional_types': 'track,episode'})
        it = (p or {}).get('item') or {}
        tid, liked = it.get('id'), self.liked
        if tid and tid != self.liked_id:
            # « J'aime »: the library (tracks and episodes) since Spotify's 2026 endpoints; the
            # former /me/tracks/contains answers 403
            try:
                got = api.call(tok, 'GET', '/me/library/contains', {'uris': it.get('uri', '')}) or [False]
                liked = bool(got[0])
            except api.ApiError as e:
                if e.status in (401, 429):
                    raise
                log(f'{e.where}: {e}')
                liked = None
        with self.c.lock:
            self.player, self.read_at = p, time.monotonic()
            self.liked, self.liked_id = (liked, tid) if tid else (None, None)
            if self.volume_target is not None and self.actions.empty():
                self.volume_target = None  # sent: Spotify's own value from now on
            self.refresh()

    def do_actions(self, tok):
        """The actions asked meanwhile (buttons, knobs, commands), in order."""
        while True:
            try:
                name, act = self.actions.get_nowait()
            except queue.Empty:
                return
            try:
                act(tok)
            except api.ApiError as e:
                log(f'{name}: {e}')
                if e.status == 429:
                    raise
                if 'Restriction violated' in str(e):
                    # Spotify says it forbids less than it does: a track played alone (no
                    # playlist, album, artist) repeats in no mode at all
                    self.refused_here(name.upper())
                else:
                    self.pill(says(e).upper(), RED)
            time.sleep(0.25)  # Spotify applies, then the poll reads it back

    def ask(self, name, act):
        """Run `act(tok)` in the thread `spotify` (no call while a handler holds the lock)."""
        self.actions.put((name, act))
        self.wake.set()

    def play_call(self, tok, method, path, query=None, body=None):
        """A playback call; without an active device, again on this computer's Spotify."""
        try:
            return api.call(tok, method, path, query, body)
        except api.ApiError as e:
            if e.reason != 'NO_ACTIVE_DEVICE' and e.status != 404:
                raise
            devices = (api.call(tok, 'GET', '/me/player/devices') or {}).get('devices') or []
            dev = next((d for d in devices if d.get('type') == 'Computer'), devices[0] if devices else None)
            if not dev:
                raise
            return api.call(tok, method, path, dict(query or {}, device_id=dev['id']), body)

    # --- actions ---------------------------------------------------------------------------
    def forbids(self, what):
        """Whether Spotify forbids `what` now (/me/player actions.disallows: `toggling_shuffle`,
        `toggling_repeat_context`, `skipping_prev`… - no playlist or album playing, an ad…)."""
        return bool((((self.player or {}).get('actions') or {}).get('disallows') or {}).get(what))

    def refused_here(self, what):
        """The pill of an action Spotify forbids where the music comes from (no playlist…)."""
        self.pill(tr('{what}: NOT POSSIBLE HERE', what=what), GREY)

    def playing(self):
        """Playing now: what was just asked (Spotify says it a moment later: a second click
        must not ask the same again), else what Spotify allows (pausing: it plays), else its
        state."""
        if self.play_asked and time.monotonic() < self.play_asked[1]:
            return self.play_asked[0]
        if self.forbids('pausing') or self.forbids('resuming'):
            return self.forbids('resuming')
        return bool((self.player or {}).get('is_playing'))

    def play_pause(self):
        playing = self.playing()
        self.play_asked = (not playing, time.monotonic() + 2.0)
        self.ask('lecture', lambda t: self.play_call(t, 'PUT', '/me/player/pause' if playing else '/me/player/play'))

    def skip(self, way):
        if self.forbids(f'skipping_{"prev" if way == "previous" else "next"}'):
            return self.refused_here(tr('PREVIOUS') if way == 'previous' else tr('NEXT'))
        self.ask(way, lambda t: self.play_call(t, 'POST', f'/me/player/{way}'))

    def seek(self, steps):
        if self.forbids('seeking') or not self.player:
            return self.refused_here('POSITION') if self.player else None
        p = self.player
        pos = (p.get('progress_ms') or 0) + (time.monotonic() - self.read_at) * 1000 * bool(p.get('is_playing'))
        target = int(clamp(pos + steps * 5000, 0, (self.item().get('duration_ms') or 0)))
        # the bar moves now (the next read confirms): each step goes on from there
        p['progress_ms'], self.read_at = target, time.monotonic()
        self.progress()
        self.ask('position', lambda t: self.play_call(t, 'PUT', '/me/player/seek', {'position_ms': target}))

    def turn_volume(self, steps):
        now = self.volume()
        if now is None:
            return
        self.volume_target = clamp(now + steps * self.settings['step'], 0, 100)
        self.ask('volume', lambda t: self.volume_target is not None and self.play_call(
            t, 'PUT', '/me/player/volume', {'volume_percent': self.volume_target}))

    def toggle_mute(self):
        now = self.volume()
        if now is None:
            return
        back = self.unmuted if now == 0 and self.unmuted else None
        self.unmuted = None if back else now
        self.volume_target = back or 0
        self.pill(tr('SOUND BACK ON') if back else tr('SPOTIFY MUTED'))
        self.ask('sourdine', lambda t: self.play_call(t, 'PUT', '/me/player/volume',
                                                      {'volume_percent': self.volume_target}))

    def toggle_shuffle(self):
        if self.forbids('toggling_shuffle'):
            return self.refused_here(tr('SHUFFLE'))
        on = not (self.player or {}).get('shuffle_state')
        self.pill(tr('SHUFFLE: {state}', state=tr('ON') if on else tr('OFF')))
        self.ask('aléatoire', lambda t: self.play_call(t, 'PUT', '/me/player/shuffle',
                                                       {'state': 'true' if on else 'false'}))

    def cycle_repeat(self):
        """Off, all, track, off…, skipping the modes Spotify forbids now (« tout » needs a
        playlist or an album: a track played alone only repeats itself)."""
        now = (self.player or {}).get('repeat_state', 'off')
        allowed = [m for m in REPEAT if m == 'off' or m == now
                   or not self.forbids({'context': 'toggling_repeat_context',
                                        'track': 'toggling_repeat_track'}[m])]
        if len(allowed) < 2:
            return self.refused_here(tr('REPEAT'))
        nxt = allowed[(allowed.index(now) + 1) % len(allowed) if now in allowed else 0]
        self.pill(tr('REPEAT: {mode}', mode=REPEAT_TEXT[nxt].upper()))
        self.ask('répétition', lambda t: self.play_call(t, 'PUT', '/me/player/repeat', {'state': nxt}))

    def toggle_like(self):
        uri = self.item().get('uri')
        if not uri:
            return
        on = not self.liked
        self.liked = on
        self.pill(tr('ADDED TO LIKED SONGS') if on else tr('REMOVED FROM LIKED SONGS'))
        self.ask('j\'aime', lambda t: api.call(t, 'PUT' if on else 'DELETE', '/me/library', {'uris': uri}))

    def radio(self):
        """The radio of the track playing: Spotify's recommendations from it, else (applications
        created since the end of 2024 have none) its artist's catalogue, shuffled."""
        it = self.item()
        if not it.get('id'):
            return self.pill(tr('NOTHING PLAYING'), GREY)
        self.pill(f"RADIO: {it.get('name', '').upper()}")
        tid, artists = it['id'], [a.get('uri') for a in it.get('artists') or [] if a.get('uri')]
        self.ask('radio', lambda t: self.start_radio(t, tid, artists))

    def start_radio(self, tok, tid, artists):
        try:
            rec = api.call(tok, 'GET', '/recommendations', {'seed_tracks': tid, 'limit': 50}) or {}
            uris = [f'spotify:track:{tid}'] + [x['uri'] for x in rec.get('tracks') or [] if x.get('uri')]
            if len(uris) > 1:
                return self.play_call(tok, 'PUT', '/me/player/play', None, {'uris': uris})
        except api.ApiError as e:
            if e.status not in (403, 404):
                raise
            log(tr("radio: no recommendations ({e}): the artist's catalogue", e=e))
        if not artists:
            raise api.ApiError(404, tr('no artist for a radio'))
        # the artist first: shuffling is refused while no playlist / album / artist plays
        self.play_call(tok, 'PUT', '/me/player/play', None, {'context_uri': artists[0]})
        time.sleep(0.4)
        try:
            self.play_call(tok, 'PUT', '/me/player/shuffle', {'state': 'true'})
        except api.ApiError as e:
            log(tr('radio: shuffle refused ({e})', e=e))

    def playlist(self, name, what):
        """Start playlist `name`, or add / remove the track playing."""
        if name not in self.playlists:
            return self.pill(tr('PLAYLIST NOT FOUND: {name}', name=name.upper()), RED)
        pid, uri = self.playlists[name]
        track = self.item().get('uri')
        if what == 'lancer':
            self.pill(tr('PLAYING: {name}', name=name.upper()))
            return self.ask('liste', lambda t: self.play_call(t, 'PUT', '/me/player/play', None,
                                                               {'context_uri': uri}))
        if not track:
            return
        self.ask('liste', lambda t: self.edit_playlist(t, name, pid, track, what))

    def edit_playlist(self, tok, name, pid, track, what):
        inside, offset = False, 0
        # /playlists/{id}/items (an item's `item`) since 2026: /tracks answers 403
        while not inside and offset < 1000:  # is the track in it already?
            got = api.call(tok, 'GET', f'/playlists/{pid}/items',
                           {'fields': 'items(item(uri)),next', 'limit': 100, 'offset': offset}) or {}
            inside = any((i.get('item') or i.get('track') or {}).get('uri') == track
                         for i in got.get('items') or [])
            if not got.get('next'):
                break
            offset += 100
        if what == 'ajouter':
            if inside:
                return self.pill(tr('ALREADY IN "{name}"', name=name.upper()), GREY)
            api.call(tok, 'POST', f'/playlists/{pid}/items', None, {'uris': [track]})
            return self.pill(tr('ADDED TO "{name}"', name=name.upper()))
        if not inside:
            return self.pill(tr('NOT IN "{name}"', name=name.upper()), GREY)
        api.call(tok, 'DELETE', f'/playlists/{pid}/items', None, {'items': [{'uri': track}]})
        self.pill(tr('REMOVED FROM "{name}"', name=name.upper()))

    # --- commands --------------------------------------------------------------------------
    def offer_commands(self):
        cmd = self.c.command
        cmd('play_pause', tr('Play / pause'))(lambda ev: self.play_pause())
        cmd('next', tr('Next track'))(lambda ev: self.skip('next'))
        cmd('previous', tr('Previous track'))(lambda ev: self.skip('previous'))
        cmd('like', tr('Like / unlike'))(lambda ev: self.toggle_like())
        cmd('shuffle', tr('Shuffle mode'))(lambda ev: self.toggle_shuffle())
        cmd('repeat', tr('Repeat (off, all, track)'))(lambda ev: self.cycle_repeat())
        cmd('mute', tr('Mute'))(lambda ev: self.toggle_mute())
        cmd('radio', tr('Track radio'))(lambda ev: self.radio())
        cmd('volume', tr('Spotify volume'), kind='knob')(lambda ev: self.turn_volume(ev.get('delta', 0)))
        for name in self.settings['playlists']:
            for what, label in (('lancer', tr('Play')), ('ajouter', tr('Add the track to')),
                                ('retirer', tr('Remove the track from'))):
                cmd(f'{what}:{name}', tr('{action} "{name}"', action=label, name=name))(
                    lambda ev, name=name, what=what: self.playlist(name, what))

    def push_states(self):
        ready, p = self.status == 'ready', self.player or {}
        vol = self.volume()
        rep = p.get('repeat_state', 'off')
        st = {'play_pause': {'active': ready and self.playing(),
                             'text': self.item().get('name') or 'Spotify'},
              'like': {'active': bool(ready and self.liked)},
              'shuffle': {'active': ready and bool(p.get('shuffle_state'))},
              'repeat': {'active': ready and rep != 'off', 'text': tr('Repeat: {mode}', mode=REPEAT_TEXT.get(rep, tr('off')))},
              'mute': {'active': ready and vol == 0},
              'volume': {'active': ready and vol is not None, 'value': (vol or 0) / 100,
                         'text': f'Spotify {vol} %' if vol is not None else 'Spotify'}}
        context = (p.get('context') or {}).get('uri')
        for name in self.settings['playlists']:
            uri = self.playlists.get(name, (None, None))[1]
            st[f'lancer:{name}'] = {'active': bool(ready and uri and uri == context), 'text': name}
        for cid, s in st.items():
            if self.states.get(cid) != s:
                self.states[cid] = s
                self.c.command_state(cid, **s)

    # --- inputs on the page ----------------------------------------------------------------
    @staticmethod
    def mine(ev):
        return ev.get('mine') and ev.get('page') == PAGE

    def key(self, ev, down):
        b = ev.get('button', '')
        if self.mine(ev) and b.startswith('button'):
            (self.down.add if down else self.down.discard)(b)
            self.refresh()

    def on_turn(self, ev):
        if not self.mine(ev) or self.status != 'ready':
            return
        k, d = ev.get('knob'), ev.get('delta', 0)
        if k == 1:
            self.seek(d)
        elif k == 4:
            self.turn_volume(d)
        self.refresh()

    def on_hold(self, ev):
        """The button « suivant » (or « radio du titre ») held: the radio of the track playing."""
        b = ev.get('button', '')
        if not (self.mine(ev) and self.status == 'ready' and b.startswith('button')):
            return
        if self.settings['keys'][int(b[-1]) - 1] in ('suivant', 'radio du titre'):
            self.radio()
            self.refresh()

    def on_click(self, ev):
        if not self.mine(ev):
            return
        if self.status == 'error':
            self.wake.set()  # try again now
            return
        if self.status != 'ready':
            return
        b, act = ev.get('button', ''), None
        if b[-1:] in ('1', '2', '3', '4') and b[:-1] in ('button', 'knob'):
            act = self.key_action(self.settings['keys' if b[0] == 'b' else 'clicks'][int(b[-1]) - 1])
        if act:
            act()
            self.refresh()


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'spotify')
    sp = Spotify(client, args)
    client.run(tick=lambda dt: sp.progress(), fps=2)


if __name__ == '__main__':
    main()
