#!/usr/bin/env python3
"""Module « Lecteur média » de new hera-nux: page « Média » (cover, title, artist, album,
progress) of the player that plays (Spotify, browser, VLC... - the app follows them, MPRIS).

On the page: button 1 = previous, button 2 (or knob 1 click) = play / pause, button 3 = next,
knob 1 = seek (5 s per step), knob 4 = the player's volume.
Options: --alert (an alert for each new track; it wakes the screen up as the app's setting says,
not with --no-wake), --show (page shown at start), --name NAME.
"""
import argparse
import hashlib
import os
import threading
import time
import urllib.parse
import urllib.request


from hsm01 import Client
from hsm01 import widgets as W
from hsm01.i18n import tr

PAGE = 'media'
BG, TEXT, GREY, ACCENT, TRACK = '#0b1010', '#e8eeee', '#8a9696', '#00b4a8', '#1c2626'
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'art')
BAR_X, BAR_W = 198, 270
KEY, KEY_DOWN = '#141c1c', '#2c4242'
# keys under buttons 1-3: previous, play / pause (changes with the state), next
KEYS = [('k1', 'media-skip-backward'), ('k2', 'media-playback-start'),
        ('k3', 'media-skip-forward')]


def clock(s):
    s = max(0, int(s))
    return f'{s // 60}:{s % 60:02d}'


class Media:
    def __init__(self, client, args):
        self.c, self.args = client, args
        self.state, self.read_at = {}, time.monotonic()
        self.track = None  # (title, artist) of the last alert
        self.art = {}  # cover URL -> local file (None while downloading)
        client.page(PAGE, self.widgets(), name=args.name, background=BG, show=args.show)
        with client.lock:
            client.on('media', self.on_media)
            client.on('click', self.on_click)
            client.on('turn', self.on_turn)
            client.on('press', lambda ev: self.key(ev, True))
            client.on('release', lambda ev: self.key(ev, False))
            self.on_media(client.call({'type': 'media'}))

    @staticmethod
    def widgets():
        return [
            W.text('player', 12, 4, 456, 26, tr('Media'), size=15, align='left', color=GREY),
            W.rect('cover_bg', 16, 40, 168, 168, '#141c1c', radius=10),
            W.icon('cover', 20, 44, 160, 160, 'multimedia-player'),
            W.text('title', BAR_X, 48, BAR_W, 32, '—', size=21, bold=True, align='left',
                   color=TEXT),
            W.text('artist', BAR_X, 86, BAR_W, 26, '', size=16, align='left', color='#c8d2d2'),
            W.text('album', BAR_X, 116, BAR_W, 24, '', size=14, align='left', color=GREY),
            W.rect('bar_bg', BAR_X, 162, BAR_W, 6, TRACK, radius=3),
            W.rect('bar', BAR_X, 162, 1, 6, ACCENT, radius=3),
            W.text('time', BAR_X, 174, BAR_W, 22, '', size=13, align='left', color=GREY),
            W.separator('sep', 0, 214, 480, 6, color='#283434', fade=True),
            *Media.keys(),
            # knob 4 (under the 4th column): the player's volume
            W.icon('k4', 376, 230, 28, 28, 'audio-volume-high', tint=GREY),
            W.text('k4t', 406, 230, 66, 28, 'volume', size=13, align='left', color=GREY),
        ]

    @staticmethod
    def keys():
        """A key under each of buttons 1-3 (one column of 120 px per button): previous,
        play / pause, next."""
        out = []
        for n, (i, name) in enumerate(KEYS):
            cx = 60 + n * 120
            edge = W.frame(border='#2c3838', width=1.0, radius=10, padding=0)
            out += [W.rect(f'{i}bg', cx - 44, 224, 88, 44, KEY, radius=10, frame=edge),
                    W.icon(i, cx - 17, 229, 34, 34, name, tint=TEXT)]
        return out

    def key(self, ev, down):
        """A key lights up while its button is held."""
        b = ev.get('button', '')
        if ev.get('mine') and ev.get('page') == PAGE and b in ('button1', 'button2', 'button3'):
            self.c.update(PAGE, {f'k{b[-1]}bg': {'color': KEY_DOWN if down else KEY}})

    # --- state ---------------------------------------------------------------------------
    def on_media(self, m):
        self.state, self.read_at = m, time.monotonic()
        if not m.get('player'):
            self.c.update(PAGE, {'player': {'text': tr('Media · no player')},
                                 'title': {'text': '—'}, 'artist': {'text': ''},
                                 'album': {'text': ''}, 'time': {'text': ''},
                                 'cover': {'name': 'multimedia-player'}})
            self.c.leds(PAGE, [0, 0, 0, 0])
            return
        playing = m.get('status') == 'Playing'
        status = {'Playing': tr('playing'), 'Paused': tr('paused')}.get(m.get('status'), tr('stopped'))
        self.c.update(PAGE, {
            'player': {'text': f'{m["player"]} · {status}'},
            'title': {'text': m.get('title') or '—'},
            'artist': {'text': m.get('artist') or ''},
            'album': {'text': m.get('album') or ''},
            'cover': {'name': self.cover(m.get('art') or '')},
            'k2': {'name': 'media-playback-pause' if playing else 'media-playback-start'},
        })
        self.c.leds(PAGE, [40, 100 if playing else 40, 40, 0])
        self.progress()
        track = (m.get('title'), m.get('artist'))
        if self.args.alert and playing and track != self.track and m.get('title'):
            if self.track is not None:
                cover = self.cover(m.get('art') or '')
                extra = {'image': cover} if cover.startswith('/') else {'icon': 'multimedia-player'}
                if self.args.no_wake:  # otherwise: the app's setting (Appareil → Notifications)
                    extra['wake'] = False
                self.c.alert(m['title'], m.get('artist') or '', app=m['player'], duration=4,
                             **extra)
            self.track = track
        elif self.track is None:
            self.track = track

    def cover(self, url):
        """Local file of the cover (downloaded in the background), else a theme icon."""
        if url.startswith('file://'):
            return urllib.parse.unquote(url[len('file://'):])
        if not url.startswith(('http://', 'https://')):
            return 'multimedia-player'
        path = os.path.join(CACHE, hashlib.sha1(url.encode()).hexdigest() + '.img')
        if os.path.exists(path):
            return path
        if url not in self.art:
            self.art[url] = None
            threading.Thread(target=self.download, args=(url, path), daemon=True).start()
        return 'multimedia-player'

    def download(self, url, path):
        try:
            os.makedirs(CACHE, exist_ok=True)
            with urllib.request.urlopen(url, timeout=8) as r:
                data = r.read(4 << 20)
            with open(path + '.part', 'wb') as f:
                f.write(data)
            os.replace(path + '.part', path)
        except OSError as e:
            print(tr('cover: {e}', e=e), flush=True)
            return
        if self.state.get('art') == url:
            self.c.update(PAGE, {'cover': {'name': path}})

    def progress(self):
        m = self.state
        length, pos = m.get('length') or 0, m.get('position') or 0
        if m.get('status') == 'Playing':
            pos += time.monotonic() - self.read_at
        frac = min(1.0, pos / length) if length else 0.0
        shown = (max(1, round(BAR_W * frac)), f'{clock(pos)} / {clock(length)}' if length else '')
        if shown != getattr(self, 'shown', None):
            self.shown = shown
            self.c.update(PAGE, {'bar': {'w': shown[0]}, 'time': {'text': shown[1]}})

    # --- inputs --------------------------------------------------------------------------
    def control(self, action, value=None):
        msg = {'type': 'media', 'action': action}
        if value is not None:
            msg['value'] = value
        self.c.post(msg)

    def on_click(self, ev):
        if not (ev.get('mine') and ev.get('page') == PAGE):
            return
        action = {'button1': 'previous', 'button2': 'play_pause', 'button3': 'next',
                  'knob1': 'play_pause'}.get(ev['button'])
        if action:
            self.control(action)

    def on_turn(self, ev):
        if not (ev.get('mine') and ev.get('page') == PAGE):
            return
        if ev['knob'] == 1:
            self.control('seek', 5.0 * ev['delta'])
        elif ev['knob'] == 4:
            self.control('volume', 0.02 * ev['delta'])


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--alert', action='store_true', help='alert at each new track')
    ap.add_argument('--no-wake', action='store_true',
                    help='these alerts do not wake the screen up (lost during standby)')
    ap.add_argument('--show', action='store_true', help='show the page at start')
    ap.add_argument('--name', default=tr('Media'), help='name of the page')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'media')
    media = Media(client, args)
    client.run(tick=lambda dt: media.progress(), fps=2)


if __name__ == '__main__':
    main()
