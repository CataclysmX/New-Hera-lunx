#!/usr/bin/env python3
"""Fake Spotify (accounts + Web API + covers on 127.0.0.1), to try the « Spotify » module without
an account: /authorize sends the browser straight back to the redirect address with a code (the
user agreeing), /api/token gives and refreshes tokens, /v1/me/player… plays three tracks
(Morceau 1-3 by Artiste A), likes, playlists (« Ma liste », « Sport »); /v1/recommendations is
closed (404) as for applications created since the end of 2024.

Usage: python3 -I tools/fake_spotify.py [--port 0] [--client-id abc] [--secret s3cret] [--free]
Prints `port N` once ready. Module: HSM01_SPOTIFY_ACCOUNTS=http://127.0.0.1:N,
HSM01_SPOTIFY_API=http://127.0.0.1:N/v1, HSM01_SPOTIFY_BROWSER="curl -sL" (the « browser »).
--free: player commands refused (Premium required). Each call is printed: METHOD path -> status.
"""
import argparse
import base64
import http.server
import json
import struct
import threading
import urllib.parse
import uuid
import zlib

ARTIST = {'name': 'Artiste A', 'uri': 'spotify:artist:a1', 'id': 'a1'}


def track(n, base):
    return {'id': f't{n}', 'uri': f'spotify:track:t{n}', 'name': f'Morceau {n}', 'type': 'track',
            'duration_ms': 200000, 'artists': [ARTIST],
            'album': {'name': 'Album test', 'images': [{'url': f'{base}/img/{n}.png'}]}}


def png(rgb, size=64):
    raw = b''.join(b'\x00' + bytes(rgb) * size for _ in range(size))

    def chunk(kind, data):
        return (struct.pack('>I', len(data)) + kind + data
                + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff))
    head = struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', head) + chunk(b'IDAT', zlib.compress(raw))
            + chunk(b'IEND', b''))


class State:
    def __init__(self, args):
        self.args, self.lock = args, threading.Lock()
        self.tokens, self.refresh, self.codes = set(), set(), {}
        self.index, self.playing, self.progress = 0, True, 30000
        self.shuffle, self.repeat, self.volume = False, 'off', 50
        self.liked, self.context = set(), None
        self.playlists = {'p1': {'name': 'Ma liste', 'tracks': ['spotify:track:t2']},
                          'p2': {'name': 'Sport', 'tracks': []}}

    def player(self, base):
        return {'is_playing': self.playing, 'progress_ms': self.progress,
                'shuffle_state': self.shuffle, 'repeat_state': self.repeat,
                'device': {'id': 'd1', 'name': 'Ordinateur test', 'type': 'Computer',
                           'volume_percent': self.volume, 'is_active': True},
                'context': {'uri': self.context} if self.context else None,
                'item': track(self.index + 1, base)}


def handler(state):
    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def reply(self, code, body=None, kind='application/json', headers=()):
            data = b'' if body is None else body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(code)
            for k, v in headers:
                self.send_header(k, v)
            if data:
                self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            print(f'{self.command} {urllib.parse.urlparse(self.path).path} -> {code}', flush=True)

        def error(self, code, message, reason=''):
            body = {'error': {'status': code, 'message': message}}
            if reason:
                body['error']['reason'] = reason
            self.reply(code, body)

        def base(self):
            return f'http://{self.headers.get("Host")}'

        def body(self):
            n = int(self.headers.get('Content-Length') or 0)
            return self.rfile.read(n) if n else b''

        def do_GET(self):
            u = urllib.parse.urlparse(self.path)
            q = dict(urllib.parse.parse_qsl(u.query))
            if u.path == '/authorize':
                if q.get('client_id') != state.args.client_id:
                    return self.reply(400, {'error': 'invalid_client'})
                code = uuid.uuid4().hex
                state.codes[code] = q.get('redirect_uri')
                sep = '&' if '?' in q['redirect_uri'] else '?'
                loc = f'{q["redirect_uri"]}{sep}code={code}&state={urllib.parse.quote(q.get("state", ""))}'
                return self.reply(302, None, headers=[('Location', loc)])
            if u.path.startswith('/img/'):
                n = int(u.path.split('/')[-1].split('.')[0])
                return self.reply(200, png((40 * n, 180, 90)), 'image/png')
            self.api('GET', u, q)

        def do_POST(self):
            u = urllib.parse.urlparse(self.path)
            if u.path == '/api/token':
                return self.token(dict(urllib.parse.parse_qsl(self.body().decode())))
            self.api('POST', u, dict(urllib.parse.parse_qsl(u.query)))

        def do_PUT(self):
            u = urllib.parse.urlparse(self.path)
            self.api('PUT', u, dict(urllib.parse.parse_qsl(u.query)))

        def do_DELETE(self):
            u = urllib.parse.urlparse(self.path)
            self.api('DELETE', u, dict(urllib.parse.parse_qsl(u.query)))

        def token(self, form):
            a = state.args
            basic = base64.b64encode(f'{a.client_id}:{a.secret}'.encode()).decode()
            if self.headers.get('Authorization') != f'Basic {basic}':
                return self.reply(400, {'error': 'invalid_client', 'error_description': 'Invalid client'})
            grant = form.get('grant_type')
            if grant == 'authorization_code':
                if state.codes.pop(form.get('code'), None) != form.get('redirect_uri'):
                    return self.reply(400, {'error': 'invalid_grant', 'error_description': 'Invalid authorization code'})
            elif grant != 'refresh_token' or form.get('refresh_token') not in state.refresh:
                return self.reply(400, {'error': 'invalid_grant', 'error_description': 'Invalid refresh token'})
            access, refresh = uuid.uuid4().hex, uuid.uuid4().hex
            state.tokens.add(access)
            state.refresh.add(refresh)
            self.reply(200, {'access_token': access, 'token_type': 'Bearer', 'expires_in': 3600,
                             'refresh_token': refresh, 'scope': 'user-read-playback-state'})

        def api(self, method, u, q):
            if self.headers.get('Authorization', '')[7:] not in state.tokens:
                return self.error(401, 'The access token expired')
            path = u.path.removeprefix('/v1')
            body = self.body()
            data = json.loads(body) if body.strip() else {}
            with state.lock:
                self.route(method, path, q, data)

        def route(self, method, path, q, data):
            s = state
            player = path.startswith('/me/player/') and path != '/me/player/devices'
            if player and method in ('PUT', 'POST') and s.args.free:
                return self.error(403, 'Player command failed: Premium required', 'PREMIUM_REQUIRED')
            if (method, path) == ('GET', '/me/player'):
                return self.reply(200, s.player(self.base()))
            if path == '/me/player/devices':
                return self.reply(200, {'devices': [{'id': 'd1', 'name': 'Ordinateur test',
                                                     'type': 'Computer', 'is_active': True}]})
            if path == '/me/player/play':
                if data.get('context_uri'):
                    s.context, s.index = data['context_uri'], 0 if 'artist' in data['context_uri'] else 1
                elif data.get('uris'):
                    s.context, s.index = None, int(data['uris'][0][-1]) - 1
                s.playing = True
            elif path == '/me/player/pause':
                s.playing = False
            elif path in ('/me/player/next', '/me/player/previous'):
                s.index = (s.index + (1 if path.endswith('next') else -1)) % 3
                s.progress = 0
            elif path == '/me/player/seek':
                s.progress = int(q['position_ms'])
            elif path == '/me/player/volume':
                s.volume = int(q['volume_percent'])
            elif path == '/me/player/shuffle':
                s.shuffle = q['state'] == 'true'
            elif path == '/me/player/repeat':
                s.repeat = q['state']
            elif path == '/me/library/contains':
                return self.reply(200, [u.rsplit(':', 1)[-1] in s.liked for u in q['uris'].split(',')])
            elif path == '/me/library':
                ids = {u.rsplit(':', 1)[-1] for u in q['uris'].split(',')}
                s.liked = s.liked | ids if method == 'PUT' else s.liked - ids
                return self.reply(200, None)
            elif path.startswith('/me/tracks') or path.endswith('/tracks'):
                return self.error(403, 'Forbidden')  # moved by Spotify in 2026 (library, items)
            elif path == '/me/playlists':
                items = [{'id': k, 'name': p['name'], 'uri': f'spotify:playlist:{k}'}
                         for k, p in s.playlists.items()]
                return self.reply(200, {'items': items, 'next': None})
            elif path.startswith('/playlists/') and path.endswith('/items'):
                return self.playlist(method, path.split('/')[2], data)
            elif path == '/recommendations':
                return self.error(404, 'Not found')
            else:
                return self.error(404, f'unknown {method} {path}')
            # the player commands: done, with some text that is not JSON (as Spotify since 2026)
            self.reply(200, b'ok', 'text/plain')

        def playlist(self, method, pid, data):
            p = state.playlists.get(pid)
            if p is None:
                return self.error(404, 'playlist')
            if method == 'GET':
                return self.reply(200, {'items': [{'item': {'uri': u}} for u in p['tracks']], 'next': None})
            if method == 'POST':
                p['tracks'] += data.get('uris', [])
            else:
                gone = {t['uri'] for t in data.get('items', [])}
                p['tracks'] = [u for u in p['tracks'] if u not in gone]
            print(f'  {p["name"]} : {len(p["tracks"])} morceau(x)', flush=True)
            self.reply(201 if method == 'POST' else 200, {'snapshot_id': uuid.uuid4().hex})
    return H


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--port', type=int, default=0)
    ap.add_argument('--client-id', default='abc')
    ap.add_argument('--secret', default='s3cret')
    ap.add_argument('--free', action='store_true', help='compte gratuit : lecture refusée')
    args = ap.parse_args()
    server = http.server.ThreadingHTTPServer(('127.0.0.1', args.port), handler(State(args)))
    print(f'port {server.server_address[1]}', flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == '__main__':
    main()
