"""The « Data Out » UDP format of Forza Motorsport / Horizon, that SimHub and many dashboards
read, and that other games copy as their « SimHub-compatible » or « Forza-compatible » output
(Motor Town…): one datagram = one sample, little-endian.

- « Sled », 232 bytes: motion and engine only;
- « Dash » (FM7 and later), 311 bytes: Sled + position, speed, power, torque, tyre temperatures
  (°F), boost (psi), fuel (0..1), laps, race position, pedals (0-255), gear (0 = reverse, 11 =
  neutral), steering;
- Forza Horizon, 324 bytes: the Dash fields 12 bytes further.

`decode(data)` -> raw fields or ValueError; `sample(raw, game)` -> a dashboard sample
(hsm01.dashboard), None while `IsRaceOn` is 0 (menu, pause, no car: nothing to show).
"""
import math
import struct
from .i18n import tr

SLED, DASH, HORIZON = 232, 311, 324


def decode(data):
    n = len(data)
    if n not in (SLED, DASH, HORIZON):
        raise ValueError(tr('not a Forza packet ({n} bytes)', n=n))
    u = struct.unpack_from
    raw = {'size': n, 'is_race_on': u('<i', data, 0)[0], 'timestamp_ms': u('<I', data, 4)[0],
           'max_rpm': u('<f', data, 8)[0], 'idle_rpm': u('<f', data, 12)[0],
           'rpm': u('<f', data, 16)[0], 'velocity': u('<3f', data, 32),
           'susp': u('<4f', data, 68), 'car_ordinal': u('<i', data, 212)[0]}
    if n == SLED:
        return raw
    d = 12 if n == HORIZON else 0
    raw.update({'speed_ms': u('<f', data, 244 + d)[0], 'power_w': u('<f', data, 248 + d)[0],
                'torque_nm': u('<f', data, 252 + d)[0], 'tire_f': u('<4f', data, 256 + d),
                'boost_psi': u('<f', data, 272 + d)[0], 'fuel': u('<f', data, 276 + d)[0],
                'distance_m': u('<f', data, 280 + d)[0], 'best': u('<f', data, 284 + d)[0],
                'last': u('<f', data, 288 + d)[0], 'current': u('<f', data, 292 + d)[0],
                'race_time': u('<f', data, 296 + d)[0], 'lap_number': u('<H', data, 300 + d)[0],
                'position': data[302 + d], 'steer': u('<b', data, 308 + d)[0]})
    raw.update(zip(('accel', 'brake', 'clutch', 'handbrake', 'gear'), data[303 + d:308 + d]))
    return raw


def _num(v):
    return v if isinstance(v, (int, float)) and math.isfinite(v) else None


def _given(v):
    v = _num(v)
    return v if v else None


def sample(raw, game='Forza'):
    """The dashboard's sample of decoded fields (None while not racing)."""
    if not raw['is_race_on']:
        return None
    s = {'game': game, 'rpm': _num(raw['rpm']), 'max_rpm': _given(raw['max_rpm'])}
    if raw['size'] == SLED:  # no speedometer: the velocity's length
        v = raw['velocity']
        s['speed'] = _num(math.sqrt(sum(x * x for x in v)) * 3.6)
        return s
    g = raw['gear']
    corners = ((1, -1), (1, 1), (-1, -1), (-1, 1))  # FL FR RL RR
    s.update({'speed': _num(abs(raw['speed_ms']) * 3.6),
              'gear': -1 if g == 0 else 0 if g == 11 else g,
              'throttle': raw['accel'] / 255, 'brake': raw['brake'] / 255,
              'clutch': raw['clutch'] / 255, 'steering': raw['steer'] / 127,
              'handbrake': raw['handbrake'] > 0, 'fuel_ratio': _given(raw['fuel']),
              'boost': _num(raw['boost_psi'] * 0.0689476), 'power_kw': _num(raw['power_w'] / 1000),
              'torque_nm': _num(raw['torque_nm']),
              'wheels': [{'x': x, 'y': y, 'tire_c': _given((t - 32) / 1.8) if t else None,
                          'susp': _num(sus)}
                         for (x, y), t, sus in zip(corners, raw['tire_f'], raw['susp'])]})
    if raw['race_time'] > 0 or raw['lap_number'] or raw['position']:
        s['race'] = {'lap': raw['lap_number'] + 1, 'position': raw['position'],
                     'lap_time': _num(raw['current']), 'last': _given(raw['last']),
                     'best': _given(raw['best']), 'elapsed': _num(raw['race_time'])}
    return s
