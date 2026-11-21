"""Motor Town's telemetry (Documentation/Telemetry/MotorTownTelemetryProtocol.md of the game,
protocol 1.2, revision 3), standard library only: one UDP datagram = one sample.

- native « Motor Town v1 »: header `MTTM`, major 1, any minor (fields only appended: v1.0 153
  bytes, v1.1 849, v1.2 858), datagram length = 8 + payload_size; little-endian, no padding;
- « Forza-compatible (Data Out) », also called SimHub-compatible: hsm01.forza (Sled 232, Dash 311,
  Forza Horizon 324 bytes), shared with the SimHub module.

`decode(data)` -> raw fields (dict) or ValueError; `sample(raw)` -> the dashboard's sample
(hsm01.dashboard): fields the game leaves at 0 or not finite are left out (None), as the protocol
says a zero can mean « not available ».
"""
import math
import struct

from hsm01 import forza
from hsm01.i18n import tr

NATIVE_V10 = [  # offset, name, format (from byte 0 of the datagram)
    (8, 'timestamp_us', 'q'), (16, 'frame_number', 'I'), (20, 'vehicle_id', 'q'),
    (28, 'vehicle_class_hash', 'I'), (32, 'position_cm', '3f'), (44, 'rotation_xyzw', '4f'),
    (60, 'velocity_world_cm_s', '3f'), (72, 'velocity_local_cm_s', '3f'),
    (84, 'angular_velocity_local_rad_s', '3f'), (96, 'acceleration_local_cm_s2', '3f'),
    (108, 'engine_rpm', 'f'), (112, 'engine_max_rpm', 'f'), (116, 'current_gear', 'i'),
    (120, 'max_forward_gear', 'i'), (124, 'speed_kmh', 'f'), (128, 'throttle', 'f'),
    (132, 'brake', 'f'), (136, 'clutch', 'f'), (140, 'steering', 'f'), (144, 'handbrake', 'B'),
    (145, 'flags', 'I'), (149, 'fuel_ratio', 'f')]
NATIVE_V11 = [
    (153, 'flags2', 'I'), (157, 'autopilot_mode', 'B'), (158, 'turn_signal', 'B'),
    (159, 'transmission_type', 'B'), (160, 'fuel_type', 'B'), (161, 'mission_type', 'B'),
    (162, 'weather', 'B'), (163, 'destination_kind', 'B'), (164, 'race_source', 'B'),
    (165, 'num_reverse_gears', 'B'), (166, 'coolant_temp_c', 'f'), (170, 'engine_torque_nm', 'f'),
    (174, 'engine_power_kw', 'f'), (178, 'boost_bar', 'f'), (182, 'engine_damage', 'f'),
    (186, 'current_gear_ratio', 'f'), (190, 'cruise_target_kmh', 'f'),
    (194, 'speed_limiter_target_kmh', 'f'), (198, 'autopilot_max_speed_kmh', 'f'),
    (202, 'fuel_amount', 'f'), (206, 'fuel_capacity', 'f'), (210, 'fuel_rate_lph', 'f'),
    (214, 'fuel_economy_kpl', 'f'), (218, 'ev_power_ratio', 'f'), (222, 'odometer_km', 'd'),
    (230, 'road_speed_limit_kmh', 'f'), (234, 'destination_distance_m', 'f'),
    (238, 'gross_weight_kg', 'f'), (242, 'cargo_weight_kg', 'f'), (246, 'race_elapsed_s', 'f'),
    (250, 'lap_time_s', 'f'), (254, 'last_lap_s', 'f'), (258, 'best_lap_s', 'f'),
    (262, 'lap_number', 'H'), (264, 'total_laps', 'H'), (266, 'race_position', 'B'),
    (267, 'num_racers', 'B'), (268, 'section_index', 'h'), (270, 'section_count', 'h'),
    (272, 'time_of_day_h', 'f'), (276, 'air_temp_c', 'f'), (280, 'rain_strength', 'f'),
    (284, 'money', 'q'), (292, 'wheel_count', 'B')]
NATIVE_V12 = [(849, 'combination_gross_weight_kg', 'f'), (853, 'trailers_cargo_weight_kg', 'f'),
              (857, 'trailer_count', 'B')]
WHEEL = [(0, 'angular_velocity_rad_s', 'f'), (4, 'slip_ratio', 'f'), (8, 'slip_angle_deg', 'f'),
         (12, 'suspension_travel_m', 'f'), (16, 'suspension_ratio', 'f'), (20, 'tire_temp_c', 'f'),
         (24, 'brake_temp_c', 'f'), (28, 'vertical_load_n', 'f'), (32, 'tire_damage', 'f'),
         (36, 'local_forward_m', 'f'), (40, 'local_right_m', 'f'), (44, 'surface', 'B'),
         (45, 'contact', 'B')]
WHEELS_AT, WHEEL_SIZE, WHEEL_SLOTS = 293, 46, 10
# flags (v1.0) and flags2 (v1.1) bits
PAUSED, DRIVEN, REVERSE, HEADLIGHTS, ABS, TC, CRUISE = (1 << i for i in range(7))
F2 = {'autopilot': 0, 'ai_driving': 1, 'overheat': 2, 'ignition': 3, 'rev_limiter': 4,
      'shifting': 5, 'auto_shift': 6, 'high_beam': 7, 'brake_lights': 8, 'horn': 9,
      'parking_brake': 10, 'wrong_way': 11, 'limiter': 12, 'trailer': 13, 'in_race': 14,
      'finished': 15, 'disqualified': 16, 'police': 17, 'suspect': 18}


def _read(data, fields, base=0):
    out = {}
    for off, name, fmt in fields:
        v = struct.unpack_from('<' + fmt, data, base + off)
        out[name] = v if len(v) > 1 else v[0]
    return out


def _text(raw):
    return raw.split(b'\0', 1)[0].decode('utf-8', 'replace')


def decode(data):
    """Raw fields of a datagram (native: 'format' 'mttm', 'version' (major, minor); Forza:
    'format' 'forza'). ValueError for anything else (wrong length, major version…)."""
    if data[:4] == b'MTTM':
        if len(data) < 8:
            raise ValueError(tr('truncated header'))
        major, minor, size = struct.unpack_from('<BBH', data, 4)
        if major != 1:
            raise ValueError(tr('unknown version {major}.{minor}', major=major, minor=minor))
        if size < 145 or len(data) != 8 + size:
            raise ValueError(tr('size {n} for a payload of {size}', n=len(data), size=size))
        raw = dict(_read(data, NATIVE_V10), format='mttm', version=(major, minor))
        if size >= 841:
            raw.update(_read(data, NATIVE_V11))
            raw['vehicle_name_en'] = _text(data[753:817])
            raw['vehicle_key'] = _text(data[817:849])
            raw['wheels'] = [_read(data, WHEEL, WHEELS_AT + WHEEL_SIZE * i)
                             for i in range(min(raw['wheel_count'], WHEEL_SLOTS))]
        if size >= 850:
            raw.update(_read(data, NATIVE_V12))
        return raw
    if len(data) in (forza.SLED, forza.DASH, forza.HORIZON):
        return dict(forza.decode(data), format='forza')
    raise ValueError(tr('unknown packet ({n} bytes)', n=len(data)))


def _num(v):
    return v if isinstance(v, (int, float)) and math.isfinite(v) else None


def _given(v):
    """A value the game gives: finite and not 0 (a zero can mean « not available »)."""
    v = _num(v)
    return v if v else None


def sample(raw):
    """The dashboard's sample (hsm01.dashboard) of decoded fields."""
    return _native(raw) if raw['format'] == 'mttm' else forza.sample(raw, 'Motor Town')


def _native(r):
    flags, f2 = r['flags'], r.get('flags2', 0)

    def bit2(name):
        return bool(f2 & (1 << F2[name]))
    s = {'game': 'Motor Town', 'vehicle': r.get('vehicle_name_en') or None,
         'paused': bool(flags & PAUSED), 'speed': _num(r['speed_kmh']),
         'rpm': _num(r['engine_rpm']) if r['engine_max_rpm'] else None,
         'max_rpm': _given(r['engine_max_rpm']), 'gear': r['current_gear'],
         'gears': r['max_forward_gear'] or None, 'throttle': _num(r['throttle']),
         'brake': _num(r['brake']), 'clutch': _num(r['clutch']), 'steering': _num(r['steering']),
         'fuel_ratio': _num(r['fuel_ratio']), 'headlights': bool(flags & HEADLIGHTS),
         'abs': bool(flags & ABS), 'tc': bool(flags & TC), 'cruise': bool(flags & CRUISE),
         'handbrake': bool(r['handbrake'])}
    if r['engine_max_rpm'] == 0:  # no engine component: no gear either
        s['gear'] = None
    if 'flags2' not in r:  # v1.0: nothing more
        return s
    electric = r['fuel_type'] == 3
    s.update({
        'fuel': _num(r['fuel_amount']) if r['fuel_capacity'] else None,
        'fuel_max': _given(r['fuel_capacity']), 'fuel_unit': 'kWh' if electric else 'L',
        'coolant': _given(r['coolant_temp_c']), 'boost': _num(r['boost_bar']),
        'power_kw': _num(r['engine_power_kw']), 'torque_nm': _num(r['engine_torque_nm']),
        'engine_damage': _num(r['engine_damage']),
        'high_beam': bit2('high_beam'), 'overheat': bit2('overheat'),
        'rev_limiter': bit2('rev_limiter'), 'wrong_way': bit2('wrong_way'),
        'limiter': bit2('limiter'), 'autopilot': r['autopilot_mode'] == 2 or bit2('ai_driving'),
        'handbrake': s['handbrake'] or bit2('parking_brake'),
        'cruise': s['cruise'] or r['autopilot_mode'] == 1,
        'cruise_target': _given(r['cruise_target_kmh']), 'turn': r['turn_signal'] & 3,
        'speed_limit': _given(r['road_speed_limit_kmh']),
        'destination_m': r['destination_distance_m'] if _num(r['destination_distance_m']) is not None
        and r['destination_distance_m'] >= 0 else None,
        'odometer_km': _given(r['odometer_km']), 'gross_kg': _given(r['gross_weight_kg']),
        'cargo_kg': _num(r['cargo_weight_kg']), 'money': r['money'],
        'time_of_day': _num(r['time_of_day_h']), 'air_c': _num(r['air_temp_c']),
        'wheels': [{'x': _num(w['local_forward_m']), 'y': _num(w['local_right_m']),
                    'tire_c': _given(w['tire_temp_c']), 'brake_c': _given(w['brake_temp_c']),
                    'damage': _num(w['tire_damage']), 'susp': _num(w['suspension_ratio'])}
                   for w in r.get('wheels') or []]})
    if r['fuel_type'] == 0 and not r['fuel_capacity']:
        s['fuel_ratio'] = None  # no tank (fuel type none)
    if 'trailer_count' in r:  # v1.2: the whole combination
        s['gross_kg'] = _given(r['combination_gross_weight_kg']) or s['gross_kg']
        s['cargo_kg'] = (_num(r['cargo_weight_kg']) or 0) + (_num(r['trailers_cargo_weight_kg']) or 0)
        s['trailers'] = r['trailer_count']
    # a race only once it runs: near a time attack course the game already says race_source 2,
    # with every time at 0 and no checkpoint passed
    started = (_given(r['lap_time_s']) or _given(r['race_elapsed_s']) or r['lap_number']
               or r['race_position'] or r['section_index'] >= 0 or bit2('finished') or bit2('disqualified'))
    if r['race_source'] and started:
        s['race'] = {'source': r['race_source'], 'lap': r['lap_number'], 'laps': r['total_laps'],
                     'position': r['race_position'], 'racers': r['num_racers'],
                     'lap_time': _num(r['lap_time_s']), 'last': _num(r['last_lap_s']),
                     'best': _num(r['best_lap_s']), 'elapsed': _num(r['race_elapsed_s']),
                     'section': r['section_index'], 'sections': r['section_count'],
                     'finished': bit2('finished'), 'disqualified': bit2('disqualified')}
    return s


