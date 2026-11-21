#!/usr/bin/env python3
"""Module « OBS » de new hera-nux: OBS Studio through its WebSocket server (obs-websocket v5,
built into OBS 28 and later: Outils > Paramètres du serveur WebSocket). Address, port and
password: those of OBS on this computer (read in its settings), else those of the module's tab.

Page « OBS »: the scenes (knob 1: choose, click: show it; in studio mode: into the preview, a
second click: the transition), three audio sources of OBS's mixer (knobs 2-4: volume along OBS's
slider, click: mute) with their meters, the stream's and the recording's times; buttons
(obs.toml [boutons]; by default stream, record, pause, replay; stopping the stream or the
recording from them: hold the button). Commands (tab Commandes, any page): the same, each scene,
each audio source (mute, volume), favourite sources to show / hide, filters, media to play again,
profiles, scene collections. Widget « État d'OBS » (scene, stream, recording). Option: --show.
"""
import argparse
import json
import math
import os
import queue
import threading
import time
import tomllib

from hsm01 import Client, secrets
from hsm01 import widgets as W
from hsm01.i18n import point, tr

import obs_ws as ows

PAGE, STATE = 'obs', 'etat'
HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'obs')
KNOWN = os.path.join(CACHE, 'connu.json')  # names seen last time: commands, lists of the form
BG, TEXT, SOFT, GREY, DIM, LINE = '#0d0e12', '#e9eaf0', '#b4b7c4', '#8b8fa0', '#5b5f70', '#1c1e26'
RED, GREEN, AMBER, BLUE = '#e5484d', '#3ecf6e', '#f0b429', '#5b8cff'
DARK = {RED: '#4a1d22', GREEN: '#173d26', AMBER: '#4a3a12', BLUE: '#1d2a4a'}
KEY, KEY_DOWN, TRACK, SELECT = '#16171d', '#2a2c36', '#22242c', '#262a3a'
ROWS, ROW_H, TOP = 5, 32, 34  # the scenes' list
METER_Y, METER_H = 56, 136  # meters and sliders
STATE_W, STATE_H = 220, 68
OUT = 'OBS_WEBSOCKET_OUTPUT_'
# what the page's buttons 1-4 can do (obs.toml [boutons] b1..b4, the form's choices)
KEYS = ['direct', 'enregistrement', 'pause', 'replay', 'caméra virtuelle', 'transition',
        'mode studio', 'rien']
DEFAULT_KEYS = ['direct', 'enregistrement', 'pause', 'replay']
MEDIA_KINDS = ('ffmpeg_source', 'vlc_source')
LOCAL = ('', '127.0.0.1', 'localhost', '::1')
REFUSALS = {500: tr('ALREADY RUNNING'), 501: tr('NOT RUNNING'), 502: tr('PAUSED'), 503: tr('NOT PAUSED'),
            504: tr('DISABLED IN OBS'), 505: tr('STUDIO MODE ON'), 506: tr('STUDIO MODE OFF'),
            600: tr('NOT FOUND'), 604: tr('NOT POSSIBLE HERE')}


def log(text):
    """A line in the Journal and in ~/.cache/hsm01/obs/journal.log (never the password)."""
    print(f'OBS: {text}', flush=True)
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
    h, m = divmod(s // 60, 60)
    return f'{h}:{m:02d}:{s % 60:02d}' if h else f'{m}:{s % 60:02d}'


# OBS's mixer slider: logarithmic from -96 dB (bottom) to 0 dB (top), as libobs's log fader
LOG_OFFSET, LOG_RANGE = 6.0, 96.0


def position(mul):
    """Where OBS's mixer slider stands (0..1) for a volume multiplier."""
    if mul <= 0:
        return 0.0
    db = 20 * math.log10(mul)
    if db >= 0:
        return 1.0
    if db <= -LOG_RANGE:
        return 0.0
    top = math.log10(LOG_RANGE + LOG_OFFSET)
    return (top - math.log10(LOG_OFFSET - db)) / (top - math.log10(LOG_OFFSET))


def multiplier(pos):
    """The volume multiplier of OBS's slider at `pos` (0..1)."""
    if pos <= 0:
        return 0.0
    if pos >= 1:
        return 1.0
    db = LOG_OFFSET - (LOG_RANGE + LOG_OFFSET) * (LOG_OFFSET / (LOG_RANGE + LOG_OFFSET)) ** pos
    return 10 ** (db / 20)


def db_text(mul):
    return '-∞ dB' if mul <= 0 else point(f'{20 * math.log10(mul):.1f} dB')


def meter(peak):
    """Height (0..1) of a meter for a peak (multiplier): -60 dB to 0 dB as OBS's meters."""
    return 0.0 if peak <= 0 else clamp((20 * math.log10(peak) + 60) / 60, 0.0, 1.0)


def obs_config():
    """OBS's own WebSocket settings on this computer: {'enabled', 'port', 'password'} ({} when
    OBS keeps none: never started…)."""
    home = os.environ.get('XDG_CONFIG_HOME') or os.path.expanduser('~/.config')
    for base in (os.environ.get('HSM01_OBS_CONFIG') or os.path.join(home, 'obs-studio'),
                 os.path.expanduser('~/.var/app/com.obsproject.Studio/config/obs-studio')):
        try:
            with open(os.path.join(base, 'plugin_config', 'obs-websocket', 'config.json'),
                      encoding='utf-8') as f:
                d = json.load(f)
            port = int(d.get('server_port') or 4455)
        except (OSError, ValueError, TypeError):
            continue
        return {'enabled': bool(d.get('server_enabled')), 'port': port,
                'password': str(d.get('server_password') or '') if d.get('auth_required') else ''}
    return {}


def load_settings(path):
    """obs.toml: knobs' sources, slider step, buttons, pills, favourites, address."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        log(f'{path}: {e}')
        data = {}

    def table(key):
        v = data.get(key)
        return v if isinstance(v, dict) else {}

    def rows(key, *fields):
        return [tuple(str(r[f]).strip() for f in fields) for r in data.get(key) or []
                if isinstance(r, dict) and all(r.get(f) for f in fields)]

    knobs, keys, pills = table('molettes'), table('boutons'), table('pilules')
    try:
        port = int(data['port']) if data.get('port') else None
    except (TypeError, ValueError):
        port = None
    try:
        step = clamp(float(data.get('pas', 2)), 0.5, 10.0)
    except (TypeError, ValueError):
        step = 2.0
    keys = [str(keys.get(f'b{i + 1}') or d) for i, d in enumerate(DEFAULT_KEYS)]
    return {'host': str(data.get('adresse') or '').strip(), 'port': port,
            'knobs': [str(knobs.get(f'k{i}') or '').strip() for i in (2, 3, 4)], 'step': step,
            'keys': [k if k in KEYS else d for k, d in zip(keys, DEFAULT_KEYS)],
            'confirm': bool(data.get('maintenir_pour_arreter', True)),
            'meters': bool(data.get('vumetres', True)),
            'pills': {k: bool(pills.get(k, True)) for k in ('scene', 'son', 'sorties')},
            'sources': rows('source', 'scene', 'source'), 'filters': rows('filtre', 'source', 'filtre'),
            'medias': [m for (m,) in rows('media', 'source')]}


class Studio:
    def __init__(self, client, args):
        self.c, self.args = client, args
        self.settings = load_settings(os.path.join(HERE, 'obs.toml'))
        self.obs, self.status, self.message = None, 'start', ''
        self.scenes, self.program, self.preview, self.studio, self.selected = [], '', '', False, 0
        self.inputs, self.order, self.special = {}, [], {}  # audio inputs: name -> kind, mul, muted
        self.levels, self.shown_levels = {}, {}  # peaks since the last tick, heights shown
        self.pending = {}  # audio input -> (mul asked, until): OBS's echoes of older values ignored
        self.vol_queued = set()
        self.stream = {'active': False, 'state': '', 'ms': 0, 'at': 0.0, 'reconnecting': False}
        self.record = {'active': False, 'state': '', 'ms': 0, 'at': 0.0, 'paused': False}
        self.replay, self.vcam = None, False  # replay: None while OBS has none (settings)
        self.profiles, self.profile, self.collections, self.collection = [], '', [], ''
        self.items, self.filters, self.playing, self.lists = {}, {}, set(), {}
        self.sent, self.state_sent, self.states, self.leds_sent, self.down = {}, {}, {}, None, set()
        self.offered, self.reread_queued, self.times_shown = None, False, None
        self.known = {}
        try:
            with open(KNOWN, encoding='utf-8') as f:
                self.known = json.load(f)
        except (OSError, ValueError):
            pass
        self.actions, self.retry = queue.Queue(), threading.Event()
        client.page(PAGE, self.widgets(), name='OBS', background=BG, show=args.show)
        client.widget(STATE, STATE_W, STATE_H, self.state_widgets(), name=tr('OBS status'),
                      background=BG)
        with client.lock:
            client.on('turn', self.on_turn)
            client.on('click', self.on_click)
            client.on('hold', self.on_hold)
            client.on('press', lambda ev: self.key(ev, True))
            client.on('release', lambda ev: self.key(ev, False))
            self.offer_commands()
            self.publish_choices()
            self.refresh()
        threading.Thread(target=self.connection, name='obs', daemon=True).start()

    # --- the page --------------------------------------------------------------------------
    @staticmethod
    def widgets():
        out = [W.icon('logo', 8, 5, 22, 22, 'com.obsproject.Studio'),
               W.text('title', 36, 3, 196, 26, 'OBS', size=16, bold=True, align='left', color=TEXT),
               W.rect('lv', 236, 6, 118, 20, TRACK, radius=10),
               W.text('lvt', 236, 6, 118, 20, '', size=12, bold=True, color=DIM),
               W.rect('rc', 358, 6, 114, 20, TRACK, radius=10),
               W.text('rct', 358, 6, 114, 20, '', size=12, bold=True, color=DIM)]
        out += [W.rect(f'sep{k}', 120 * k, TOP, 1, 160, LINE) for k in (1, 2, 3)]
        for i in range(ROWS):
            y = TOP + i * ROW_H
            out += [W.rect(f's{i}', 4, y, 112, ROW_H - 3, BG, radius=6),  # selection
                    W.rect(f'p{i}', 6, y + 5, 3, ROW_H - 13, BG, radius=1),  # program / preview
                    W.text(f'n{i}', 13, y, 101, ROW_H - 3, '', size=13, align='left', color=SOFT)]
        for k in (2, 3, 4):
            x0, cx = 120 * (k - 1), 120 * (k - 1) + 60
            out += [W.text(f'an{k}', x0 + 4, TOP, 112, 18, '', size=12, bold=True, color=TEXT),
                    W.rect(f'mb{k}', cx - 22, METER_Y, 14, METER_H, BG, radius=3),
                    W.rect(f'm{k}', cx - 22, METER_Y + METER_H - 1, 14, 1, BG, radius=3),
                    W.rect(f'fb{k}', cx + 2, METER_Y, 20, METER_H, BG, radius=4),
                    W.rect(f'f{k}', cx + 2, METER_Y + METER_H - 1, 20, 1, BG, radius=4)]
        # over the (empty) columns: what to do while not connected
        out += [W.text('msg1', 16, 84, 448, 30, '', size=17, color=TEXT),
                W.text('msg2', 16, 120, 448, 24, '', size=13, color=GREY)]
        edge = W.frame(border='#262833', width=1.0, radius=10, padding=0)
        for k in range(4):
            cx = 60 + k * 120
            out += [W.text(f'k{k + 1}', k * 120, 199, 120, 16, '', size=12, color=GREY),
                    W.rect(f'b{k + 1}', cx - 44, 222, 88, 46, KEY, radius=10, frame=edge),
                    W.icon(f'bi{k + 1}', cx - 12, 225, 24, 24, '', tint=TEXT),
                    W.text(f'bt{k + 1}', cx - 44, 251, 88, 14, '', size=11, color=GREY)]
        return out

    @staticmethod
    def state_widgets():
        return [W.text('es', 8, 2, 204, 22, 'OBS', size=14, bold=True, align='left', color=TEXT),
                W.rect('eld', 10, 32, 10, 10, DIM, radius=5),
                W.text('elt', 26, 26, 190, 20, '', size=13, align='left', color=GREY),
                W.rect('erd', 10, 52, 10, 10, DIM, radius=5),
                W.text('ert', 26, 46, 190, 20, '', size=13, align='left', color=GREY)]

    def header(self):
        """(message, its second line) while not connected."""
        return {
            'absent': (tr('OBS is not running'), tr('new try every 5 s ({why})', why=self.message)),
            'off': (tr('OBS WebSocket server off'),
                    tr('OBS: Tools > WebSocket Server Settings > Enable')),
            'refused': (self.message or tr('Password refused by OBS'),
                        tr('"OBS" tab of the window; click: try again')),
        }.get(self.status, (tr('Connecting to OBS…'), ''))

    def sources(self):
        """The audio inputs of knobs 2-4: chosen in obs.toml, else the mic, the desktop audio,
        then the next ones in OBS's order ('' : none)."""
        auto = [self.special.get(k) for k in ('mic1', 'desktop1', 'mic2', 'desktop2', 'mic3', 'mic4')]
        auto = [n for n in auto if n in self.inputs] + self.order
        chosen, out = self.settings['knobs'], []
        for c in chosen:
            out.append(c or next((n for n in auto if n not in out and n not in chosen), ''))
        return out

    def elapsed(self, o):
        running = o['active'] and not o.get('paused') and not o.get('reconnecting')
        return o['ms'] + ((time.monotonic() - o['at']) * 1000 if running else 0)

    def times(self):
        """Texts and colours of the stream and recording badges (header, widget)."""
        s, r = self.stream, self.record
        live = (tr('LIVE {time}', time=clock(self.elapsed(s))), AMBER if s['reconnecting'] else RED) \
            if s['active'] else (tr('LIVE'), None)
        if r['active']:
            rec = (f'{"PAUSE" if r["paused"] else "REC"} {clock(self.elapsed(r))}',
                   AMBER if r['paused'] else RED)
        else:
            rec = ('REC', None)
        return live, rec

    def key_look(self, what):
        """(icon, label, lit, colour when lit) of a page button doing `what`."""
        live, rec, paused = self.stream['active'], self.record['active'], self.record['paused']
        return {
            'direct': ('network-wireless', tr('stop') if live else tr('live'), live, RED),
            'enregistrement': ('media-record', tr('stop') if rec else tr('record'), rec, RED),
            'pause': ('media-playback-start' if paused else 'media-playback-pause',
                      tr('resume') if paused else tr('pause'), paused, AMBER),
            'replay': ('document-save', tr('save replay') if self.replay else 'replay',
                       bool(self.replay), GREEN),
            'caméra virtuelle': ('camera-web', tr('camera'), self.vcam, BLUE),
            'transition': ('media-seek-forward', 'transition', self.studio, BLUE),
            'mode studio': ('video-display', 'studio', self.studio, BLUE),
        }.get(what, ('', '', False, KEY))

    def refresh(self):
        """Send what changed on the page, the widget, the LEDs and the commands' states."""
        ready = self.status == 'ready'
        want = {}
        m1, m2 = self.header() if not ready else ('', '')
        want['msg1'], want['msg2'] = {'text': m1}, {'text': m2}
        want['title'] = {'text': (self.program or 'OBS') if ready else 'OBS'}
        (lt, lc), (rt, rc) = self.times()
        want['lv'], want['lvt'] = {'color': lc or TRACK}, {'text': lt if ready else '',
                                                          'color': TEXT if lc else DIM}
        want['rc'], want['rct'] = {'color': rc or TRACK}, {'text': rt if ready else '',
                                                          'color': TEXT if rc else DIM}
        scenes = self.scenes if ready else []
        self.selected = clamp(self.selected, 0, max(0, len(scenes) - 1))
        first = self.selected // ROWS * ROWS
        for i in range(ROWS):
            n = first + i
            name = scenes[n] if n < len(scenes) else ''
            mark = RED if name and name == self.program else \
                GREEN if name and self.studio and name == self.preview else BG
            want[f's{i}'] = {'color': SELECT if name and n == self.selected else BG}
            want[f'p{i}'] = {'color': mark}
            want[f'n{i}'] = {'text': name, 'color': TEXT if mark == RED else SOFT}
        for k in range(1, 4):
            want[f'sep{k}'] = {'color': LINE if ready else BG}
        for k, name in zip((2, 3, 4), self.sources()):
            a = self.inputs.get(name) if ready else None
            h = round(METER_H * position(a['mul'])) if a else 0
            want[f'an{k}'] = {'text': name if ready else '', 'color': RED if a and a['muted'] else TEXT}
            want[f'mb{k}'] = {'color': TRACK if a else BG}
            want[f'fb{k}'] = {'color': TRACK if a else BG}
            want[f'f{k}'] = {'y': METER_Y + METER_H - max(1, h), 'h': max(1, h),
                             'color': (DIM if a['muted'] else BLUE) if a and h else BG}
            want[f'k{k}'] = {'text': '' if not ready or not name else 'introuvable' if not a
                             else tr('muted') if a['muted'] else db_text(a['mul'])}
            if not a:
                want[f'm{k}'] = {'color': BG}
        want['k1'] = {'text': (tr('scene {n}/{total}', n=self.selected + 1, total=len(scenes)) if scenes else tr('no scene'))
                      if ready else ''}
        for k, what in enumerate(self.settings['keys'], 1):
            icon, label, lit, color = self.key_look(what)
            bg = KEY_DOWN if f'button{k}' in self.down else DARK.get(color, KEY) if lit else KEY
            want[f'b{k}'] = {'color': bg if what != 'rien' else BG}
            want[f'bi{k}'] = {'name': icon if ready else ''}
            want[f'bt{k}'] = {'text': label if ready else ''}
        self.c.update(PAGE, self.diff(self.sent, want))
        leds = [0 if w == 'rien' else 100 if self.key_look(w)[2] else 15
                for w in self.settings['keys']] if ready else [0] * 4
        if leds != self.leds_sent:
            self.leds_sent = leds
            self.c.leds(PAGE, leds)
        self.refresh_state()
        self.push_states()

    @staticmethod
    def diff(sent, want):
        changes = {}
        for wid, fields in want.items():
            old = sent.setdefault(wid, {})
            d = {k: v for k, v in fields.items() if old.get(k) != v}
            if d:
                old.update(d)
                changes[wid] = d
        return changes

    def refresh_state(self):
        """The « État d'OBS » widget: scene, stream, recording."""
        ready = self.status == 'ready'
        (lt, lc), (rt, rc) = self.times()
        want = {'es': {'text': (self.program or 'OBS') if ready else tr('OBS not connected')},
                'eld': {'color': lc or DIM}, 'elt': {'text': lt if lc else tr('not live') if ready else '',
                                                     'color': TEXT if lc else GREY},
                'erd': {'color': rc or DIM}, 'ert': {'text': rt if rc else tr('not recording')
                                                     if ready else '', 'color': TEXT if rc else GREY}}
        self.c.update(STATE, self.diff(self.state_sent, want))

    def tick(self, dt):
        """10 times a second: the meters (falling slowly), the times once a second."""
        ready = self.status == 'ready'
        for k, name in zip((2, 3, 4), self.sources()):
            a = self.inputs.get(name) if ready else None
            peak = self.levels.pop(name, 0.0) if a else 0.0
            frac = max(meter(peak), self.shown_levels.get(k, 0.0) - 0.06)
            self.shown_levels[k] = frac
            h = round(METER_H * frac)
            color = BG if not a or h == 0 else RED if frac > 51 / 60 else AMBER if frac > 40 / 60 else GREEN
            self.c.update(PAGE, self.diff(self.sent, {f'm{k}': {
                'y': METER_Y + METER_H - max(1, h), 'h': max(1, h), 'color': color}}))
        shown = self.times()
        if ready and shown != self.times_shown:
            self.times_shown = shown
            self.refresh()

    def pill(self, text, color=BLUE):
        self.c.pill(text, border=color, dot=color, duration=2.0, app='OBS')

    # --- commands ------------------------------------------------------------------------------
    def offer_commands(self):
        """The commands, OBS's lists (scenes, audio inputs, profiles…) as read, else as seen last
        time (bindings work from the start); sent again only when they change."""
        k = self.known
        scenes, audio = self.scenes or k.get('scenes', []), self.order or k.get('audio', [])
        profiles = self.profiles or k.get('profiles', [])
        collections = self.collections or k.get('collections', [])
        e = [('direct', tr('Start / stop streaming'), lambda ev: self.toggle_output('stream')),
             ('enregistrement', tr('Start / stop recording'),
              lambda ev: self.toggle_output('record')),
             ('pause', tr('Pause / resume recording'), lambda ev: self.pause()),
             ('replay', tr('Save the replay'), lambda ev: self.save_replay()),
             ('camera', tr('Virtual camera'), lambda ev: self.toggle_vcam()),
             ('transition', tr('Transition (studio mode)'), lambda ev: self.transition()),
             ('studio', tr('Studio mode'), lambda ev: self.toggle_studio()),
             ('scenes', tr('Next / previous scene'), lambda ev: self.step_scene(ev.get('delta', 0)),
              'knob')]
        e += [(f'scene:{s}', tr('Scene "{name}"', name=s), lambda ev, s=s: self.show_scene(s)) for s in scenes]
        for a in audio:
            e += [(f'couper:{a}', tr('Mute "{name}"', name=a), lambda ev, a=a: self.toggle_mute(a)),
                  (f'volume:{a}', tr('Volume of "{name}"', name=a),
                   lambda ev, a=a: self.turn_volume(a, ev.get('delta', 0)), 'knob')]
        e += [(f'source:{s}/{x}', tr('Show / hide "{item}" ({scene})', item=x, scene=s),
               lambda ev, s=s, x=x: self.toggle_item(s, x)) for s, x in self.settings['sources']]
        e += [(f'filtre:{s}/{f}', tr('Filter "{filter}" of "{source}"', filter=f, source=s),
               lambda ev, s=s, f=f: self.toggle_filter(s, f)) for s, f in self.settings['filters']]
        e += [(f'media:{m}', tr('Play "{name}"', name=m), lambda ev, m=m: self.play_media(m))
              for m in self.settings['medias']]
        e += [(f'profil:{p}', tr('Profile "{name}"', name=p), lambda ev, p=p: self.set_profile(p)) for p in profiles]
        e += [(f'collection:{c}', tr('Collection "{name}"', name=c), lambda ev, c=c: self.set_collection(c))
              for c in collections]
        ids = [x[0] for x in e]
        if ids != self.offered:
            self.offered, self.states = ids, {}
            self.c.commands(e, replace=True)

    def push_states(self):
        ready = self.status == 'ready'
        (lt, lc), (rt, rc) = self.times()
        st = {'direct': {'active': ready and self.stream['active'], 'text': lt.capitalize()},
              'enregistrement': {'active': ready and self.record['active'],
                                 'text': rt.replace('REC', tr('Recording')).capitalize()},
              'pause': {'active': ready and self.record['paused']},
              'replay': {'active': ready and bool(self.replay)},
              'camera': {'active': ready and self.vcam},
              'transition': {'active': ready and self.studio},
              'studio': {'active': ready and self.studio},
              'scenes': {'active': ready, 'text': self.program or 'OBS'}}
        for s in self.scenes:
            st[f'scene:{s}'] = {'active': ready and s == self.program}
        for name, a in self.inputs.items():
            st[f'couper:{name}'] = {'active': ready and a['muted']}
            st[f'volume:{name}'] = {'active': ready and not a['muted'], 'value': position(a['mul']),
                                    'text': f'{name} {db_text(a["mul"])}'}
        for s, x in self.settings['sources']:
            st[f'source:{s}/{x}'] = {'active': ready and bool(self.items.get((s, x), [0, False])[1])}
        for s, f in self.settings['filters']:
            st[f'filtre:{s}/{f}'] = {'active': ready and bool(self.filters.get((s, f)))}
        for m in self.settings['medias']:
            st[f'media:{m}'] = {'active': ready and m in self.playing}
        for p in self.profiles:
            st[f'profil:{p}'] = {'active': ready and p == self.profile}
        for c in self.collections:
            st[f'collection:{c}'] = {'active': ready and c == self.collection}
        for cid, s in st.items():
            if cid in (self.offered or ()) and self.states.get(cid) != s:
                self.states[cid] = s
                self.c.command_state(cid, **s)

    def publish_choices(self):
        lists = self.lists or self.known.get('lists') or {}
        if lists:
            self.c.choices(**lists)

    def remember(self):
        try:
            os.makedirs(CACHE, exist_ok=True)
            with open(KNOWN + '.part', 'w', encoding='utf-8') as f:
                json.dump({'scenes': self.scenes, 'audio': self.order, 'profiles': self.profiles,
                           'collections': self.collections, 'lists': self.lists}, f, ensure_ascii=False)
            os.replace(KNOWN + '.part', KNOWN)
        except OSError as e:
            log(f'{KNOWN}: {e}')

    # --- connection (thread `obs`) -------------------------------------------------------------
    def set_status(self, status, message=''):
        with self.c.lock:
            self.status, self.message = status, message
            self.refresh()

    def wait(self, seconds):
        self.retry.wait(seconds)
        self.retry.clear()

    def target(self):
        """(host, port, password, OBS's own settings when it runs here)."""
        s = self.settings
        host = s['host'] or '127.0.0.1'
        conf = obs_config() if host in LOCAL else {}
        port = s['port'] or conf.get('port') or 4455
        password = (secrets.get('OBS_PASSWORD') or '').strip() or conf.get('password', '')
        return host, port, password, conf

    def connection(self):
        while not self.c.closed:
            host, port, password, conf = self.target()
            events = ows.ALL_EVENTS | (ows.VOLUME_METERS if self.settings['meters'] else 0)
            try:
                obs = ows.Obs(host, port, password, self.on_event, events=events)
            except ows.ObsError as e:
                if e.close == ows.AUTH_FAILED:
                    log(f'{host}:{port}: {e}')
                    self.set_status('refused', str(e)[:1].upper() + str(e)[1:])
                    self.wait(None)  # a click tries again; a new password restarts the module
                else:
                    self.set_status('off' if conf and not conf.get('enabled') else 'absent',
                                    f'{host}:{port}')
                    self.wait(5)
                continue
            log(tr('connected to {address} (obs-websocket {version})', address=f'{host}:{port}', version=obs.version))
            while not self.actions.empty():
                self.actions.get_nowait()
            self.obs = obs
            try:
                self.read_all(obs)
                self.serve(obs)
            except ows.ObsError as e:
                log(f'{e}')
            finally:
                self.obs = None
                obs.close()
            if not self.c.closed:
                log(tr('disconnected'))
                self.set_status('absent', f'{host}:{port}')
                self.wait(2)

    def serve(self, obs):
        """The actions asked meanwhile, in order; the times read again every 2 s while live."""
        polled = 0.0
        while not obs.closed and not self.c.closed:
            try:
                name, act = self.actions.get(timeout=0.5)
            except queue.Empty:
                name = None
            if name:
                try:
                    act(obs)
                except ows.ObsError as e:
                    if obs.closed:
                        raise
                    log(f'{name}: {e}')
                    self.pill(f'{name.upper()}: {REFUSALS.get(e.code) or str(e).upper()}', RED)
            if time.monotonic() - polled > 2 and (self.stream['active'] or self.record['active']):
                polled = time.monotonic()
                self.read_times(obs)

    def read_times(self, obs):
        s, r = obs.request('GetStreamStatus'), obs.request('GetRecordStatus')
        now = time.monotonic()
        with self.c.lock:
            if s.get('outputActive'):
                self.stream.update(ms=int(s.get('outputDuration') or 0), at=now,
                                   reconnecting=bool(s.get('outputReconnecting')))
            if r.get('outputActive'):
                self.record.update(ms=int(r.get('outputDuration') or 0), at=now,
                                   paused=bool(r.get('outputPaused')))

    @staticmethod
    def optional(obs, kind):
        """Whether output `kind` runs; None when OBS has none (replay buffer off in its settings)."""
        try:
            return bool(obs.request(kind).get('outputActive'))
        except ows.ObsError:
            if obs.closed:
                raise
            return None

    def read_all(self, obs):
        """Everything shown or offered: scenes, audio inputs, outputs, profiles, collections,
        the sources of each scene and the filters of each source (lists of the form)."""
        sl = obs.request('GetSceneList')
        by_index = sorted(sl.get('scenes') or [], key=lambda s: -int(s.get('sceneIndex', 0)))
        scenes = [s.get('sceneName', '') for s in by_index]  # the OBS window's order, top first
        studio = bool(obs.request('GetStudioModeEnabled').get('studioModeEnabled'))
        all_inputs = obs.request('GetInputList').get('inputs') or []
        inputs, order = {}, []
        for i in all_inputs:
            name = i.get('inputName', '')
            try:
                vol = obs.request('GetInputVolume', {'inputName': name})
                muted = obs.request('GetInputMute', {'inputName': name}).get('inputMuted')
            except ows.ObsError:  # no audio (camera, text…)
                if obs.closed:
                    raise
                continue
            inputs[name] = {'kind': i.get('inputKind', ''), 'muted': bool(muted),
                            'mul': float(vol.get('inputVolumeMul') or 0.0)}
            order.append(name)
        special = obs.request('GetSpecialInputs')
        stream, record = obs.request('GetStreamStatus'), obs.request('GetRecordStatus')
        replay, vcam = self.optional(obs, 'GetReplayBufferStatus'), self.optional(obs, 'GetVirtualCamStatus')
        prof, coll = obs.request('GetProfileList'), obs.request('GetSceneCollectionList')
        items, elements = {}, {}
        for s in scenes:
            got = obs.request('GetSceneItemList', {'sceneName': s}).get('sceneItems') or []
            got = sorted(got, key=lambda x: -int(x.get('sceneItemIndex', 0)))
            elements[f'elements:{s}'] = [x.get('sourceName', '') for x in got]
            for x in got:
                items[(s, x.get('sourceName', ''))] = [x.get('sceneItemId'), bool(x.get('sceneItemEnabled'))]
        filters, by_source = {}, {}
        for src in scenes + [i.get('inputName', '') for i in all_inputs]:
            try:
                got = obs.request('GetSourceFilterList', {'sourceName': src}).get('filters') or []
            except ows.ObsError:
                if obs.closed:
                    raise
                continue
            if got:
                by_source[f'filtres:{src}'] = [f.get('filterName', '') for f in got]
            for f in got:
                filters[(src, f.get('filterName', ''))] = bool(f.get('filterEnabled'))
        medias = [i.get('inputName', '') for i in all_inputs
                  if (i.get('unversionedInputKind') or i.get('inputKind')) in MEDIA_KINDS]
        now = time.monotonic()
        with self.c.lock:
            self.scenes, self.studio = scenes, studio
            self.program = sl.get('currentProgramSceneName') or ''
            self.preview = sl.get('currentPreviewSceneName') or ''
            if self.program in scenes and self.status != 'ready':  # read again: the choice stays
                self.selected = scenes.index(self.program)
            self.inputs, self.order, self.special = inputs, order, special
            self.stream.update(active=bool(stream.get('outputActive')), at=now,
                               ms=int(stream.get('outputDuration') or 0),
                               reconnecting=bool(stream.get('outputReconnecting')))
            self.record.update(active=bool(record.get('outputActive')), at=now,
                               ms=int(record.get('outputDuration') or 0),
                               paused=bool(record.get('outputPaused')))
            self.replay, self.vcam = replay, bool(vcam)
            self.profiles, self.profile = list(prof.get('profiles') or []), prof.get('currentProfileName', '')
            self.collections = list(coll.get('sceneCollections') or [])
            self.collection = coll.get('currentSceneCollectionName', '')
            self.items, self.filters = items, filters
            self.lists = dict({'scenes': scenes, 'entrees': order, 'medias': medias,
                               'avec_filtres': [k.split(':', 1)[1] for k in by_source]},
                              **elements, **by_source)
            self.status, self.reread_queued = 'ready', False
            self.offer_commands()
            self.publish_choices()
            self.remember()
            self.refresh()

    def reread(self):
        """Read everything again (a scene, a source, a collection… came, went or was renamed)."""
        if not self.reread_queued:
            self.reread_queued = True
            self.actions.put(('relecture', self.read_all))

    # --- OBS's events (its reader thread) ------------------------------------------------------
    def on_event(self, kind, d):
        if kind == 'InputVolumeMeters':  # 20 a second: kept for the next tick only
            for i in d.get('inputs') or []:
                chans = [ch[1] for ch in i.get('inputLevelsMul') or [] if len(ch) > 1]
                name, peak = i.get('inputName'), max(chans, default=0.0)
                if peak > self.levels.get(name, 0.0):
                    self.levels[name] = peak
            return
        with self.c.lock:
            self.apply(kind, d)
            self.refresh()

    def apply(self, kind, d):
        pills = self.settings['pills']
        name = d.get('inputName', '')
        if kind == 'CurrentProgramSceneChanged':
            self.program = d.get('sceneName', '')
            if self.program in self.scenes:
                self.selected = self.scenes.index(self.program)
            if pills['scene']:
                self.pill(tr('SCENE: {name}', name=self.program.upper()))
        elif kind == 'CurrentPreviewSceneChanged':
            self.preview = d.get('sceneName', '')
            if pills['scene']:
                self.pill(tr('PREVIEW: {name}', name=self.preview.upper()), GREEN)
        elif kind == 'SceneListChanged':
            by_index = sorted(d.get('scenes') or [], key=lambda s: -int(s.get('sceneIndex', 0)))
            self.scenes = [s.get('sceneName', '') for s in by_index]
            self.reread()
        elif kind in ('SceneNameChanged', 'SceneCreated', 'SceneRemoved', 'InputCreated',
                      'InputRemoved', 'InputNameChanged', 'SceneItemCreated', 'SceneItemRemoved',
                      'SourceFilterCreated', 'SourceFilterRemoved', 'SourceFilterNameChanged',
                      'ProfileListChanged', 'SceneCollectionListChanged',
                      'CurrentSceneCollectionChanged'):
            self.reread()
        elif kind == 'StudioModeStateChanged':
            self.studio = bool(d.get('studioModeEnabled'))
            self.pill(tr('STUDIO MODE: {state}', state=tr('ON') if self.studio else tr('OFF')))
            if self.studio:
                self.reread()  # its preview
        elif kind in ('StreamStateChanged', 'RecordStateChanged'):
            self.output_changed('stream' if kind[0] == 'S' else 'record', d)
        elif kind == 'ReplayBufferStateChanged':
            self.replay = bool(d.get('outputActive'))
            if pills['sorties'] and d.get('outputState') in (OUT + 'STARTED', OUT + 'STOPPED'):
                self.pill(tr('REPLAY READY') if self.replay else tr('REPLAY STOPPED'), GREEN if self.replay else GREY)
        elif kind == 'ReplayBufferSaved':
            if pills['sorties']:
                self.pill(tr('REPLAY SAVED'), GREEN)
        elif kind == 'VirtualcamStateChanged':
            self.vcam = bool(d.get('outputActive'))
            if pills['sorties'] and d.get('outputState') in (OUT + 'STARTED', OUT + 'STOPPED'):
                self.pill(tr('VIRTUAL CAMERA: {state}', state=tr('ON') if self.vcam else tr('OFF')), BLUE if self.vcam else GREY)
        elif kind == 'InputMuteStateChanged' and name in self.inputs:
            self.inputs[name]['muted'] = bool(d.get('inputMuted'))
            if pills['son'] and name in self.sources():
                self.pill(tr('{name}: {state}', name=name.upper(), state=tr('MUTED') if d.get('inputMuted') else tr('UNMUTED')),
                          RED if d.get('inputMuted') else GREEN)
        elif kind == 'InputVolumeChanged' and name in self.inputs:
            mul, asked = float(d.get('inputVolumeMul') or 0.0), self.pending.get(name)
            if asked and time.monotonic() < asked[1] and abs(asked[0] - mul) > 1e-6:
                return  # an older value, while the knob turns
            self.pending.pop(name, None)
            self.inputs[name]['mul'] = mul
        elif kind == 'SceneItemEnableStateChanged':
            scene, iid = d.get('sceneName'), d.get('sceneItemId')
            for (s, src), it in self.items.items():
                if s == scene and it[0] == iid:
                    it[1] = bool(d.get('sceneItemEnabled'))
        elif kind == 'SourceFilterEnableStateChanged':
            self.filters[(d.get('sourceName'), d.get('filterName'))] = bool(d.get('filterEnabled'))
        elif kind == 'MediaInputPlaybackStarted':
            self.playing.add(name)
        elif kind == 'MediaInputPlaybackEnded':
            self.playing.discard(name)
        elif kind == 'CurrentProfileChanged':
            self.profile = d.get('profileName', '')
            self.pill(tr('PROFILE: {name}', name=self.profile.upper()))
        elif kind == 'ExitStarted':
            log(tr('OBS is closing'))

    def output_changed(self, which, d):
        o, state = (self.stream if which == 'stream' else self.record), d.get('outputState', '')
        step, now = state.removeprefix(OUT), time.monotonic()
        o['active'], o['state'] = bool(d.get('outputActive')), state
        if step == 'STARTED':
            o.update(ms=0, at=now, paused=False, reconnecting=False)
        elif step in ('PAUSED', 'RECONNECTING'):
            o['ms'], o['at'] = int(self.elapsed(o)), now
            o['paused' if step == 'PAUSED' else 'reconnecting'] = True
        elif step in ('RESUMED', 'RECONNECTED'):
            o['at'] = now
            o['paused' if step == 'RESUMED' else 'reconnecting'] = False
        texts = {'stream': {'STARTED': (tr('LIVE'), RED), 'STOPPED': (tr('STREAM ENDED'), GREY),
                            'RECONNECTING': (tr('STREAM: RECONNECTING…'), AMBER),
                            'RECONNECTED': (tr('STREAM BACK'), GREEN)},
                 'record': {'STARTED': (tr('RECORDING STARTED'), RED),
                            'STOPPED': (tr('RECORDING ENDED'), GREY),
                            'PAUSED': (tr('RECORDING PAUSED'), AMBER),
                            'RESUMED': (tr('RECORDING RESUMED'), RED)}}[which]
        if self.settings['pills']['sorties'] and step in texts:
            self.pill(*texts[step])

    # --- actions (asked from handlers: run in the thread `obs`) --------------------------------
    def ask(self, name, act):
        """Run `act(obs)` in the thread `obs` (no request while a handler holds the lock)."""
        if self.status != 'ready':
            return self.pill(tr('OBS NOT CONNECTED'), GREY)
        self.actions.put((name, act))

    def toggle_output(self, which, held=True):
        """Start or stop the stream / the recording. From a page button, stopping needs the
        button held (setting maintenir_pour_arreter); commands are bindings made on purpose."""
        o, kind = (self.stream, 'Stream') if which == 'stream' else (self.record, 'Record')
        if o['state'].endswith(('STARTING', 'STOPPING')):
            return
        stop = o['active']
        if stop and not held:
            what = tr('THE STREAM') if which == 'stream' else tr('THE RECORDING')
            return self.pill(tr('HOLD TO STOP {what}', what=what), AMBER)
        self.ask(tr('stream') if which == 'stream' else tr('recording'),
                 lambda obs: obs.request(f'{"Stop" if stop else "Start"}{kind}'))

    def pause(self):
        if not self.record['active']:
            return self.pill(tr('NO RECORDING RUNNING'), GREY)
        self.ask('pause', lambda obs: obs.request('ToggleRecordPause'))

    def save_replay(self):
        if self.replay is None:
            return self.pill(tr('REPLAY DISABLED IN OBS (SETTINGS > OUTPUT)'), GREY)
        self.ask('replay', lambda obs: obs.request('SaveReplayBuffer' if self.replay else 'StartReplayBuffer'))

    def toggle_vcam(self):
        self.ask(tr('virtual camera'), lambda obs: obs.request('ToggleVirtualCam'))

    def transition(self):
        if not self.studio:
            return self.pill(tr('STUDIO MODE OFF'), GREY)
        self.ask('transition', lambda obs: obs.request('TriggerStudioModeTransition'))

    def toggle_studio(self):
        on = not self.studio
        self.ask(tr('studio mode'), lambda obs: obs.request('SetStudioModeEnabled', {'studioModeEnabled': on}))

    def show_scene(self, name, page=False):
        """Show scene `name`; in studio mode into the preview, and from the page a second click
        on the scene in the preview makes the transition."""
        if self.studio and page and name == self.preview:
            return self.transition()
        kind = 'SetCurrentPreviewScene' if self.studio else 'SetCurrentProgramScene'
        self.ask(tr('scene'), lambda obs: obs.request(kind, {'sceneName': name}))

    def step_scene(self, delta):
        if not self.scenes or not delta:
            return
        now = self.preview if self.studio else self.program
        i = self.scenes.index(now) if now in self.scenes else 0
        self.show_scene(self.scenes[clamp(i + delta, 0, len(self.scenes) - 1)])

    def toggle_mute(self, name):
        a = self.inputs.get(name)
        if not a:
            return self.pill(tr('{name}: NOT FOUND', name=name.upper()), GREY) if name else None
        muted = not a['muted']
        self.ask('son', lambda obs: obs.request('SetInputMute', {'inputName': name, 'inputMuted': muted}))

    def turn_volume(self, name, steps):
        """Move OBS's slider of `name` by `steps` × the step (% of the slider); the newest value
        only is sent when the knob turns faster than OBS answers."""
        a = self.inputs.get(name)
        if not a or not steps:
            return
        a['mul'] = multiplier(clamp(position(a['mul']) + steps * self.settings['step'] / 100, 0.0, 1.0))
        self.pending[name] = (a['mul'], time.monotonic() + 1.0)
        if name not in self.vol_queued:
            self.vol_queued.add(name)
            self.ask('volume', lambda obs: self.send_volume(obs, name))

    def send_volume(self, obs, name):
        with self.c.lock:
            self.vol_queued.discard(name)
            mul = (self.inputs.get(name) or {}).get('mul')
        if mul is not None:
            obs.request('SetInputVolume', {'inputName': name, 'inputVolumeMul': mul})

    def toggle_item(self, scene, src):
        self.ask('source', lambda obs: self.flip_item(obs, scene, src))

    def flip_item(self, obs, scene, src):
        iid = obs.request('GetSceneItemId', {'sceneName': scene, 'sourceName': src})['sceneItemId']
        on = obs.request('GetSceneItemEnabled', {'sceneName': scene, 'sceneItemId': iid}).get('sceneItemEnabled')
        obs.request('SetSceneItemEnabled', {'sceneName': scene, 'sceneItemId': iid, 'sceneItemEnabled': not on})
        with self.c.lock:
            self.items[(scene, src)] = [iid, not on]
        self.pill(tr('{name}: {state}', name=src.upper(), state=tr('HIDDEN') if on else tr('SHOWN')))

    def toggle_filter(self, src, flt):
        self.ask('filtre', lambda obs: self.flip_filter(obs, src, flt))

    def flip_filter(self, obs, src, flt):
        on = obs.request('GetSourceFilter', {'sourceName': src, 'filterName': flt}).get('filterEnabled')
        obs.request('SetSourceFilterEnabled', {'sourceName': src, 'filterName': flt, 'filterEnabled': not on})
        self.pill(tr('FILTER {name}: {state}', name=flt.upper(), state=tr('DISABLED') if on else tr('ENABLED')))

    def play_media(self, name):
        self.pill(tr('PLAYING: {name}', name=name.upper()))
        self.ask(tr('media'), lambda obs: obs.request('TriggerMediaInputAction', {
            'inputName': name, 'mediaAction': 'OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART'}))

    def set_profile(self, name):
        self.ask('profil', lambda obs: obs.request('SetCurrentProfile', {'profileName': name}))

    def set_collection(self, name):
        self.pill(f'COLLECTION: {name.upper()}')
        self.ask('collection', lambda obs: obs.request('SetCurrentSceneCollection',
                                                       {'sceneCollectionName': name}))

    # --- inputs on the page --------------------------------------------------------------------
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
            self.selected = clamp(self.selected + d, 0, max(0, len(self.scenes) - 1))
        elif k in (2, 3, 4):
            self.turn_volume(self.sources()[k - 2], d)
        self.refresh()

    def press_key(self, b, held):
        """Button `b` (1-4) of the page, clicked or held."""
        what, confirm = self.settings['keys'][b - 1], self.settings['confirm']
        act = {'direct': lambda: self.toggle_output('stream', held or not confirm),
               'enregistrement': lambda: self.toggle_output('record', held or not confirm),
               'pause': self.pause, 'replay': self.save_replay, 'caméra virtuelle': self.toggle_vcam,
               'transition': self.transition, 'mode studio': self.toggle_studio}.get(what)
        if act:
            act()

    def on_hold(self, ev):
        b = ev.get('button', '')
        if self.mine(ev) and self.status == 'ready' and b.startswith('button') and b[-1:] in '1234':
            self.press_key(int(b[-1]), True)
            self.refresh()

    def on_click(self, ev):
        if not self.mine(ev):
            return
        if self.status in ('refused', 'absent', 'off'):
            self.retry.set()  # try again now
            return
        if self.status != 'ready':
            return
        b = ev.get('button', '')
        if b == 'knob1' and self.scenes:
            self.show_scene(self.scenes[self.selected], page=True)
        elif b in ('knob2', 'knob3', 'knob4'):
            self.toggle_mute(self.sources()[int(b[-1]) - 2])
        elif b.startswith('button') and b[-1:] in '1234':
            self.press_key(int(b[-1]), False)
        self.refresh()


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'obs')
    st = Studio(client, args)
    client.run(tick=st.tick, fps=10)


if __name__ == '__main__':
    main()
