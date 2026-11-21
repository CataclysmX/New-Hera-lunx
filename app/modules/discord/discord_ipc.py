"""Discord's local RPC (the desktop client's Unix socket) and its OAuth2 token, standard library
only. Frames: [op u32 LE][length u32 LE][JSON]; op 0 handshake, 1 frame, 2 close, 3 ping, 4 pong.

>>> rpc = Rpc(client_id, on_event=print)   # connects, handshake: Discord answers READY
>>> code = rpc.request('AUTHORIZE', {'client_id': client_id, 'scopes': SCOPES}, timeout=180)
>>> rpc.request('AUTHENTICATE', {'access_token': token(...)['access_token']})
>>> rpc.request('SET_VOICE_SETTINGS', {'mute': True})

Tests: HSM01_DISCORD_IPC (socket path), HSM01_DISCORD_API (token endpoint base URL).
"""
import json
import os
import socket
import struct
import threading
import urllib.error
import urllib.parse
import urllib.request
import uuid
from hsm01.i18n import tr

OP_HANDSHAKE, OP_FRAME, OP_CLOSE, OP_PING, OP_PONG = range(5)
SCOPES = ['rpc', 'rpc.voice.read', 'rpc.voice.write']
API = os.environ.get('HSM01_DISCORD_API') or 'https://discord.com/api'
USER_AGENT = 'hsm01-discord/1.0 (new hera-nux)'
# where the clients open their socket: package / AUR, Flatpak, Snap, Canary, PTB
SUBDIRS = ['', 'app/com.discordapp.Discord', 'app/com.discordapp.DiscordCanary',
           '.flatpak/com.discordapp.Discord/xdg-run', 'snap.discord', 'snap.discord-canary']


class RpcError(Exception):
    """Discord refused a command (`code`, message), or the connection went away (code 0)."""

    def __init__(self, code, message):
        super().__init__(f'{message} ({code})' if code else message)
        self.code = code


def socket_paths():
    if os.environ.get('HSM01_DISCORD_IPC'):
        return [os.environ['HSM01_DISCORD_IPC']]
    bases = [os.environ.get(k) for k in ('XDG_RUNTIME_DIR', 'TMPDIR', 'TMP', 'TEMP')] + ['/tmp']
    seen, out = set(), []
    for base in filter(None, bases):
        for sub in SUBDIRS:
            for i in range(10):
                p = os.path.join(base, sub, f'discord-ipc-{i}')
                if p not in seen:
                    seen.add(p)
                    out.append(p)
    return out


class Rpc:
    """One connection to the Discord client. `on_event(evt, data)` is called from the reader
    thread for each DISPATCH (and ('CLOSED', reason) once at the end): it must not block."""

    def __init__(self, client_id, on_event):
        self.on_event = on_event
        self.sock = self._open()
        self._write = threading.Lock()
        self._pending = {}  # nonce -> [event, reply]
        self._lock = threading.Lock()
        self.closed = False
        self.sock.settimeout(10)  # a client that never answers the handshake
        try:
            self._send(OP_HANDSHAKE, {'v': 1, 'client_id': str(client_id)})
            op, msg = self._recv()
        except (OSError, ValueError, struct.error) as e:
            self.sock.close()
            raise RpcError(0, tr('no answer from Discord ({e})', e=e)) from None
        if op == OP_CLOSE or msg.get('evt') != 'READY':
            self.sock.close()
            raise RpcError(msg.get('code', 0), msg.get('message') or tr('handshake refused'))
        self.sock.settimeout(None)
        self.ready = msg.get('data') or {}
        threading.Thread(target=self._read, name='discord-ipc', daemon=True).start()

    @staticmethod
    def _open():
        """The first Discord socket that accepts a connection."""
        for path in socket_paths():
            if not os.path.exists(path):
                continue
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                s.connect(path)
                return s
            except OSError:
                s.close()
        raise FileNotFoundError(tr('Discord is not running (no discord-ipc socket)'))

    def _send(self, op, payload):
        data = json.dumps(payload).encode()
        with self._write:
            self.sock.sendall(struct.pack('<II', op, len(data)) + data)

    def _exact(self, n):
        buf = b''
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise ConnectionError(tr('Discord closed the connection'))
            buf += chunk
        return buf

    def _recv(self):
        op, length = struct.unpack('<II', self._exact(8))
        return op, json.loads(self._exact(length) or b'{}')

    def _read(self):
        reason = tr('closed')
        try:
            while True:
                op, msg = self._recv()
                if op == OP_PING:
                    self._send(OP_PONG, msg)
                elif op == OP_CLOSE:
                    reason = msg.get('message') or reason
                    break
                elif msg.get('nonce') and msg.get('cmd') != 'DISPATCH':
                    with self._lock:
                        box = self._pending.get(msg['nonce'])
                    if box:
                        box[1] = msg
                        box[0].set()
                elif msg.get('cmd') == 'DISPATCH':
                    self.on_event(msg.get('evt'), msg.get('data') or {})
        except (OSError, ValueError, struct.error) as e:
            reason = str(e) or reason
        self.closed = True
        with self._lock:
            for box in self._pending.values():
                box[0].set()
        self.on_event('CLOSED', {'message': reason})

    def request(self, cmd, args=None, evt=None, timeout=5.0):
        """Send command `cmd` and return its `data` (raises RpcError when refused)."""
        if self.closed:
            raise RpcError(0, tr('connection to Discord closed'))
        nonce = str(uuid.uuid4())
        box = [threading.Event(), None]
        with self._lock:
            self._pending[nonce] = box
        try:
            msg = {'cmd': cmd, 'args': args or {}, 'nonce': nonce}
            if evt:
                msg['evt'] = evt
            self._send(OP_FRAME, msg)
            if not box[0].wait(timeout):
                raise RpcError(0, tr('{cmd}: no answer from Discord', cmd=cmd))
        except OSError as e:
            raise RpcError(0, f'{cmd}: {e}') from e
        finally:
            with self._lock:
                self._pending.pop(nonce, None)
        reply = box[1]
        if reply is None:
            raise RpcError(0, tr('connection to Discord closed'))
        if reply.get('evt') == 'ERROR':
            data = reply.get('data') or {}
            raise RpcError(data.get('code', -1), data.get('message') or tr('error'))
        return reply.get('data') or {}

    def subscribe(self, evt, args=None):
        return self.request('SUBSCRIBE', args, evt=evt)

    def unsubscribe(self, evt, args=None):
        return self.request('UNSUBSCRIBE', args, evt=evt)

    def close(self):
        self.closed = True
        try:
            self._send(OP_CLOSE, {})
        except OSError:
            pass
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
            self.sock.close()
        except OSError:
            pass


class AuthError(Exception):
    """discord.com refused the code or the refresh token (its own words in the message)."""


def token(client_id, client_secret, **fields):
    """OAuth2 token: fields = code + redirect_uri (grant authorization_code) or refresh_token
    (grant refresh_token). Returns {access_token, refresh_token, expires_in, scope…}."""
    grant = 'refresh_token' if 'refresh_token' in fields else 'authorization_code'
    body = urllib.parse.urlencode(dict(fields, client_id=client_id, client_secret=client_secret,
                                       grant_type=grant)).encode()
    req = urllib.request.Request(f'{API}/oauth2/token', data=body, method='POST', headers={
        'Content-Type': 'application/x-www-form-urlencoded', 'User-Agent': USER_AGENT,
        'Accept': 'application/json'})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        try:
            err = json.loads(e.read())
            text = err.get('error_description') or err.get('error') or str(e)
        except ValueError:
            text = str(e)
        raise AuthError(f'{text} (HTTP {e.code})') from None
    except (urllib.error.URLError, OSError, ValueError) as e:
        raise AuthError(tr('discord.com unreachable: {e}', e=e)) from None
