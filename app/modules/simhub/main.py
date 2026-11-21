#!/usr/bin/env python3
"""Module « SimHub » de new hera-nux: a game's telemetry in the format SimHub reads, on the same
dashboard as Motor Town (hsm01.dashboard): revs, gear, speed, pedals, fuel, boost, tyres, race;
shift lights on the LEDs; pills; widget « Tableau de bord ». Knob 1: the screens; buttons 1-4:
keys sent to the game (simhub.toml). Two sources (simhub.toml `source`):

- « flux du jeu » (by default): the game's « SimHub-compatible » / « Forza Data Out » UDP stream
  (hsm01.forza: 232, 311 or 324 bytes), sent to this computer on `port` (8888) — Motor Town,
  Forza, and the games copying that format; no SimHub program needed;
- « programme SimHub »: the SimHub program itself (Windows, or Wine / Proton), through its web
  API (Dash Studio server, port 8888): any game SimHub reads. Option: --show.
"""
import argparse
import os
import re
import threading
import time
import tomllib
import urllib.parse

from hsm01 import Client, forza, udp
from hsm01.dashboard import Dashboard
from hsm01.i18n import tr

import simhub_api as api

HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'simhub')
STREAM, PROGRAM = 'flux du jeu', 'programme SimHub'
GD = 'DataCorePlugin.GameData.'
# sample key -> SimHub's GameData property: read each time, then every fifth time
FAST = {'speed': 'SpeedKmh', 'rpm': 'Rpms', 'gear': 'Gear', 'throttle': 'Throttle',
        'brake': 'Brake', 'clutch': 'Clutch'}
SLOW = {'max_rpm': 'MaxRpm', 'fuel': 'Fuel', 'fuel_max': 'MaxFuel', 'coolant': 'WaterTemperature',
        'abs': 'ABSActive', 'tc': 'TCActive', 'limiter': 'PitLimiterOn', 'vehicle': 'CarModel',
        'lap': 'CurrentLap', 'laps': 'TotalLaps', 'position': 'Position', 'racers': 'OpponentsCount',
        'lap_time': 'CurrentLapTime', 'last': 'LastLapTime', 'best': 'BestLapTime',
        'tire_fl': 'TyreTemperatureFrontLeft', 'tire_fr': 'TyreTemperatureFrontRight',
        'tire_rl': 'TyreTemperatureRearLeft', 'tire_rr': 'TyreTemperatureRearRight',
        'brake_fl': 'BrakeTemperatureFrontLeft', 'brake_fr': 'BrakeTemperatureFrontRight',
        'brake_rl': 'BrakeTemperatureRearLeft', 'brake_rr': 'BrakeTemperatureRearRight'}
MISSING_RETRY = 60.0  # s before asking again for a property SimHub did not know
HOST = re.compile(r'^[A-Za-z0-9]([A-Za-z0-9.-]{0,252})$')  # an IPv4 address or a computer's name


def log(text):
    """A line in the Journal and in ~/.cache/hsm01/simhub/journal.log."""
    print(f'SimHub: {text}', flush=True)
    try:
        os.makedirs(CACHE, exist_ok=True)
        with open(os.path.join(CACHE, 'journal.log'), 'a', encoding='utf-8') as f:
            f.write(f'{time.strftime("%Y-%m-%d %H:%M:%S")} {text}\n')
    except OSError:
        pass


def load_settings(path):
    """simhub.toml: source, UDP port and address, SimHub's address and rate, display options."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        log(f'{path}: {e}')
        data = {}
    pills = data.get('pilules') if isinstance(data.get('pilules'), dict) else {}
    try:
        rate = max(2.0, min(30.0, float(data.get('frequence', 10))))
    except (TypeError, ValueError):
        rate = 10.0
    listen, port, relays, problems = udp.settings(data, 8888)
    keys = [(str(t.get('nom') or t.get('touches')), str(t['touches']).strip())
            for t in data.get('touche') or [] if isinstance(t, dict) and str(t.get('touches', '')).strip()]
    # the SimHub program: its IP (or name) and TCP port; a former `adresse` URL still read
    old = urllib.parse.urlsplit(str(data.get('adresse') or ''))
    host = str(data.get('ip_programme') or old.hostname or '127.0.0.1').strip()
    if not HOST.match(host):
        problems.append(tr('program: "{host}" is neither an IP address nor a computer name', host=host))
        host = '127.0.0.1'
    try:
        tcp = int(data.get('port_programme') or old.port or 8888)
        if not 1 <= tcp <= 65535:
            raise ValueError
    except (TypeError, ValueError):
        problems.append(tr('program: port "{port}" between 1 and 65535', port=data.get('port_programme')))
        tcp = 8888
    base = os.environ.get('HSM01_SIMHUB_URL') or f'http://{host}:{tcp}'
    return {'source': PROGRAM if str(data.get('source', STREAM)) == PROGRAM else STREAM,
            'listen': listen, 'port': port, 'relays': relays, 'problems': problems,
            'base': base.strip(), 'rate': rate,
            'options': {'shift_lights': bool(data.get('leds_regime', True)),
                        'auto_race': bool(data.get('ecran_course', True)),
                        'pills': {'alertes': bool(pills.get('alertes', True)),
                                  'course': bool(pills.get('course', True))},
                        'keys': keys[:4]}}


def sample(v, game):
    """The dashboard's sample of the SimHub program's values (pedals 0-100 %, times in s)."""
    def n(key):
        x = v.get(key)
        return x if isinstance(x, (int, float)) and not isinstance(x, bool) else None

    def pct(key):
        x = n(key)
        return x / 100 if x is not None else None
    s = {'game': game or 'SimHub', 'vehicle': v.get('vehicle') if isinstance(v.get('vehicle'), str) else None,
         'speed': n('speed'), 'rpm': n('rpm'), 'max_rpm': n('max_rpm') or None,
         'gear': str(v['gear']) if v.get('gear') not in (None, '') else None,
         'throttle': pct('throttle'), 'brake': pct('brake'), 'clutch': pct('clutch'),
         'fuel': n('fuel'), 'fuel_max': n('fuel_max') or None, 'coolant': n('coolant') or None,
         'abs': bool(v.get('abs')), 'tc': bool(v.get('tc')), 'limiter': bool(v.get('limiter'))}
    if isinstance(s['gear'], str) and s['gear'].endswith('.0'):
        s['gear'] = s['gear'][:-2]  # a gear SimHub gave as a number
    corners = (('fl', 1, -1), ('fr', 1, 1), ('rl', -1, -1), ('rr', -1, 1))
    wheels = [{'x': x, 'y': y, 'tire_c': n(f'tire_{c}') or None, 'brake_c': n(f'brake_{c}') or None}
              for c, x, y in corners]
    if any(w['tire_c'] or w['brake_c'] for w in wheels):
        s['wheels'] = wheels
    lap, pos, t = n('lap'), n('position'), n('lap_time')
    if lap or pos or t:
        racers = n('racers')
        s['race'] = {'lap': int(lap or 0), 'laps': int(n('laps') or 0), 'position': int(pos or 0),
                     'racers': int(racers) if racers else None, 'lap_time': t,
                     'last': n('last'), 'best': n('best')}
    return s


class SimHubModule:
    def __init__(self, client, args):
        self.c = client
        self.settings = s = load_settings(os.path.join(HERE, 'simhub.toml'))
        stream = s['source'] == STREAM
        first = tr('Waiting for the game') if stream else tr('Connecting to SimHub…')
        self.dash = Dashboard(client, 'SimHub', hints=self.hints(first), options=s['options'])
        if args.show:
            client.show(self.dash.page_id)
        self.values, self.missing = {}, {}
        target = self.receive if stream else self.poll
        threading.Thread(target=target, name='simhub', daemon=True).start()

    def hints(self, first):
        s = self.settings
        if s['source'] == STREAM:
            pc = udp.this_pc()
            line = tr('driving; address 127.0.0.1 (this PC)') + (
                ' ' + tr('or {pc} (other PC)', pc=pc) if pc and s['listen'] == udp.ANY else '')
            relays = ', '.join(f'{h}:{p}' for h, p in s['relays'])
            out = [first, tr('Game: "SimHub compatible" output (Forza Data Out), port {port}', port=s['port']),
                   line + ('; ' + tr('relay {relays}', relays=relays) if relays else '')]
        else:
            out = [first, tr('SimHub: Dash Studio web server on ({base})', base=s['base']),
                   tr('a running game that SimHub recognises (address and port: "SimHub" tab)')]
        if s['problems']:  # a wrong setting: said instead of the last line
            out[2] = tr('Setting:') + ' ' + s['problems'][0]
        return out

    # --- the game's stream (UDP) ---------------------------------------------------------------
    def receive(self):
        s = self.settings
        for p in s['problems']:
            log(tr('setting: {problem}', problem=p))
        try:
            rx = udp.Receiver(s['listen'], s['port'], s['relays'])
        except OSError as e:
            log(tr('UDP listening on {address} impossible: {e}', address=f"{s['listen']}:{s['port']}", e=e))
            self.dash.message('error', [tr('SimHub: port unavailable'), f"UDP {s['listen']}:{s['port']}: {e.strerror or e}", tr('another program on this port? ("SimHub" tab)')])
            return
        relays = ', '.join(f'{h}:{p}' for h, p in s['relays'])
        log(tr('UDP listening on {address} (SimHub compatible stream)', address=f"{s['listen']}:{s['port']}")
            + (', ' + tr('relayed to {relays}', relays=relays) if relays else ''))
        seen, bad, idle = None, 0, False
        while not self.c.closed:
            try:
                got = rx.recv(1.0)
            except OSError as e:
                log(tr('reception: {e}', e=e))
                time.sleep(1)
                continue
            if got is None:
                continue
            data, addr = got
            try:
                raw = forza.decode(data)
            except ValueError as e:
                bad += 1
                if bad in (1, 100, 10000):
                    log(tr('packet refused: {e} (only the Forza Data Out format is read)', e=e))
                continue
            if len(data) != seen:
                seen = len(data)
                log(tr('reception: Forza Data Out format ({n} bytes) from {address}', n=len(data), address=addr[0]))
            smp = forza.sample(raw, 'SimHub')
            if smp is None:  # IsRaceOn 0: menu, pause, on foot
                if not idle:
                    idle = True
                    self.dash.message('waiting', self.hints(tr('Game paused, or not driving')))
                continue
            idle = False
            self.dash.feed(smp)

    # --- the SimHub program (web API) ----------------------------------------------------------
    def read(self, props):
        for key, name in props.items():
            if time.monotonic() < self.missing.get(name, 0):
                continue
            try:
                self.values[key] = self.api.get(GD + name)
            except api.SimHubError as e:
                if e.status == 0:
                    raise
                if name not in self.missing:
                    log(tr('{name}: {e} (asked again every {secs} s)', name=name, e=e, secs=f'{MISSING_RETRY:.0f}'))
                self.missing[name] = time.monotonic() + MISSING_RETRY
                self.values[key] = None

    def optional(self, name):
        """A property SimHub may not have (older versions): None then; unreachable: raised."""
        try:
            return self.api.get(name)
        except api.SimHubError as e:
            if e.status == 0:
                raise
            return None

    def poll(self):
        self.api = api.SimHub(self.settings['base'])
        period, turn, state, game, running = 1.0 / self.settings['rate'], 0, None, '', True
        while not self.c.closed:
            t0 = time.monotonic()
            try:
                if turn % 5 == 0:
                    running = self.optional('DataCorePlugin.GameRunning')
                    game = self.optional('DataCorePlugin.CurrentGame') or ''
                    if running is not False:
                        self.read(SLOW)
                if running is False:
                    if state != 'idle':
                        state = 'idle'
                        log(tr('connected ({base}): no game running', base=self.settings['base']))
                    self.dash.message('waiting', self.hints(tr('SimHub: no game running')))
                else:
                    self.read(FAST)
                    if state != 'live':
                        state = 'live'
                        log(tr('game: {game}; speed read: {speed}, gear: {gear}, lap: {lap}', game=game or '?', speed=repr(self.values.get('speed')), gear=repr(self.values.get('gear')), lap=repr(self.values.get('lap_time'))))
                    self.dash.feed(sample(self.values, str(game)))
            except api.SimHubError as e:
                if state != 'absent':
                    state = 'absent'
                    log(str(e))
                self.dash.message('absent', self.hints(tr('SimHub unreachable')))
                time.sleep(3)
                continue
            turn += 1
            time.sleep(max(0.0, period - (time.monotonic() - t0)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'simhub')
    m = SimHubModule(client, args)
    client.run(tick=m.dash.tick, fps=20)


if __name__ == '__main__':
    main()
