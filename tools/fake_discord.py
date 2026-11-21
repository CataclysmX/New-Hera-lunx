#!/usr/bin/env python3
"""Fake Discord desktop client, to try the « Discord » module without Discord nor an account:
its local RPC (Unix socket, same frames and commands), discord.com's OAuth2 token endpoint and
the avatars (HTTP on 127.0.0.1). One server « Serveur test » (voice channels Général, Jeux), you
(Moi) in Général with Alice and Bob; Alice speaks now and then.

Usage: python3 -I tools/fake_discord.py --socket DIR/discord-ipc-0 [--port 0] [--client-id 123]
       [--secret s3cret] [--deny] [--none]
Prints `port N` once ready. Then start the module with HSM01_DISCORD_IPC=<socket>,
HSM01_DISCORD_API=http://127.0.0.1:N and HSM01_DISCORD_CDN=http://127.0.0.1:N (and its .env:
DISCORD_CLIENT_ID / DISCORD_CLIENT_SECRET = the ones above).
Control (HTTP GET): /control/join?user=chloe, /control/leave?user=bob, /control/speak?user=alice
"""
import argparse
import copy
import http.server
import json
import os
import socket
import struct
import threading
import time
import urllib.parse
import uuid
import zlib

ME = {'id': '100000000000000001', 'username': 'moi', 'global_name': 'Moi', 'avatar': 'a1',
      'discriminator': '0'}
USERS = {
    'moi': ME,
    'alice': {'id': '200000000000000002', 'username': 'alice', 'global_name': 'Alice',
              'avatar': 'b2', 'discriminator': '0'},
    'bob': {'id': '300000000000000003', 'username': 'bob', 'global_name': None,
            'avatar': None, 'discriminator': '0'},
    'chloe': {'id': '400000000000000004', 'username': 'chloe', 'global_name': 'Chloé',
              'avatar': 'c3', 'discriminator': '0'},
}
GUILD = {'id': '900', 'name': 'Serveur test'}
CHANNELS = [{'id': '901', 'name': 'Général', 'type': 2}, {'id': '902', 'name': 'Jeux', 'type': 2},
            {'id': '903', 'name': 'blabla', 'type': 0}]


class State:
    def __init__(self, args):
        self.args = args
        self.lock = threading.RLock()
        self.voice = {'input': {'volume': 80.0, 'device_id': 'default'},
                      'output': {'volume': 100.0, 'device_id': 'default'},
                      'mode': {'type': 'VOICE_ACTIVITY'}, 'mute': False, 'deaf': False}
        self.channel = None if args.none else '901'
        self.present = {'901': ['moi', 'alice', 'bob'], '902': []}
        self.local = {}  # user id -> {'volume', 'mute'} (my settings for them)
        self.self_mute = {'bob'}
        self.tokens, self.refresh = set(), set()
        self.conns = []

    def voice_state(self, name):
        u = USERS[name]
        loc = self.local.get(u['id'], {'volume': 100.0, 'mute': False})
        return {'nick': None, 'mute': loc['mute'], 'volume': loc['volume'],
                'pan': {'left': 1.0, 'right': 1.0}, 'user': u,
                'voice_state': {'mute': False, 'deaf': False, 'self_mute': name in self.self_mute,
                                'self_deaf': False, 'suppress': False}}

    def channel_obj(self, cid):
        if cid is None:
            return None
        ch = next(c for c in CHANNELS if c['id'] == cid)
        names = self.present.get(cid, [])
        return dict(ch, guild_id=GUILD['id'], voice_states=[self.voice_state(n) for n in names])

    def dispatch(self, evt, data, channel=None):
        for c in list(self.conns):
            c.dispatch(evt, data, channel)


class Conn:
    """One RPC client (the module)."""

    def __init__(self, state, sock):
        self.s, self.sock = state, sock
        self.authed = False
        self.subs = set()  # (evt, channel_id or None)
        self.write = threading.Lock()

    def send(self, op, payload):
        data = json.dumps(payload).encode()
        with self.write:
            self.sock.sendall(struct.pack('<II', op, len(data)) + data)

    def recv(self):
        head = self.exact(8)
        op, n = struct.unpack('<II', head)
        return op, json.loads(self.exact(n) or b'{}')

    def exact(self, n):
        buf = b''
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError('closed')
            buf += chunk
        return buf

    def dispatch(self, evt, data, channel=None):
        if (evt, channel) in self.subs or (evt, None) in self.subs:
            try:
                self.send(1, {'cmd': 'DISPATCH', 'evt': evt, 'data': data, 'nonce': None})
            except OSError:
                pass

    def run(self):
        try:
            op, hello = self.recv()
            if op != 0 or str(hello.get('client_id')) != self.s.args.client_id:
                self.send(2, {'code': 4000, 'message': 'Invalid Client ID'})
                return
            self.send(1, {'cmd': 'DISPATCH', 'evt': 'READY', 'nonce': None, 'data': {
                'v': 1, 'config': {'cdn_host': 'cdn.discordapp.com'}, 'user': ME}})
            with self.s.lock:
                self.s.conns.append(self)
            while True:
                op, msg = self.recv()
                if op == 3:
                    self.send(4, msg)
                elif op == 2:
                    return
                elif op == 1:
                    threading.Thread(target=self.answer, args=(msg,), daemon=True).start()
        except (OSError, ValueError, struct.error):
            pass
        finally:
            with self.s.lock:
                if self in self.s.conns:
                    self.s.conns.remove(self)
            self.sock.close()

    def answer(self, msg):
        cmd, args, nonce = msg.get('cmd'), msg.get('args') or {}, msg.get('nonce')
        try:
            data = self.command(cmd, args, msg.get('evt'))
            reply = {'cmd': cmd, 'data': data, 'evt': None, 'nonce': nonce}
        except RpcFail as e:
            reply = {'cmd': cmd, 'evt': 'ERROR', 'data': {'code': e.code, 'message': e.msg},
                     'nonce': nonce}
        print(f'{cmd} {json.dumps(args, ensure_ascii=False)} -> '
              f'{"ERROR " + reply["data"]["message"] if reply["evt"] else "ok"}', flush=True)
        self.send(1, reply)

    def command(self, cmd, args, evt):
        s = self.s
        if cmd == 'AUTHORIZE':
            if 'rpc.voice.write' not in args.get('scopes', []):
                raise RpcFail(4002, 'Invalid scopes')
            time.sleep(0.3)  # the user clicks « Autoriser »
            if s.args.deny:
                raise RpcFail(5000, 'OAuth2 Error: access_denied')
            return {'code': 'fake-code'}
        if cmd == 'AUTHENTICATE':
            if args.get('access_token') not in s.tokens:
                raise RpcFail(4009, 'Invalid access token')
            self.authed = True
            return {'user': ME, 'scopes': ['rpc', 'rpc.voice.read', 'rpc.voice.write'],
                    'application': {'id': s.args.client_id, 'name': 'Test'}}
        if not self.authed:
            raise RpcFail(4006, 'Not authenticated or invalid scope')
        with s.lock:
            return self.authed_command(cmd, args, evt)

    def authed_command(self, cmd, args, evt):
        s = self.s
        if cmd in ('SUBSCRIBE', 'UNSUBSCRIBE'):
            key = (evt, args.get('channel_id'))
            (self.subs.add if cmd == 'SUBSCRIBE' else self.subs.discard)(key)
            return {'evt': evt}
        if cmd == 'GET_VOICE_SETTINGS':
            return copy.deepcopy(s.voice)
        if cmd == 'SET_VOICE_SETTINGS':
            for k in ('mute', 'deaf'):
                if k in args:
                    s.voice[k] = bool(args[k])
            for k in ('input', 'output'):
                if 'volume' in args.get(k, {}):
                    top = 100 if k == 'input' else 200
                    s.voice[k]['volume'] = float(max(0, min(top, args[k]['volume'])))
            if 'type' in args.get('mode', {}):
                s.voice['mode']['type'] = args['mode']['type']
            s.dispatch('VOICE_SETTINGS_UPDATE', copy.deepcopy(s.voice))
            return copy.deepcopy(s.voice)
        if cmd == 'GET_SELECTED_VOICE_CHANNEL':
            return s.channel_obj(s.channel)
        if cmd == 'GET_CHANNEL':
            return s.channel_obj(args['channel_id'])
        if cmd == 'SELECT_VOICE_CHANNEL':
            old, new = s.channel, args.get('channel_id')
            if old and new and not args.get('force'):
                raise RpcFail(5003, 'Already connected to a voice channel')
            if old:
                s.present[old].remove('moi')
            if new:
                s.present.setdefault(new, []).insert(0, 'moi')
            s.channel = new
            s.dispatch('VOICE_CHANNEL_SELECT', {'channel_id': new, 'guild_id': GUILD['id']})
            return s.channel_obj(new)
        if cmd == 'GET_GUILDS':
            return {'guilds': [dict(GUILD, icon_url=None)]}
        if cmd == 'GET_GUILD':
            return dict(GUILD, icon_url=None, members=[])
        if cmd == 'GET_CHANNELS':
            return {'channels': [{'id': c['id'], 'name': c['name'], 'type': c['type']}
                                 for c in CHANNELS]}
        if cmd == 'SET_USER_VOICE_SETTINGS':
            uid = args['user_id']
            loc = s.local.setdefault(uid, {'volume': 100.0, 'mute': False})
            if 'volume' in args:
                loc['volume'] = float(max(0, min(200, args['volume'])))
            if 'mute' in args:
                loc['mute'] = bool(args['mute'])
            name = next(n for n, u in USERS.items() if u['id'] == uid)
            if s.channel and name in s.present.get(s.channel, []):
                s.dispatch('VOICE_STATE_UPDATE', s.voice_state(name), s.channel)
            return dict(loc, user_id=uid, pan={'left': 1.0, 'right': 1.0})
        raise RpcFail(4000, f'Unknown command {cmd}')


class RpcFail(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code, self.msg = code, msg


def png(rgb, size=64):
    """A plain PNG of colour `rgb` (an avatar)."""
    raw = b''.join(b'\x00' + bytes(rgb) * size for _ in range(size))

    def chunk(kind, data):
        return (struct.pack('>I', len(data)) + kind + data
                + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff))
    head = struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', head) + chunk(b'IDAT', zlib.compress(raw))
            + chunk(b'IEND', b''))


def http_handler(state):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def reply(self, code, body, kind='application/json'):
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(code)
            self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self):
            n = int(self.headers.get('Content-Length') or 0)
            form = dict(urllib.parse.parse_qsl(self.rfile.read(n).decode()))
            a = state.args
            if self.path != '/oauth2/token':
                return self.reply(404, {'error': 'not_found'})
            if form.get('client_id') != a.client_id or form.get('client_secret') != a.secret:
                return self.reply(401, {'error': 'invalid_client'})
            grant = form.get('grant_type')
            ok = (grant == 'authorization_code' and form.get('code') == 'fake-code'
                  and form.get('redirect_uri') == 'http://localhost') or \
                 (grant == 'refresh_token' and form.get('refresh_token') in state.refresh)
            if not ok:
                return self.reply(400, {'error': 'invalid_grant',
                                        'error_description': 'Invalid "code" in request.'})
            access, refresh = uuid.uuid4().hex, uuid.uuid4().hex
            state.tokens.add(access)
            state.refresh.add(refresh)
            print(f'token ({grant}) delivered', flush=True)
            self.reply(200, {'access_token': access, 'refresh_token': refresh,
                             'expires_in': 604800, 'token_type': 'Bearer',
                             'scope': 'rpc rpc.voice.read rpc.voice.write'})

        def do_GET(self):
            url = urllib.parse.urlparse(self.path)
            if url.path.endswith('.png'):
                seed = zlib.crc32(url.path.encode())
                return self.reply(200, png((seed & 255, seed >> 8 & 255, seed >> 16 & 255)),
                                  'image/png')
            q = dict(urllib.parse.parse_qsl(url.query))
            name = q.get('user', '')
            with state.lock:
                ch = state.channel
                if name not in USERS or ch is None:
                    return self.reply(400, {'error': 'user or channel'})
                here = state.present[ch]
                if url.path == '/control/join' and name not in here:
                    here.append(name)
                    state.dispatch('VOICE_STATE_CREATE', state.voice_state(name), ch)
                elif url.path == '/control/leave' and name in here:
                    here.remove(name)
                    state.dispatch('VOICE_STATE_DELETE', state.voice_state(name), ch)
                elif url.path == '/control/speak':
                    uid = USERS[name]['id']
                    state.dispatch('SPEAKING_START', {'channel_id': ch, 'user_id': uid}, ch)
            self.reply(200, {'ok': True})
    return Handler


def speaker(state):
    """Alice speaks for a second every three."""
    on = False
    while True:
        time.sleep(1.5)
        with state.lock:
            ch = state.channel
            if ch and 'alice' in state.present.get(ch, []):
                on = not on
                evt = 'SPEAKING_START' if on else 'SPEAKING_STOP'
                state.dispatch(evt, {'channel_id': ch, 'user_id': USERS['alice']['id']}, ch)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--socket', required=True)
    ap.add_argument('--port', type=int, default=0)
    ap.add_argument('--client-id', default='123')
    ap.add_argument('--secret', default='s3cret')
    ap.add_argument('--deny', action='store_true', help='refuser l\'autorisation')
    ap.add_argument('--none', action='store_true', help='dans aucun salon au départ')
    args = ap.parse_args()
    state = State(args)
    web = http.server.ThreadingHTTPServer(('127.0.0.1', args.port), http_handler(state))
    threading.Thread(target=web.serve_forever, daemon=True).start()
    if os.path.exists(args.socket):
        os.unlink(args.socket)
    srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    srv.bind(args.socket)
    srv.listen()
    threading.Thread(target=speaker, args=(state,), daemon=True).start()
    print(f'port {web.server_address[1]}', flush=True)
    try:
        while True:
            conn, _ = srv.accept()
            threading.Thread(target=Conn(state, conn).run, daemon=True).start()
    except KeyboardInterrupt:
        pass
    finally:
        os.unlink(args.socket)


if __name__ == '__main__':
    main()
