#!/usr/bin/env python3
"""Module « Motor Town » de new hera-nux: the game's telemetry (UDP, 60 samples a second; native
format « Motor Town v1 », 1.0 to 1.2, or its Forza-compatible output) on a dashboard
(hsm01.dashboard): revs, gear, speed, pedals, fuel, engine temperature, speed limit, lamps
(ABS, TC, cruise control, limiter, lights, handbrake, overheating, turn signals), race screen
(position, laps, times, checkpoints), vehicle screen (tyres, brakes, engine, weights); shift
lights on the LEDs; pills (overheating, low fuel, wrong way, laps, finish); widget « Tableau de
bord ». Knob 1 (turn or click): the screens; buttons 1-4: keys sent to the game (motortown.toml).

In the game: Options > Gameplay > Enable Telemetry Output, format « Motor Town v1 », address
127.0.0.1 (this computer), port 33330. Option: --show.
"""
import argparse
import os
import threading
import time
import tomllib

from hsm01 import Client, udp
from hsm01.dashboard import Dashboard
from hsm01.i18n import tr

import mt_telemetry as mt

HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.join(os.environ.get('XDG_CACHE_HOME') or os.path.expanduser('~/.cache'),
                     'hsm01', 'motortown')


def log(text):
    """A line in the Journal and in ~/.cache/hsm01/motortown/journal.log."""
    print(f'Motor Town: {text}', flush=True)
    try:
        os.makedirs(CACHE, exist_ok=True)
        with open(os.path.join(CACHE, 'journal.log'), 'a', encoding='utf-8') as f:
            f.write(f'{time.strftime("%Y-%m-%d %H:%M:%S")} {text}\n')
    except OSError:
        pass


def load_settings(path):
    """motortown.toml: listening address and port, LEDs, race screen, pills, buttons' keys."""
    try:
        with open(path, 'rb') as f:
            data = tomllib.load(f)
    except FileNotFoundError:
        data = {}
    except tomllib.TOMLDecodeError as e:
        log(f'{path}: {e}')
        data = {}
    pills = data.get('pilules') if isinstance(data.get('pilules'), dict) else {}
    host, port, relays, problems = udp.settings(data, 33330, fallback_keys=('adresse',))
    keys = [(str(t.get('nom') or t.get('touches')), str(t['touches']).strip())
            for t in data.get('touche') or [] if isinstance(t, dict) and str(t.get('touches', '')).strip()]
    return {'host': host, 'port': port, 'relays': relays, 'problems': problems,
            'options': {'shift_lights': bool(data.get('leds_regime', True)),
                        'auto_race': bool(data.get('ecran_course', True)),
                        'pills': {'alertes': bool(pills.get('alertes', True)),
                                  'course': bool(pills.get('course', True))},
                        'keys': keys[:4]}}


class MotorTown:
    def __init__(self, client, args):
        self.c = client
        self.settings = s = load_settings(os.path.join(HERE, 'motortown.toml'))
        self.hints = [tr('Waiting for Motor Town'),
                      tr('Game: Enable Telemetry Output, format Motor Town v1, port {port}', port=s['port']),
                      self.where()]
        if s['problems']:  # a wrong setting: said instead of the address line
            self.hints[2] = tr('Setting:') + ' ' + s['problems'][0]
            for p in s['problems']:
                log(tr('setting: {problem}', problem=p))
        self.dash = Dashboard(client, 'Motor Town', hints=self.hints, options=s['options'])
        if args.show:
            client.show(self.dash.page_id)
        self.seen, self.bad = None, 0
        threading.Thread(target=self.receive, name='motortown', daemon=True).start()

    def where(self):
        """The address line of the waiting screen: where the game must send, the relays."""
        s, pc = self.settings, udp.this_pc()
        if s['host'] in (udp.ANY, udp.LOCAL) or s['host'].startswith('127.'):
            line = tr('address 127.0.0.1 (this PC)') + (' ' + tr('or {pc} (other PC)', pc=pc) if pc and s['host'] == udp.ANY else '')
        else:
            line = tr('address {host}', host=s['host'])
        relays = ', '.join(f'{h}:{p}' for h, p in s['relays'])
        return line + ('; ' + tr('relay {relays}', relays=relays) if relays else '')

    def receive(self):
        """Datagrams on the port: each one relayed, decoded and given to the dashboard."""
        s = self.settings
        try:
            rx = udp.Receiver(s['host'], s['port'], s['relays'])
        except OSError as e:
            log(tr('listening on {address} impossible: {e}', address=f"{s['host']}:{s['port']}", e=e))
            self.dash.message('error', [tr('Motor Town: port unavailable'),
                                        f"UDP {s['host']}:{s['port']}: {e.strerror or e}",
                                        tr('another program on this port? ("Motor Town" tab)')])
            return
        relays = ', '.join(f'{h}:{p}' for h, p in s['relays'])
        log(tr('listening on {address}', address=f"{s['host']}:{s['port']}") + (', ' + tr('relayed to {relays}', relays=relays) if relays else ''))
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
                raw = mt.decode(data)
            except (ValueError, IndexError) as e:  # not ours, or a format the doc does not know
                self.bad += 1
                if self.bad in (1, 100, 10000):
                    log(tr('packet refused ({n} bytes): {e}', n=len(data), e=e))
                continue
            kind = (raw['format'], raw.get('version'), len(data))
            if kind != self.seen:
                self.seen = kind
                what = (f'Motor Town v{raw["version"][0]}.{raw["version"][1]}' if raw['format'] == 'mttm'
                        else tr('Forza compatible'))
                log(tr('reception: {what} ({n} bytes) from {address}', what=what, n=len(data), address=addr[0]))
            sample = mt.sample(raw)
            if sample is not None:
                self.dash.feed(sample)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'motortown')
    m = MotorTown(client, args)
    client.run(tick=m.dash.tick, fps=20)


if __name__ == '__main__':
    main()
