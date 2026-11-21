#!/usr/bin/env python3
"""A fake SimHub web API for the SimHub module's test: `GET /api/getproperty/<name>` answers the
value as JSON (numbers, texts, booleans; durations as SimHub's « 00:01:23.4560000 » text), 404 for
a property it does not know (TyreTemperature* left out on purpose). A car at 123 km/h in 4th,
6800 rpm of 8000, a race: lap 2 of 5, position 3, opponents 11. Prints `port N`, then one line
per new property asked.

Usage: python3 -I fake_simhub.py [--no-game]
"""
import argparse
import http.server
import json
import threading
import urllib.parse

ap = argparse.ArgumentParser()
ap.add_argument('--no-game', action='store_true', help='SimHub running, no game')
args = ap.parse_args()
GD = 'DataCorePlugin.GameData.'
VALUES = {'DataCorePlugin.GameRunning': not args.no_game, 'DataCorePlugin.CurrentGame': 'AssettoCorsa',
          GD + 'SpeedKmh': 123.4, GD + 'Rpms': 6800.0, GD + 'MaxRpm': 8000.0, GD + 'Gear': '4',
          GD + 'Throttle': 87.0, GD + 'Brake': 0.0, GD + 'Clutch': 0.0, GD + 'Fuel': 31.2,
          GD + 'MaxFuel': 60.0, GD + 'WaterTemperature': 88.0, GD + 'ABSActive': 1,
          GD + 'TCActive': 0, GD + 'PitLimiterOn': 0, GD + 'CarModel': 'Voiture test',
          GD + 'CurrentLap': 2, GD + 'TotalLaps': 5, GD + 'Position': 3, GD + 'OpponentsCount': 11,
          GD + 'CurrentLapTime': '00:00:41.2500000', GD + 'LastLapTime': '00:01:32.1230000',
          GD + 'BestLapTime': '00:01:31.9870000', GD + 'BrakeTemperatureFrontLeft': 410.0}
asked, lock = set(), threading.Lock()


class H(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'  # kept alive, as the module expects

    def log_message(self, *a):
        pass

    def do_GET(self):
        path = urllib.parse.unquote(urllib.parse.urlsplit(self.path).path)
        name = path.removeprefix('/api/getproperty/')
        with lock:
            if name not in asked:
                asked.add(name)
                print(f'propriété {name}', flush=True)
        if not path.startswith('/api/getproperty/') or name not in VALUES:
            body, code = b'Property not found', 404
        else:
            body, code = json.dumps(VALUES[name]).encode(), 200
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


srv = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
print(f'port {srv.server_address[1]}', flush=True)
srv.serve_forever()
