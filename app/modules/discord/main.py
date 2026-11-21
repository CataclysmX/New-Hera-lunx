#!/usr/bin/env python3
"""Module « Discord » de new hera-nux: the Hercules options for Discord, through the local RPC of
the official desktop client and YOUR Discord application (its ID and secret: « Données
sensibles » of the module's tab; guide at the top of discord.toml, docs/app/discord.md).

Page « Discord »: the voice channel (members, avatars, who speaks, muted / deafened), or the
favourite channels while you are in none.
  knob 1: choose a member (a channel) - click: mute them for you (join the channel);
  knob 2: volume of the chosen member (0-200 %) - click: back to 100 %;
  knob 3: my input (0-100 %) - click: mute;  knob 4: my output (0-200 %) - click: deafen;
  buttons 1-4: mute, deafen, push-to-talk / voice activity, leave the channel.
Commands (tab Commandes, any page): the same, one « Rejoindre / quitter » per favourite channel,
« Volume de … » / « Couper … » / « … à N % » per followed member (discord.toml).
Options: --show (page shown at start). Tokens: kept in the module's .env (owner only).
"""
import argparse
import json
import os
import queue
import threading
import time
import tomllib
import urllib.request

from hsm01 import Client, secrets
from hsm01 import widgets as W
from hsm01.i18n import tr

import discord_ipc as ipc

PAGE = 'discord'
HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'discord')
CDN = os.environ.get('HSM01_DISCORD_CDN') or 'https://cdn.discordapp.com'
BG, TILE, TEXT, GREY = '#0d0f14', '#181c26', '#e8eaf0', '#8a90a0'
BLURPLE, GREEN, RED = '#5865f2', '#23a55a', '#f23f43'
KEY, KEY_DOWN, KEY_ON = '#151a22', '#2c3646', '#5c2026'
COLS, ROWS, TILE_W, TILE_H, TOP = 4, 2, 116, 80, 36
# widgets « Membres du salon vocal » (pages of the user): id → (name, side by side); in a
# column a member is a line LIST_W × ROW_H, in a row a cell CELL_W × ROW_H
LISTS = {'membres': (tr('Voice channel members, in a column'), False),
         'membres_ligne': (tr('Voice channel members, in a row'), True)}
LIST_W, CELL_W, ROW_H = 220, 150, 40
PER_PAGE = COLS * ROWS
RETRY = 5.0
CHANNEL_EVENTS = ('VOICE_STATE_CREATE', 'VOICE_STATE_UPDATE', 'VOICE_STATE_DELETE',
                  'SPEAKING_START', 'SPEAKING_STOP')
TOKENS = ('DISCORD_ACCESS_TOKEN', 'DISCORD_ACCESS_EXPIRES', 'DISCORD_REFRESH_TOKEN')
# names of the members met in voice channels (the settings form offers them)
SEEN = os.path.join(CACHE, 'membres.json')


class Refused(Exception):
    """Discord did not authorize the application (its own words in the message)."""


def log(text):
    """A line in the Journal and in ~/.cache/hsm01/discord/journal.log (never a token)."""
    print(f'Discord: {text}', flush=True)
    try:
        os.makedirs(CACHE, exist_ok=True)
        with open(os.path.join(CACHE, 'journal.log'), 'a', encoding='utf-8') as f:
            f.write(f'{time.strftime("%Y-%m-%d %H:%M:%S")} {text}\n')
    except OSError:
        pass


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def load_settings(path):
    """discord.toml: redirect address, knob step, favourite channels, followed members."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        print(f'{path}: {e}', flush=True)
        data = {}
    favourites = [{'server': str(s.get('serveur', '')).strip(), 'name': str(s['salon']).strip(),
                   'id': None} for s in data.get('salon', []) if s.get('salon')]
    members = [(str(m['nom']).strip(), m.get('niveau')) for m in data.get('membre', [])
               if m.get('nom')]
    return {'redirect': str(data.get('redirection') or 'http://localhost'),
            'step': clamp(int(data.get('pas', 2)), 1, 50),
            'favourites': favourites, 'members': members,
            'pill_join': bool(data.get('pilule_arrivee', True)),
            'pill_leave': bool(data.get('pilule_depart', False))}


def member(vs):
    """A voice state of the RPC (voice_states[], VOICE_STATE_*) as one member of the channel."""
    u, st = vs.get('user') or {}, vs.get('voice_state') or {}
    names = {n.casefold() for n in (vs.get('nick'), u.get('global_name'), u.get('username')) if n}
    return {'id': u.get('id'), 'name': vs.get('nick') or u.get('global_name') or u.get('username')
            or '?', 'names': names, 'avatar': u.get('avatar'),
            'discriminator': u.get('discriminator') or '0',
            'volume': round(vs.get('volume', 100)), 'mute': bool(vs.get('mute')),
            'self_mute': bool(st.get('self_mute') or st.get('mute')),
            'deaf': bool(st.get('self_deaf') or st.get('deaf'))}


class Discord:
    def __init__(self, client, args):
        self.c, self.args = client, args
        self.settings = load_settings(os.path.join(HERE, 'discord.toml'))
        self.favourites = self.settings['favourites']
        self.client_id = (secrets.get('DISCORD_CLIENT_ID') or '').strip()
        self.secret = (secrets.get('DISCORD_CLIENT_SECRET') or '').strip()
        self.rpc = None
        self.status, self.message = 'start', ''
        self.me = None
        self.voice = {'mute': False, 'deaf': False, 'input': 100, 'output': 100, 'ptt': False}
        self.channel, self.members, self.speaking = None, [], set()
        self.selected, self.down = 0, set()
        self.guilds = {}  # id -> name
        self.sent, self.states, self.avatars, self.leds_sent = {}, {}, {}, None
        self.lists = {}  # members widget id → (its members, fields sent)
        self.seen = set()
        try:
            with open(SEEN, encoding='utf-8') as f:
                self.seen = {str(n) for n in json.load(f)}
        except (OSError, ValueError):
            pass
        self.retry = threading.Event()
        client.page(PAGE, self.widgets(), name='Discord', background=BG, show=args.show)
        if self.seen:
            client.choices(membres=sorted(self.seen, key=str.casefold))
        with client.lock:
            client.on('turn', self.on_turn)
            client.on('click', self.on_click)
            client.on('press', lambda ev: self.key(ev, True))
            client.on('release', lambda ev: self.key(ev, False))
            self.offer_commands()
            self.refresh()
        threading.Thread(target=self.connection, name='discord', daemon=True).start()

    # --- the page --------------------------------------------------------------------------
    @staticmethod
    def widgets():
        out = [W.icon('logo', 8, 5, 24, 24, 'discord'),
               W.text('title', 40, 3, 262, 28, 'Discord', size=17, bold=True, align='left',
                      color=TEXT),
               W.text('where', 304, 7, 168, 22, '', size=13, align='right', color=GREY)]
        for i in range(PER_PAGE):
            x, y = 2 + (i % COLS) * 120, TOP + (i // COLS) * (TILE_H + 2)
            out += [W.rect(f's{i}', x, y, TILE_W, TILE_H, BG, radius=12),  # selection
                    W.rect(f't{i}', x + 2, y + 2, TILE_W - 4, TILE_H - 4, BG, radius=10),
                    W.rect(f'r{i}', x + 33, y + 4, 50, 50, BG, radius=25),  # speaking ring
                    W.image(f'a{i}', x + 36, y + 7, 44, 44, '', radius=22),  # avatar
                    W.icon(f'c{i}', x + 42, y + 13, 32, 32, ''),  # channel
                    W.icon(f'm{i}', x + 80, y + 38, 18, 18, '', tint=RED),  # muted, deafened
                    W.text(f'n{i}', x + 4, y + 54, TILE_W - 8, 16, '', size=13, bold=True,
                           color=TEXT),
                    W.text(f'u{i}', x + 4, y + 68, TILE_W - 8, 11, '', size=10, color=GREY)]
        # over the (empty) tiles: what to do while not connected
        out += [W.text('msg1', 16, 84, 448, 30, '', size=17, color=TEXT),
                W.text('msg2', 16, 120, 448, 24, '', size=13, color=GREY)]
        edge = W.frame(border='#2a3240', width=1.0, radius=10, padding=0)
        for k in range(4):
            cx = 60 + k * 120
            out += [W.text(f'k{k + 1}', k * 120, 201, 120, 16, '', size=12, color=GREY),
                    W.rect(f'b{k + 1}', cx - 44, 222, 88, 46, KEY, radius=10, frame=edge),
                    W.icon(f'bi{k + 1}', cx - 12, 225, 24, 24, '', tint=TEXT),
                    W.text(f'bt{k + 1}', cx - 44, 251, 88, 14, '', size=11, color=GREY)]
        return out

    def header(self):
        """(title, top right, message, its second line) for the state."""
        s, ch = self.status, self.channel
        if s == 'ready' and ch:
            pages = (len(self.members) + PER_PAGE - 1) // PER_PAGE
            more = f' · {self.selected // PER_PAGE + 1}/{pages}' if pages > 1 else ''
            return ch['name'], ch['guild'] + more, '', ''
        if s == 'ready' and self.favourites:
            return tr('Not in a voice channel'), tr('favourite channels'), '', ''
        return {
            'ready': (tr('Not in a voice channel'), '', tr('No favourite channel'),
                      tr('discord.toml: [[salon]] serveur = … salon = …')),
            'config': ('Discord', tr('to set up'), tr('ID and secret of your Discord application'),
                       tr('window, "Discord" tab (guide at the top of discord.toml)')),
            'absent': ('Discord', '', tr('Discord is not running'),
                       tr('official client (not Vesktop); new try every 5 s')),
            'auth': ('Discord', '', tr('Allow the request in Discord'),
                     tr('"Authorize" window of the Discord client')),
            'refused': ('Discord', tr('click: ask again'), tr('Authorization refused by Discord'),
                        self.message),
            'error': ('Discord', tr('click: try again'), tr('Error'), self.message),
        }.get(s, ('Discord', '', tr('Connecting to Discord…'), ''))

    def items(self):
        """The tiles: members of the channel, else favourite channels (once connected)."""
        if self.status != 'ready':
            return []
        return self.members if self.channel else self.favourites

    def tile(self, it, selected):
        """Fields of a tile's widgets (`it` None: empty)."""
        bg = BG if it is None else TILE
        f = {'s': {'color': BLURPLE if selected else BG}, 't': {'color': bg},
             'r': {'color': bg}, 'a': {'path': ''}, 'c': {'name': ''}, 'm': {'name': ''},
             'n': {'text': '', 'color': TEXT}, 'u': {'text': ''}}
        if it is None:
            return f
        if self.channel:  # a member
            me = it['id'] == self.me
            f['r']['color'] = GREEN if it['id'] in self.speaking else TILE
            f['a']['path'] = self.avatar(it)
            f['m']['name'] = ('audio-volume-muted' if it['deaf'] else
                              'microphone-sensitivity-muted' if it['self_mute'] else '')
            f['n'] = {'text': it['name'], 'color': RED if it['mute'] else TEXT}
            f['u']['text'] = (tr('you') if me else tr('muted for you') if it['mute']
                              else f'{it["volume"]} %')
        else:  # a favourite channel
            f['c']['name'] = 'discord'
            f['n']['text'] = it['name']
            f['u']['text'] = it['server'] if it['id'] else 'introuvable'
        return f

    def refresh(self):
        """Send what changed on the page, the LEDs and the commands' states."""
        want = {}
        title, where, m1, m2 = self.header()
        for i, text in (('title', title), ('where', where), ('msg1', m1), ('msg2', m2)):
            want[i] = {'text': text}
        items = self.items()
        self.selected = clamp(self.selected, 0, max(0, len(items) - 1))
        first = self.selected // PER_PAGE * PER_PAGE
        for i in range(PER_PAGE):
            n = first + i
            it = items[n] if n < len(items) else None
            for part, fields in self.tile(it, it is not None and n == self.selected).items():
                want[f'{part}{i}'] = fields
        want.update(self.controls())
        changes = {}
        for wid, fields in want.items():
            old = self.sent.setdefault(wid, {})
            diff = {k: v for k, v in fields.items() if old.get(k) != v}
            if diff:
                old.update(diff)
                changes[wid] = diff
        self.c.update(PAGE, changes)
        v, ready = self.voice, self.status == 'ready'
        leds = ([100 if v['mute'] else 15, 100 if v['deaf'] else 15, 100 if v['ptt'] else 15,
                 60 if self.channel else 0] if ready else [0, 0, 0, 0])
        if leds != self.leds_sent:
            self.leds_sent = leds
            self.c.leds(PAGE, leds)
        self.push_states()
        self.refresh_list()

    # --- the members widgets ----------------------------------------------------------------
    @staticmethod
    def list_widgets(n, side_by_side):
        """A member a line (column), or a cell after the other (row): ring, avatar, name, volume,
        muted icon."""
        text_w, icon_x = (74, 126) if side_by_side else (146, 198)
        out = []
        for i in range(n):
            x, y = (i * CELL_W, 0) if side_by_side else (0, i * ROW_H)
            out += [W.rect(f'lr{i}', x + 4, y + 2, 36, 36, BG, radius=18),  # speaking ring
                    W.image(f'la{i}', x + 6, y + 4, 32, 32, '', radius=16),
                    W.text(f'ln{i}', x + 48, y + 3, text_w, 20, '', size=14, bold=True,
                           align='left', color=TEXT),
                    W.text(f'lu{i}', x + 48, y + 22, text_w, 14, '', size=10, align='left',
                           color=GREY),
                    W.icon(f'lm{i}', x + icon_x, y + 12, 16, 16, '', tint=RED)]
        return out

    def list_rows(self):
        """Rows of the members widget: the members of my channel, else one line saying what is
        going on (not connected, in no channel…)."""
        rows = [{'path': self.avatar(m), 'name': m['name'], 'color': RED if m['mute'] else TEXT,
                 'sub': (tr('you') if m['id'] == self.me else tr('muted for you') if m['mute']
                         else f'{m["volume"]} %'),
                 'ring': GREEN if m['id'] in self.speaking else BG,
                 'icon': ('audio-volume-muted' if m['deaf'] else
                          'microphone-sensitivity-muted' if m['self_mute'] else '')}
                for m in (self.members if self.status == 'ready' and self.channel else [])]
        if not rows:
            title, _, m1, _ = self.header()
            rows = [{'path': '', 'name': m1 or title, 'color': GREY, 'sub': '', 'ring': BG,
                     'icon': ''}]
        return rows

    def refresh_list(self):
        """The « Membres du salon vocal » widgets (column, row): made again when their size
        changes (members come and go), otherwise only what changed is sent."""
        rows = self.list_rows()
        for list_id, (name, side_by_side) in LISTS.items():
            n, sent = self.lists.get(list_id, (0, {}))
            if len(rows) != n:
                n, sent = len(rows), {}
                w, h = (n * CELL_W, ROW_H) if side_by_side else (LIST_W, n * ROW_H)
                self.c.widget(list_id, w, h, self.list_widgets(n, side_by_side), name=name,
                              background=BG)
            self.lists[list_id] = (n, sent)
            changes = {}
            for i, r in enumerate(rows):
                for wid, fields in ((f'lr{i}', {'color': r['ring']}),
                                    (f'la{i}', {'path': r['path']}),
                                    (f'ln{i}', {'text': r['name'], 'color': r['color']}),
                                    (f'lu{i}', {'text': r['sub']}),
                                    (f'lm{i}', {'name': r['icon']})):
                    if sent.get(wid) != fields:
                        sent[wid] = fields
                        changes[wid] = fields
            self.c.update(list_id, changes)

    def controls(self):
        """Knob labels and the keys above buttons 1-4."""
        v, ready, ch = self.voice, self.status == 'ready', self.channel
        m = self.chosen()
        out = {'k1': {'text': (tr('member') if ch else tr('channel')) if ready else ''},
               'k2': {'text': ('' if not (ready and m) else tr('knob 1: a member')
                               if m['id'] == self.me else tr('volume {pct} %', pct=m['volume']))},
               'k3': {'text': tr('input {pct} %', pct=v['input']) if ready else ''},
               'k4': {'text': tr('output {pct} %', pct=v['output']) if ready else ''}}
        keys = [('microphone-sensitivity-muted', tr('mic muted'), v['mute'])
                if v['mute'] else ('audio-input-microphone', tr('mic'), False),
                ('audio-volume-muted', 'sourd', True) if v['deaf']
                else ('audio-headphones', 'son', False),
                ('input-keyboard', 'push-to-talk', False) if v['ptt']
                else ('audio-input-microphone', 'voix', False),
                ('call-stop', 'quitter', False)]
        for k, (icon, label, on) in enumerate(keys, 1):
            usable = ready and (k != 4 or ch is not None)
            color = KEY_DOWN if f'button{k}' in self.down else KEY_ON if on and ready else KEY
            out[f'b{k}'] = {'color': color}
            out[f'bi{k}'] = {'name': icon if usable else ''}
            out[f'bt{k}'] = {'text': label if usable else ''}
        return out

    def avatar(self, m):
        """Local PNG of a member's avatar (downloaded once, in the background), else ''."""
        if not m.get('id'):
            return ''
        key = f'{m["id"]}_{m["avatar"] or "défaut"}'
        path = os.path.join(CACHE, key + '.png')
        if os.path.exists(path):
            return path
        if key not in self.avatars:
            self.avatars[key] = None
            if m['avatar']:
                url = f'{CDN}/avatars/{m["id"]}/{m["avatar"]}.png?size=64'
            else:
                d = m['discriminator']
                n = (int(m['id']) >> 22) % 6 if d in ('', '0') else int(d) % 5
                url = f'{CDN}/embed/avatars/{n}.png'
            threading.Thread(target=self.download, args=(url, path), daemon=True).start()
        return ''

    def download(self, url, path):
        try:
            os.makedirs(CACHE, exist_ok=True)
            req = urllib.request.Request(url, headers={'User-Agent': ipc.USER_AGENT})
            with urllib.request.urlopen(req, timeout=8) as r:
                data = r.read(1 << 20)
            if not data.startswith(b'\x89PNG'):
                raise ValueError(tr('not a PNG image'))
            with open(path + '.part', 'wb') as f:
                f.write(data)
            os.replace(path + '.part', path)
        except (OSError, ValueError) as e:
            print(f'avatar: {e}', flush=True)
            return
        with self.c.lock:
            self.refresh()

    # --- connection to Discord ------------------------------------------------------------
    def set_status(self, status, message=''):
        with self.c.lock:
            self.status, self.message = status, message
            self.refresh()

    def wait(self, seconds=None):
        """Until `seconds` pass or a click on the page asks to try again."""
        self.retry.wait(seconds)
        self.retry.clear()

    def connection(self):
        """Connect, authorize, follow Discord's events; again when Discord comes back."""
        while not self.c.closed:
            if not (self.client_id and self.secret):
                return self.set_status('config')  # the window restarts the module once typed
            events = queue.Queue()
            rpc = self.connect(events)
            if rpc is None:
                continue
            try:
                self.session(rpc, events)
            except Refused as e:
                log(tr('authorization: {e}', e=e))
                self.set_status('refused', str(e))
                self.wait()
            except ipc.AuthError as e:
                log(tr('token refused by discord.com: {e}', e=e))
                self.set_status('error', tr('discord.com refuses the token: {e}', e=e))
                self.wait()
            except ipc.RpcError as e:
                log(f'Discord: {e}')
                self.set_status('error', str(e))
                self.wait(None if e.code else RETRY)
            finally:
                rpc.close()
                with self.c.lock:
                    self.rpc, self.channel, self.members = None, None, []
                    self.speaking.clear()

    def connect(self, events):
        """A connection to Discord (its events into `events`), else None after waiting."""
        try:
            self.set_status('connecting')
            return ipc.Rpc(self.client_id, lambda e, d: events.put((e, d)))
        except FileNotFoundError:
            self.set_status('absent')
            self.wait(RETRY)
        except ipc.RpcError as e:
            log(tr('connection refused by Discord: {e}', e=e))
            if e.code:  # refused (application ID…): not again and again
                self.set_status('error', tr('Discord refuses the connection: {e}', e=e))
                self.wait(60)
            else:  # no answer (Discord busy, just after an authorization…): soon again
                self.set_status('absent')
                self.wait(RETRY)
        return None

    def session(self, rpc, events):
        """Authorized, set up, then Discord's events until the connection closes."""
        self.me = (rpc.ready.get('user') or {}).get('id')
        self.login(rpc)
        with self.c.lock:
            self.rpc = rpc
            self.setup(rpc)
            self.status = 'ready'
            self.refresh()
        log(tr('connected'))
        threading.Thread(target=self.publish_choices, args=(rpc,), daemon=True).start()
        since = time.monotonic()
        log(tr('connection closed ({why})', why=self.follow(rpc, events)))
        self.set_status('absent')
        if time.monotonic() - since < RETRY:
            self.wait(2)  # closed at once (Discord resets after an authorization)

    def login(self, rpc):
        """AUTHENTICATE with the token kept in .env (refreshed if needed), else ask Discord
        (AUTHORIZE: its window), exchange the code for a token and keep it."""
        tok = self.saved_token()
        if tok:
            try:
                return rpc.request('AUTHENTICATE', {'access_token': tok})
            except ipc.RpcError as e:
                log(tr('kept token refused ({e}): new authorization', e=e))
                self.forget_token()
        self.set_status('auth')
        log(tr('authorization request (scopes {scopes})', scopes=' '.join(ipc.SCOPES)))
        # no redirect_uri here (refused in the RPC flow): discord.com takes the application's,
        # which must then be its only one (« Missing "redirect_uri" » otherwise)
        try:
            reply = rpc.request('AUTHORIZE', {'client_id': self.client_id,
                                              'scopes': ipc.SCOPES}, timeout=300)
        except ipc.RpcError as e:
            if 'redirect_uri' in str(e):
                log(tr('authorization: {e}', e=e))
                raise Refused(tr('Discord portal, OAuth2, Redirects: a single address, {address}, then Save Changes', address=self.settings['redirect'])) from None
            raise Refused(str(e)) from None
        if not reply.get('code'):
            raise Refused(tr('answer without a code (fields: {fields})', fields=', '.join(sorted(reply)) or tr('none')))
        log(tr('authorized: exchanging the code for a token'))
        t = ipc.token(self.client_id, self.secret, code=reply['code'],
                      redirect_uri=self.settings['redirect'])
        self.keep_token(t)
        log(tr('token obtained (scopes: {scopes})', scopes=t.get('scope', '?')))
        return rpc.request('AUTHENTICATE', {'access_token': t['access_token']})

    def saved_token(self):
        access = secrets.get('DISCORD_ACCESS_TOKEN')
        expires = float(secrets.get('DISCORD_ACCESS_EXPIRES') or 0)
        if access and expires - time.time() > 300:
            return access
        refresh = secrets.get('DISCORD_REFRESH_TOKEN')
        if not refresh:
            return None
        try:
            t = ipc.token(self.client_id, self.secret, refresh_token=refresh)
        except ipc.AuthError as e:
            print(tr('token renewal refused ({e}): new authorization', e=e), flush=True)
            self.forget_token()
            return None
        self.keep_token(t)
        return t['access_token']

    @staticmethod
    def keep_token(t):
        secrets.save('DISCORD_ACCESS_TOKEN', t['access_token'])
        secrets.save('DISCORD_ACCESS_EXPIRES', str(int(time.time() + t.get('expires_in', 0))))
        if t.get('refresh_token'):
            secrets.save('DISCORD_REFRESH_TOKEN', t['refresh_token'])

    @staticmethod
    def forget_token():
        for name in TOKENS:
            secrets.save(name, '')

    def setup(self, rpc):
        """Once authenticated: events, voice settings, favourites, the channel I am in."""
        rpc.subscribe('VOICE_SETTINGS_UPDATE')
        rpc.subscribe('VOICE_CHANNEL_SELECT')
        self.apply_voice(rpc.request('GET_VOICE_SETTINGS'))
        self.resolve_favourites(rpc)
        self.select_channel(rpc, rpc.request('GET_SELECTED_VOICE_CHANNEL'))

    def follow(self, rpc, events):
        """Discord's events until it goes away; returns why."""
        while True:
            evt, data = events.get()
            if evt == 'CLOSED':
                return data.get('message')
            with self.c.lock:
                try:
                    self.on_event(rpc, evt, data)
                except ipc.RpcError as e:
                    print(f'{evt}: {e}', flush=True)
                self.refresh()

    def on_event(self, rpc, evt, data):
        if evt == 'VOICE_SETTINGS_UPDATE':
            self.apply_voice(data)
        elif evt == 'VOICE_CHANNEL_SELECT':
            cid = data.get('channel_id')
            if cid != (self.channel or {}).get('id'):
                ch = rpc.request('GET_CHANNEL', {'channel_id': cid}) if cid else None
                self.select_channel(rpc, ch)
        elif evt in ('VOICE_STATE_CREATE', 'VOICE_STATE_UPDATE'):
            m = member(data)
            i = next((i for i, o in enumerate(self.members) if o['id'] == m['id']), None)
            if i is None:
                self.members.append(m)
                self.remember([m])
                if m['id'] != self.me and self.settings['pill_join']:
                    self.c.pill(tr('{name} JOINED THE CHANNEL', name=m['name'].upper()), border=GREEN,
                                dot=GREEN, duration=3, app='Discord')
            else:
                self.members[i] = m
        elif evt == 'VOICE_STATE_DELETE':
            gone = (data.get('user') or {}).get('id')
            left = next((o for o in self.members if o['id'] == gone), None)
            self.members = [o for o in self.members if o['id'] != gone]
            self.speaking.discard(gone)
            if left and gone != self.me and self.settings['pill_leave']:
                self.c.pill(tr('{name} LEFT THE CHANNEL', name=left['name'].upper()), border=GREY, dot=GREY,
                            duration=3, app='Discord')
        elif evt == 'SPEAKING_START':
            self.speaking.add(data.get('user_id'))
        elif evt == 'SPEAKING_STOP':
            self.speaking.discard(data.get('user_id'))

    def apply_voice(self, d):
        v = self.voice
        for k in ('mute', 'deaf'):
            if k in d:
                v[k] = bool(d[k])
        for k in ('input', 'output'):
            if isinstance(d.get(k), dict) and 'volume' in d[k]:
                v[k] = round(d[k]['volume'])
        mode = (d.get('mode') or {}).get('type')
        if mode:
            v['ptt'] = mode == 'PUSH_TO_TALK'

    def select_channel(self, rpc, ch):
        """The voice channel I am in now (None: none): its members, its events."""
        old = self.channel
        if old:
            for e in CHANNEL_EVENTS:
                try:
                    rpc.unsubscribe(e, {'channel_id': old['id']})
                except ipc.RpcError:
                    pass
        self.speaking.clear()
        self.selected = 0
        if not ch:
            self.channel, self.members = None, []
            return
        gid = ch.get('guild_id')
        self.channel = {'id': ch['id'], 'name': ch.get('name') or tr('Call'),
                        'guild_id': gid, 'guild': self.guild_name(rpc, gid)}
        members = [member(v) for v in ch.get('voice_states') or []]
        self.members = sorted(members, key=lambda m: m['id'] != self.me)  # me first
        # chosen: the first one who is not me (knob 2 sets their volume)
        self.selected = next((i for i, m in enumerate(self.members) if m['id'] != self.me), 0)
        self.remember(self.members)
        for e in CHANNEL_EVENTS:
            rpc.subscribe(e, {'channel_id': ch['id']})

    def publish_choices(self, rpc):
        """The settings form's lists: my servers and the voice channels of each."""
        try:
            guilds = rpc.request('GET_GUILDS').get('guilds') or []
            lists = {'serveurs': sorted((g.get('name', '') for g in guilds), key=str.casefold)}
            for g in guilds:
                got = rpc.request('GET_CHANNELS', {'guild_id': g['id']}).get('channels') or []
                lists[f'salons:{g.get("name", "")}'] = [c.get('name', '') for c in got
                                                         if c.get('type') in (2, 13)]
                time.sleep(0.05)  # gently: one server after the other
            self.c.choices(**lists)
        except (ipc.RpcError, OSError) as e:
            log(tr('lists of servers and channels: {e}', e=e))

    def remember(self, members):
        """Names of the members met, offered by the form (« Membres suivis »), kept in the cache."""
        new = {m['name'] for m in members if m['id'] != self.me} - self.seen
        if not new:
            return
        self.seen |= new
        try:
            os.makedirs(CACHE, exist_ok=True)
            with open(SEEN, 'w', encoding='utf-8') as f:
                json.dump(sorted(self.seen, key=str.casefold), f, ensure_ascii=False)
        except OSError as e:
            log(tr('members met: {e}', e=e))
        self.c.choices(membres=sorted(self.seen, key=str.casefold))

    def guild_name(self, rpc, gid):
        if not gid:
            return tr('private call')
        if gid not in self.guilds:
            try:
                self.guilds[gid] = rpc.request('GET_GUILD', {'guild_id': gid}).get('name', '')
            except ipc.RpcError:
                return ''
        return self.guilds[gid]

    def resolve_favourites(self, rpc):
        """The IDs of the favourite channels, from their names (server / channel)."""
        if not self.favourites:
            return
        guilds = rpc.request('GET_GUILDS').get('guilds') or []
        self.guilds.update({g['id']: g.get('name', '') for g in guilds})
        by_name = {g.get('name', '').casefold(): g['id'] for g in guilds}
        channels = {}
        for f in self.favourites:
            gid = by_name.get(f['server'].casefold())
            if gid is None:
                print(tr('favourite server not found: "{server}" (servers: {servers})', server=f['server'], servers=', '.join(sorted(g.get('name', '') for g in guilds))), flush=True)
                continue
            if gid not in channels:
                got = rpc.request('GET_CHANNELS', {'guild_id': gid}).get('channels') or []
                channels[gid] = [c for c in got if c.get('type') in (2, 13)]  # voice, stage
            ch = next((c for c in channels[gid]
                       if c.get('name', '').casefold() == f['name'].casefold()), None)
            f['id'] = ch['id'] if ch else None
            if ch is None:
                names = ', '.join(c.get('name', '') for c in channels[gid])
                print(tr('favourite channel not found: "{server} / {channel}" (voice channels: {names})', server=f['server'], channel=f['name'], names=names), flush=True)

    # --- actions (with the client's lock: a handler, or refresh after) ---------------------
    def ask(self, cmd, args, what):
        """One command to Discord; None (and a word in the Journal) when it cannot."""
        if self.rpc is None or self.status != 'ready':
            return None
        try:
            return self.rpc.request(cmd, args)
        except ipc.RpcError as e:
            log(f'{what}: {e}')
            return None

    def set_voice(self, args, what):
        d = self.ask('SET_VOICE_SETTINGS', args, what)
        if d:
            self.apply_voice(d)

    def toggle(self, key):
        what = {'mute': tr('mic'), 'deaf': tr('deafen mode')}[key]
        self.set_voice({key: not self.voice[key]}, what)

    def toggle_mode(self):
        mode = 'VOICE_ACTIVITY' if self.voice['ptt'] else 'PUSH_TO_TALK'
        self.set_voice({'mode': {'type': mode}}, tr('voice mode'))

    def volume(self, key, steps):
        top = 100 if key == 'input' else 200
        new = clamp(self.voice[key] + steps * self.settings['step'], 0, top)
        if new != self.voice[key]:
            self.set_voice({key: {'volume': new}}, f'volume ({key})')

    def leave(self):
        if self.channel and self.ask('SELECT_VOICE_CHANNEL', {'channel_id': None}, 'quitter') \
                is not None:
            self.select_channel(self.rpc, None)

    def join(self, f):
        """Join favourite channel `f`, or leave it when I am in it."""
        if f['id'] is None and self.rpc is not None and self.status == 'ready':
            try:
                self.resolve_favourites(self.rpc)
            except ipc.RpcError as e:
                print(tr('favourite channels: {e}', e=e), flush=True)
        if f['id'] is None:
            return
        if self.channel and self.channel['id'] == f['id']:
            return self.leave()
        ch = self.ask('SELECT_VOICE_CHANNEL', {'channel_id': f['id'], 'force': True},
                      tr('join {name}', name=f['name']))
        if ch:
            self.select_channel(self.rpc, ch)

    def set_member(self, m, **fields):
        d = self.ask('SET_USER_VOICE_SETTINGS', dict(fields, user_id=m['id']), m['name'])
        if d:
            m['volume'] = round(d.get('volume', m['volume']))
            m['mute'] = bool(d.get('mute', m['mute']))

    def reset_volume(self, m):
        """Knob 2's click: the chosen member back to 100 %, said by a pill (even when already
        there); me chosen: how to choose someone."""
        if m['id'] == self.me:
            return self.c.pill(tr('KNOB 1: CHOOSE A MEMBER'), border=GREY, dot=GREY,
                               duration=2, app='Discord')
        self.set_member(m, volume=100)
        self.c.pill(f"{m['name'].upper()}: {m['volume']} %", border=BLURPLE, dot=BLURPLE,
                    duration=1.5, app='Discord')

    def member_volume(self, m, steps):
        new = clamp(m['volume'] + steps * self.settings['step'], 0, 200)
        if new != m['volume']:
            self.set_member(m, volume=new)

    def member_mute(self, m):
        if m['id'] == self.me:
            return self.toggle('mute')
        self.set_member(m, mute=not m['mute'])

    def chosen(self):
        """The member chosen with knob 1 (None: not in a channel)."""
        if self.channel and self.status == 'ready' and self.members:
            return self.members[clamp(self.selected, 0, len(self.members) - 1)]
        return None

    def followed(self, name):
        """A followed member (discord.toml) in my channel, by name; None when absent."""
        key = name.casefold()
        return next((m for m in self.members if key in m['names']), None)

    # --- commands --------------------------------------------------------------------------
    def offer_commands(self):
        cmd = self.c.command
        done = self.refresh
        cmd('mute', tr('Mute the mic'))(lambda ev: (self.toggle('mute'), done()))
        cmd('deafen', tr('Deafen'))(lambda ev: (self.toggle('deaf'), done()))
        cmd('input_volume', tr('My input (mic)'), kind='knob')(
            lambda ev: (self.volume('input', ev.get('delta', 0)), done()))
        cmd('output_volume', tr('My output (sound)'), kind='knob')(
            lambda ev: (self.volume('output', ev.get('delta', 0)), done()))
        cmd('disconnect', tr('Leave the voice channel'))(lambda ev: (self.leave(), done()))
        cmd('voice_mode', tr('Push-to-talk / voice activity'))(
            lambda ev: (self.toggle_mode(), done()))
        for f in self.favourites:
            cmd(f'salon:{f["server"]}/{f["name"]}', tr('Join / leave "{name}"', name=f['name']),
                description=f['server'])(lambda ev, f=f: (self.join(f), done()))
        for name, level in self.settings['members']:
            def on_member(fn, name=name):
                return lambda ev: (self.followed(name) and fn(self.followed(name), ev), done())
            cmd(f'volume:{name}', tr('Volume of "{name}"', name=name), kind='knob')(
                on_member(lambda m, ev: self.member_volume(m, ev.get('delta', 0))))
            cmd(f'couper:{name}', tr('Mute "{name}" (for you)', name=name))(
                on_member(lambda m, ev: self.member_mute(m)))
            if isinstance(level, (int, float)):
                lvl = clamp(int(level), 0, 200)
                cmd(f'niveau:{name}', tr('"{name}" at {level} %', name=name, level=lvl))(
                    on_member(lambda m, ev, lvl=lvl: self.set_member(m, volume=lvl)))

    def push_states(self):
        """What the app shows of each command (LEDs, knob widgets), only what changed."""
        v, ready, ch = self.voice, self.status == 'ready', self.channel
        st = {
            'mute': {'active': ready and v['mute'], 'text': tr('Mic muted') if v['mute']
                     else tr('Mic')},
            'deafen': {'active': ready and v['deaf'], 'text': tr('Deafened') if v['deaf'] else tr('Sound')},
            'input_volume': {'active': ready, 'value': v['input'] / 100,
                             'text': tr('Input {pct} %', pct=v['input'])},
            'output_volume': {'active': ready, 'value': v['output'] / 100,
                              'text': tr('Output {pct} %', pct=v['output'])},
            'disconnect': {'active': ready and ch is not None,
                           'text': ch['name'] if ch else tr('No channel')},
            'voice_mode': {'active': ready and v['ptt'],
                           'text': 'Push-to-talk' if v['ptt'] else tr('Voice activity')},
        }
        for f in self.favourites:
            here = bool(ch and f['id'] and ch['id'] == f['id'])
            st[f'salon:{f["server"]}/{f["name"]}'] = {
                'active': ready and here, 'text': f['name'] if f['id'] else f'{f["name"]} ?'}
        for name, level in self.settings['members']:
            m = self.followed(name) if ready else None
            st[f'volume:{name}'] = {'active': m is not None,
                                    'value': m['volume'] / 100 if m else 0.0,
                                    'text': f'{name} {m["volume"]} %' if m else tr('{name} absent', name=name)}
            st[f'couper:{name}'] = {'active': bool(m and m['mute']),
                                    'text': tr('{name} muted', name=name) if m and m['mute'] else name}
            if isinstance(level, (int, float)):
                lvl = clamp(int(level), 0, 200)
                st[f'niveau:{name}'] = {'active': bool(m and m['volume'] == lvl),
                                        'text': f'{name} {lvl} %'}
        for cid, s in st.items():
            if self.states.get(cid) != s:
                self.states[cid] = s
                self.c.command_state(cid, **s)

    # --- inputs on the page ---------------------------------------------------------------
    @staticmethod
    def mine(ev):
        return ev.get('mine') and ev.get('page') == PAGE

    def key(self, ev, down):
        b = ev.get('button', '')
        if self.mine(ev) and b.startswith('button'):
            (self.down.add if down else self.down.discard)(b)
            self.refresh()

    def on_turn(self, ev):
        if not self.mine(ev):
            return
        k, d = ev.get('knob'), ev.get('delta', 0)
        m = self.chosen()
        if k == 1:
            self.selected = clamp(self.selected + d, 0, max(0, len(self.items()) - 1))
        elif k == 2 and m and m['id'] != self.me:
            before = m['volume']
            self.member_volume(m, d)
            if m['volume'] == before:
                self.knob2_note(tr('volume stayed at {before} % ({steps} step(s))', before=before, steps=f'{d:+d}'))
        elif k == 2:
            self.knob2_note(tr('no effect:') + ' ' + (tr('you are chosen') if m else
                                               tr('no member chosen (state {state})', state=self.status)))
        elif k in (3, 4):
            self.volume('input' if k == 3 else 'output', d)
        self.refresh()

    def knob2_note(self, why):
        """Why knob 2 changed nothing, in the journal (once every 2 s at most)."""
        now = time.monotonic()
        if now - getattr(self, 'knob2_noted', 0.0) >= 2.0:
            self.knob2_noted = now
            log(tr('knob 2: {why}', why=why))

    def on_click(self, ev):
        if not self.mine(ev):
            return
        if self.status in ('refused', 'error', 'absent'):
            self.retry.set()  # try again now
            return
        b, m, items = ev.get('button'), self.chosen(), self.items()
        if b == 'knob1' and m:
            self.member_mute(m)
        elif b == 'knob1' and items and not self.channel:
            self.join(items[clamp(self.selected, 0, len(items) - 1)])
        elif b == 'knob2' and m:
            self.reset_volume(m)
        elif b in ('knob3', 'button1'):
            self.toggle('mute')
        elif b in ('knob4', 'button2'):
            self.toggle('deaf')
        elif b == 'button3':
            self.toggle_mode()
        elif b == 'button4':
            self.leave()
        self.refresh()


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'discord')
    Discord(client, args)
    client.run()


if __name__ == '__main__':
    main()
