"""A car dashboard on the Stream 100, shared by the telemetry modules (Motor Town, SimHub…): each
module turns its game's data into a `sample` (dict below) and gives it to `Dashboard.feed`; the
dashboard draws it on its page and widget, lights the LEDs as shift lights and says what matters
by pills (overheating, low fuel, wrong way, laps…).

>>> dash = Dashboard(client, 'Motor Town', hints=['Options > Gameplay > télémétrie…'])
>>> dash.feed({'speed': 48.0, 'rpm': 1976, 'max_rpm': 7700, 'gear': 3, 'vehicle': 'Fortem'})
>>> client.run(tick=dash.tick, fps=20)

Sample keys (absent or None: not shown): game, vehicle, paused; speed (km/h), rpm, max_rpm, gear
(int: <0 reverse, 0 neutral; or a text 'R', 'N', '3'), gears; throttle, brake, clutch (0..1);
fuel, fuel_max (litres, or kWh with fuel_unit 'kWh'), fuel_ratio (0..1); coolant (°C), boost
(bar), power_kw, torque_nm, engine_damage (0..1); abs, tc, cruise, limiter, headlights,
high_beam, handbrake, overheat, rev_limiter, wrong_way, autopilot (booleans); cruise_target
(km/h), turn (0 off, 1 left, 2 right, 3 hazard); speed_limit (km/h), destination_m (m);
race: {lap, laps, position, racers, lap_time, last, best, elapsed (s), section, sections,
finished, disqualified}; wheels: [{x (m forward), y (m right), tire_c, brake_c, damage (0..1),
susp (0..1)}]; odometer_km, gross_kg, cargo_kg, trailers, money, time_of_day (h), air_c.
"""
import math
import time

from . import widgets as W
from .i18n import point, tr

BG, TEXT, SOFT, GREY, DIM, TRACK = '#0b0c10', '#eef0f4', '#c2c6d0', '#8a8f9c', '#4a4f5c', '#1c1f27'
GREEN, AMBER, RED, BLUE, CYAN = '#38d07a', '#f2b33d', '#ef4b4b', '#4f8cff', '#3cc8e0'
LAMP_OFF = '#171a21'
SEGMENTS = 20
SCREENS = ['conduite', 'course', 'vehicule']
SCREEN_NAMES = {'conduite': tr('Driving'), 'course': tr('Race'), 'vehicule': tr('Vehicle')}
SILENCE = 1.5  # s without a sample: the game is not sending (menu, on foot…)
# lamps: (sample key, text, colour when on)
LAMPS = [('abs', 'ABS', AMBER), ('tc', 'TC', AMBER), ('cruise', tr('CC'), GREEN),
         ('limiter', 'LIM', CYAN), ('headlights', tr('LIGHTS'), GREEN),
         ('high_beam', tr('HIGH'), BLUE), ('handbrake', tr('BRAKE'), RED), ('overheat', 'TEMP', RED)]


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def num(v):
    """A finite number, else None (fields a game leaves out, NaN…)."""
    try:
        v = float(v)
    except (TypeError, ValueError):
        return None
    return v if math.isfinite(v) else None


def lap_text(s):
    """1:23.456 (h:mm:ss.mmm past an hour); '' for none."""
    s = num(s)
    if not s or s <= 0:
        return ''
    ms = int(round(s * 1000))
    m, ms = divmod(ms, 60000)
    h, m = divmod(m, 60)
    return f'{h}:{m:02d}:{ms / 1000:06.3f}' if h else f'{m}:{ms / 1000:06.3f}'


def gear_text(g):
    if g is None or g == '':
        return ''
    if isinstance(g, str):
        return g.strip().upper()[:3]
    g = int(g)
    return 'N' if g == 0 else 'R' if g == -1 else f'R{-g}' if g < 0 else str(g)


def decimal(v, digits=1):
    return point(f'{v:.{digits}f}')


def distance_text(m):
    m = num(m)
    if m is None or m < 0:
        return ''
    return f'{decimal(m / 1000)} km' if m >= 1000 else f'{int(m)} m'


class Dashboard:
    """The page `page` (screens: driving, race, vehicle; knob 1 turns or clicks between them),
    the widget « Tableau de bord » (gear, speed, revs), shift lights on the LEDs and pills.
    Options: shift_lights, auto_race (show the race screen when one starts), pills
    {'alertes', 'course'}, keys: up to 4 (label, keys) sent to the game by buttons 1-4."""

    def __init__(self, client, name, page='tableau', hints=(), options=None):
        self.c, self.name, self.page_id = client, name, page
        o = options or {}
        self.shift_lights = o.get('shift_lights', True)
        self.auto_race = o.get('auto_race', True)
        self.pills = dict({'alertes': True, 'course': True}, **(o.get('pills') or {}))
        self.keys = [(str(label), str(keys)) for label, keys in (o.get('keys') or [])][:4]
        self.hints = list(hints)
        self.sample, self.at, self.status = {}, 0.0, 'waiting'
        self.screen, self.drawn_screen = 'conduite', None
        self.sent, self.wsent, self.leds_sent, self.blink = {}, {}, None, False
        self.memory = {}  # what the pills remember (low fuel said, lap seen…)
        self.widget_made = False
        client.on('turn', self.on_turn)
        client.on('click', self.on_click)
        self.draw_page()

    # --- input from the module ---------------------------------------------------------------
    def feed(self, sample):
        """A new sample from the game (any thread): drawn at the next tick."""
        self.sample, self.at = sample, time.monotonic()

    def message(self, status, lines=None):
        """Not receiving: `status` ('waiting', 'error'…) and the lines to show instead."""
        if lines is not None:
            self.hints = list(lines)
        self.status = status

    # --- page ----------------------------------------------------------------------------------
    def draw_page(self):
        """(Re)make the page for the current screen (each screen has its own widgets)."""
        out = [W.text('veh', 8, 2, 300, 24, self.name, size=15, bold=True, align='left', color=TEXT),
               W.icon('tl', 314, 1, 24, 24, 'go-previous', tint=DIM),  # turn signals
               W.icon('tr', 340, 1, 24, 24, 'go-next', tint=DIM),
               W.text('info', 370, 4, 102, 20, '', size=12, align='right', color=GREY)]
        x0, step = 10, 23
        out += [W.rect(f'rv{i}', x0 + i * step, 30, step - 2, 16, TRACK, radius=3) for i in range(SEGMENTS)]
        out += getattr(self, f'widgets_{self.screen}')()
        out += [W.text('msg1', 16, 90, 448, 30, '', size=17, color=TEXT),
                W.text('msg2', 16, 124, 448, 22, '', size=13, color=GREY),
                W.text('msg3', 16, 148, 448, 22, '', size=13, color=GREY)]
        for i, (_, text, _) in enumerate(LAMPS):
            out += [W.rect(f'lp{i}', 8 + i * 58, 204, 54, 22, LAMP_OFF, radius=7),
                    W.text(f'lt{i}', 8 + i * 58, 204, 54, 22, text, size=11, bold=True, color=DIM)]
        if self.keys:  # what buttons 1-4 send to the game, above them
            out += [W.text(f'key{k + 1}', k * 120 + 4, 240, 112, 22, label, size=12, color=SOFT,
                           frame=W.frame(border='#262a34', width=1.0, radius=8, padding=0))
                    for k, (label, _) in enumerate(self.keys) if label]
        else:
            out += [W.text('trip', 8, 238, 180, 22, '', size=12, align='left', color=SOFT),
                    W.text('trip2', 292, 238, 180, 22, '', size=12, align='right', color=SOFT)]
        self.c.page(self.page_id, out, name=self.name, background=BG)
        self.sent, self.drawn_screen = {}, self.screen

    @staticmethod
    def widgets_conduite():
        out = [W.text('spd', 8, 50, 150, 70, '', size=58, bold=True, align='right', color=TEXT),
               W.text('spdu', 8, 118, 150, 18, 'km/h', size=13, align='right', color=GREY),
               W.rect('lmr', 22, 142, 46, 46, BG, radius=23),
               W.rect('lmi', 27, 147, 36, 36, BG, radius=18),
               W.text('lmt', 22, 142, 46, 46, '', size=15, bold=True, color='#111111'),
               W.text('cru', 74, 150, 86, 30, '', size=14, bold=True, align='left', color=GREEN),
               W.text('gear', 160, 46, 160, 116, '', size=100, bold=True, color=TEXT),
               W.text('rpmt', 160, 162, 160, 20, '', size=13, color=GREY),
               W.text('dest', 160, 180, 160, 18, '', size=12, color=SOFT)]
        for i, (key, color) in enumerate((('th', GREEN), ('br', RED), ('cl', BLUE))):
            x = 332 + i * 16
            out += [W.rect(f'{key}b', x, 56, 10, 120, TRACK, radius=3),
                    W.rect(key, x, 175, 10, 1, color, radius=3)]
        out += [W.text('fut', 384, 54, 90, 18, '', size=12, align='left', color=GREY),
                W.rect('fub', 384, 74, 88, 8, TRACK, radius=4),
                W.rect('fuf', 384, 74, 1, 8, GREEN, radius=4),
                W.text('fu', 384, 84, 90, 20, '', size=14, bold=True, align='left', color=TEXT),
                W.text('cot', 384, 110, 90, 18, '', size=12, align='left', color=GREY),
                W.text('co', 384, 128, 90, 20, '', size=14, bold=True, align='left', color=TEXT),
                W.text('bo', 384, 156, 90, 20, '', size=12, align='left', color=GREY)]
        return out

    @staticmethod
    def widgets_course():
        return [W.text('rpos', 8, 50, 150, 60, '', size=44, bold=True, align='left', color=TEXT),
                W.text('rposu', 8, 108, 150, 18, '', size=12, align='left', color=GREY),
                W.text('rlap', 8, 130, 150, 30, '', size=22, bold=True, align='left', color=TEXT),
                W.text('rsec', 8, 162, 150, 22, '', size=13, align='left', color=GREY),
                W.text('rtime', 160, 54, 312, 56, '', size=44, bold=True, align='right', color=TEXT),
                W.text('rlast', 160, 116, 312, 24, '', size=16, align='right', color=SOFT),
                W.text('rbest', 160, 142, 312, 24, '', size=16, align='right', color='#c9a2ff'),
                W.text('rel', 160, 170, 312, 20, '', size=12, align='right', color=GREY)]

    @staticmethod
    def widgets_vehicule():
        out = []
        for i, (x, y) in enumerate(((8, 52), (124, 52), (8, 124), (124, 124))):
            out += [W.rect(f'wb{i}', x, y, 110, 66, TRACK, radius=8),
                    W.text(f'wt{i}', x + 4, y + 2, 102, 26, '', size=18, bold=True, color=TEXT),
                    W.text(f'wd{i}', x + 4, y + 28, 102, 18, '', size=11, color=GREY),
                    W.rect(f'wsb{i}', x + 8, y + 52, 94, 6, BG, radius=3),
                    W.rect(f'ws{i}', x + 8, y + 52, 1, 6, BLUE, radius=3)]
        out += [W.text(f'e{i}', 246, 52 + i * 24, 226, 22, '', size=13, align='left', color=SOFT)
                for i in range(6)]
        return out

    # --- drawing (tick) ------------------------------------------------------------------------
    def tick(self, dt):
        live = self.at and time.monotonic() - self.at < SILENCE
        status = 'live' if live else self.status
        s = self.sample if live else {}
        race = s.get('race') or {}
        if self.auto_race and race and not self.memory.get('race_on') and self.screen == 'conduite':
            self.screen = 'course'
        self.memory['race_on'] = bool(race)
        if self.screen != self.drawn_screen:
            self.draw_page()
        want = {'veh': {'text': s.get('vehicle') or self.name}}
        info = 'pause' if s.get('paused') else SCREEN_NAMES[self.screen] if live else ''
        want['info'] = {'text': info}
        for i, line in enumerate((self.hints + ['', '', ''])[:3] if not live else ['', '', '']):
            want[f'msg{i + 1}'] = {'text': line}
        frac = self.revs(s)
        for i in range(SEGMENTS):
            on = frac is not None and frac * SEGMENTS > i + 0.5
            color = (GREEN if i < SEGMENTS * 0.6 else AMBER if i < SEGMENTS * 0.85 else RED) if on else \
                (TRACK if frac is not None else BG)
            if on and s.get('rev_limiter') and self.blink:
                color = BLUE
            want[f'rv{i}'] = {'color': color}
        want.update(getattr(self, f'show_{self.screen}')(s))
        for i, (key, _, color) in enumerate(LAMPS):
            on = bool(s.get(key))
            want[f'lp{i}'] = {'color': color if on else LAMP_OFF if live else BG}
            want[f'lt{i}'] = {'color': BG if on else DIM if live else BG}
        turn = int(s.get('turn') or 0)
        blink = self.blink or not live
        want['tl'] = {'tint': GREEN if turn in (1, 3) and blink else DIM if live else BG}
        want['tr'] = {'tint': GREEN if turn in (2, 3) and blink else DIM if live else BG}
        if not self.keys:
            want['trip'] = {'text': self.trip(s) if live else ''}
            want['trip2'] = {'text': self.trip2(s) if live else ''}
        self.c.update(self.page_id, self.diff(self.sent, want))
        self.blink = (time.monotonic() % 0.8) < 0.4
        self.leds(s, frac, live)
        self.widget(s, frac, live)
        if live:
            self.say(s)

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

    @staticmethod
    def revs(s):
        rpm, top = num(s.get('rpm')), num(s.get('max_rpm'))
        return clamp(rpm / top, 0.0, 1.0) if rpm is not None and top else None

    def show_conduite(self, s):
        w = {}
        spd = num(s.get('speed'))
        w['spd'] = {'text': f'{abs(spd):.0f}' if spd is not None else ''}
        w['spdu'] = {'text': 'km/h' if spd is not None else ''}
        lim = num(s.get('speed_limit'))
        over = lim and spd is not None and spd > lim + 3
        w['lmr'] = {'color': RED if lim else BG}
        w['lmi'] = {'color': ('#ffd7d7' if over else '#ffffff') if lim else BG}
        w['lmt'] = {'text': f'{lim:.0f}' if lim else ''}
        target = num(s.get('cruise_target'))
        w['cru'] = {'text': tr('CC {speed}', speed=f'{target:.0f}') if s.get('cruise') and target else
                    'AUTO' if s.get('autopilot') else ''}
        w['gear'] = {'text': gear_text(s.get('gear')),
                     'color': RED if s.get('rev_limiter') else TEXT}
        rpm = num(s.get('rpm'))
        w['rpmt'] = {'text': tr('{rpm} rpm', rpm=f'{rpm:.0f}') if rpm is not None else ''}
        w['dest'] = {'text': tr('destination {distance}', distance=distance_text(s.get('destination_m')))
                     if distance_text(s.get('destination_m')) else ''}
        for key in ('th', 'br', 'cl'):
            v = num(s.get({'th': 'throttle', 'br': 'brake', 'cl': 'clutch'}[key]))
            h = round(120 * clamp(v, 0.0, 1.0)) if v is not None else 0
            w[f'{key}b'] = {'color': TRACK if v is not None else BG}
            w[key] = {'y': 176 - max(1, h), 'h': max(1, h), 'color': (GREEN if key == 'th' else RED
                      if key == 'br' else BLUE) if h else BG}
        ratio = num(s.get('fuel_ratio'))
        fuel, unit = num(s.get('fuel')), s.get('fuel_unit') or 'L'
        if ratio is None and fuel is not None and num(s.get('fuel_max')):
            ratio = fuel / num(s.get('fuel_max'))
        has_fuel = ratio is not None or fuel is not None
        w['fut'] = {'text': (tr('battery') if unit == 'kWh' else tr('fuel')) if has_fuel else ''}
        w['fub'] = {'color': TRACK if ratio is not None else BG}
        r = clamp(ratio or 0.0, 0.0, 1.0)
        w['fuf'] = {'w': max(1, round(88 * r)), 'color': (RED if r < 0.1 else AMBER if r < 0.2 else GREEN)
                    if ratio is not None and r > 0 else BG}
        w['fu'] = {'text': (f'{decimal(fuel)} {unit}' if fuel is not None else f'{ratio * 100:.0f} %')
                   if has_fuel else ''}
        cool = num(s.get('coolant'))
        cool = cool if cool else None  # 0: not given by the game
        w['cot'] = {'text': tr('engine') if cool is not None else ''}
        w['co'] = {'text': f'{cool:.0f} °C' if cool is not None else '',
                   'color': RED if s.get('overheat') or (cool or 0) > 110 else TEXT}
        boost = num(s.get('boost'))
        w['bo'] = {'text': f'turbo {decimal(boost, 2)} bar' if boost and abs(boost) > 0.01 else ''}
        return w

    def show_course(self, s):
        r = s.get('race') or {}
        w = {}
        pos, n = r.get('position'), r.get('racers')
        w['rpos'] = {'text': f'P{pos}' if pos else ''}
        w['rposu'] = {'text': tr('of {n}', n=n) if pos and n else ''}
        lap, laps = r.get('lap'), r.get('laps')
        w['rlap'] = {'text': (tr('lap {lap}/{laps}', lap=lap, laps=laps) if laps and laps > 1 else tr('lap {lap}', lap=lap)) if lap else
                     (tr('no race') if not r else '')}
        sec, secs = r.get('section'), r.get('sections')
        w['rsec'] = {'text': tr('checkpoint {n}/{total}', n=max(0, (sec or 0) + 1), total=secs) if secs else ''}
        state = tr('DISQUALIFIED') if r.get('disqualified') else tr('FINISHED') if r.get('finished') else ''
        w['rtime'] = {'text': state or lap_text(r.get('lap_time')),
                      'color': RED if r.get('disqualified') else GREEN if r.get('finished') else TEXT}
        w['rlast'] = {'text': tr('last {time}', time=lap_text(r.get('last'))) if lap_text(r.get('last')) else ''}
        w['rbest'] = {'text': tr('best {time}', time=lap_text(r.get('best'))) if lap_text(r.get('best')) else ''}
        w['rel'] = {'text': f'total {lap_text(r.get("elapsed"))}' if lap_text(r.get('elapsed')) else ''}
        return w

    def show_vehicule(self, s):
        w = {}
        wheels = self.corners(s.get('wheels') or [])
        for i in range(4):
            wh = wheels[i]
            if not wh:
                w[f'wb{i}'], w[f'wt{i}'], w[f'wd{i}'] = {'color': BG}, {'text': ''}, {'text': ''}
                w[f'wsb{i}'], w[f'ws{i}'] = {'color': BG}, {'color': BG}
                continue
            t = num(wh.get('tire_c'))
            hot = t is not None and t > 100
            w[f'wb{i}'] = {'color': '#3a1d1d' if hot else TRACK}
            w[f'wt{i}'] = {'text': f'{t:.0f} °C' if t is not None else '—'}
            parts = []
            if num(wh.get('brake_c')):
                parts.append(tr('brake {t}°', t=f"{num(wh['brake_c']):.0f}"))
            if num(wh.get('damage')):
                parts.append(tr('wear {pct} %', pct=f"{num(wh['damage']) * 100:.0f}"))
            w[f'wd{i}'] = {'text': ' · '.join(parts)}
            sus = num(wh.get('susp'))
            w[f'wsb{i}'] = {'color': BG if sus is None else '#0e1016'}
            w[f'ws{i}'] = {'w': max(1, round(94 * clamp(sus or 0.0, 0.0, 1.0))),
                           'color': BLUE if sus is not None else BG}
        lines = []
        pw, tq = num(s.get('power_kw')), num(s.get('torque_nm'))
        if pw is not None and tq is not None:
            lines.append(f'{pw:.0f} kW · {tq:.0f} N·m')
        if num(s.get('engine_damage')):
            lines.append(tr('engine worn at {pct} %', pct=f"{num(s['engine_damage']) * 100:.0f}"))
        if num(s.get('odometer_km')):
            lines.append(tr('odometer {km} km', km=f"{num(s['odometer_km']):,.0f}").replace(',', ' '))
        if num(s.get('gross_kg')):
            cargo = num(s.get('cargo_kg')) or 0
            lines.append(point(f'{num(s["gross_kg"]) / 1000:.1f} t') +
                         (' ' + tr('including {t} t of cargo', t=point(f'{cargo / 1000:.1f}')) if cargo else ''))
        if s.get('trailers'):
            lines.append(tr('{n} trailer(s)', n=s['trailers']))
        if num(s.get('air_c')) is not None and s.get('air_c'):
            lines.append(f'air {num(s["air_c"]):.0f} °C')
        for i in range(6):
            w[f'e{i}'] = {'text': lines[i] if i < len(lines) else ''}
        return w

    @staticmethod
    def corners(wheels):
        """Front left, front right, rear left, rear right among the wheels (by their position:
        the front-most and rear-most axles), None where there is none."""
        pts = [wh for wh in wheels if num(wh.get('x')) is not None and num(wh.get('y')) is not None]
        if not pts:
            return (list(wheels) + [None] * 4)[:4]
        front, rear = max(num(w['x']) for w in pts), min(num(w['x']) for w in pts)

        def pick(axle, left):
            on = [w for w in pts if abs(num(w['x']) - axle) < 0.3]
            side = [w for w in on if (num(w['y']) < 0) == left] or on
            return min(side, key=lambda w: num(w['y'])) if left else max(side, key=lambda w: num(w['y']))
        return [pick(front, True), pick(front, False), pick(rear, True), pick(rear, False)]

    def trip(self, s):
        tod = num(s.get('time_of_day'))
        clock = f'{int(tod) % 24:02d}:{int(tod * 60) % 60:02d}' if tod is not None and s.get('time_of_day') else ''
        return ' · '.join(x for x in (clock, distance_text(s.get('destination_m'))) if x)

    def trip2(self, s):
        money = s.get('money')
        return f'{int(money):,} $'.replace(',', ' ') if money not in (None, '') else ''

    # --- LEDs, widget, pills -------------------------------------------------------------------
    def leds(self, s, frac, live):
        if not (self.shift_lights and live):
            values = [0] * 4
        elif s.get('rev_limiter') or (frac or 0) > 0.97:
            values = [100] * 4 if self.blink else [0] * 4
        else:
            values = [100 if (frac or 0) >= t else 8 for t in (0.70, 0.78, 0.86, 0.93)]
        if values != self.leds_sent:
            self.leds_sent = values
            self.c.leds(self.page_id, values)

    def widget(self, s, frac, live):
        """« Tableau de bord » (220 × 68): gear, speed, revs, to place on any page."""
        if not self.widget_made:
            self.widget_made = True
            out = [W.text('g', 4, 0, 70, 60, '', size=48, bold=True, color=TEXT),
                   W.text('v', 78, 2, 138, 34, '', size=28, bold=True, align='right', color=TEXT),
                   W.text('u', 78, 36, 138, 16, '', size=11, align='right', color=GREY)]
            out += [W.rect(f'r{i}', 4 + i * 21, 58, 19, 8, TRACK, radius=2) for i in range(10)]
            self.c.widget('tableau_mini', 220, 68, out, name=tr('Dashboard'), background=BG)
        spd = num(s.get('speed'))
        want = {'g': {'text': gear_text(s.get('gear')) if live else ''},
                'v': {'text': f'{abs(spd):.0f}' if live and spd is not None else '—'},
                'u': {'text': 'km/h' if live else tr('{name}: waiting', name=self.name)}}
        for i in range(10):
            on = frac is not None and live and frac * 10 > i + 0.5
            want[f'r{i}'] = {'color': (GREEN if i < 6 else AMBER if i < 8 else RED) if on else TRACK}
        self.c.update('tableau_mini', self.diff(self.wsent, want))

    def pill(self, text, color):
        self.c.pill(text, border=color, dot=color, duration=3.0, app=self.name)

    def say(self, s):
        """Pills for what changes: overheating, low fuel, wrong way, laps, finish."""
        m = self.memory
        if self.pills['alertes']:
            for key, text, color in (('overheat', tr('ENGINE OVERHEATING'), RED),
                                     ('wrong_way', tr('WRONG WAY'), RED)):
                on = bool(s.get(key))
                if on and not m.get(key):
                    self.pill(text, color)
                m[key] = on
            ratio = num(s.get('fuel_ratio'))
            if ratio is None and num(s.get('fuel')) is not None and num(s.get('fuel_max')):
                ratio = num(s['fuel']) / num(s['fuel_max'])
            if ratio is not None and ratio > 0:
                if ratio < 0.1 and not m.get('low_fuel'):
                    m['low_fuel'] = True
                    self.pill(tr('LOW FUEL: REFUEL') if s.get('fuel_unit') != 'kWh' else
                              tr('LOW BATTERY'), AMBER)
                elif ratio > 0.15:
                    m['low_fuel'] = False
        r = s.get('race') or {}
        if self.pills['course'] and r:
            lap = r.get('lap')
            if lap and m.get('lap') and lap > m['lap'] and lap_text(r.get('last')):
                best = r.get('best') and abs(num(r['best']) - num(r['last'])) < 0.001
                self.pill(tr('LAP {n}: {time}', n=lap - 1, time=lap_text(r['last'])) + (' (' + tr('BEST') + ')' if best else ''),
                          '#c9a2ff' if best else GREEN)
            if r.get('finished') and not m.get('finished'):
                pos = r.get('position')
                self.pill(tr('FINISHED') + (f' P{pos}' if pos else ''), GREEN)
            if r.get('disqualified') and not m.get('disqualified'):
                self.pill(tr('DISQUALIFIED'), RED)
            m['lap'], m['finished'], m['disqualified'] = lap, r.get('finished'), r.get('disqualified')
        elif not r:
            m['lap'] = m['finished'] = m['disqualified'] = None

    # --- input ----------------------------------------------------------------------------------
    def mine(self, ev):
        return ev.get('mine') and ev.get('page') == self.page_id

    def on_turn(self, ev):
        if self.mine(ev) and ev.get('knob') == 1 and ev.get('delta'):
            i = SCREENS.index(self.screen) + (1 if ev['delta'] > 0 else -1)
            self.screen = SCREENS[i % len(SCREENS)]

    def on_click(self, ev):
        if not self.mine(ev):
            return
        b = ev.get('button', '')
        if b == 'knob1':
            self.screen = SCREENS[(SCREENS.index(self.screen) + 1) % len(SCREENS)]
        elif b.startswith('button') and b[-1:] in '1234':
            k = int(b[-1]) - 1
            if k < len(self.keys) and self.keys[k][1]:
                self.c.action({'type': 'keys', 'keys': self.keys[k][1]})
