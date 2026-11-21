"""OBS Studio's WebSocket server (obs-websocket v5, built into OBS 28 and later: Tools > WebSocket
Server Settings), standard library only: a small WebSocket client (RFC 6455, text messages) and
the OBS protocol on it.

>>> obs = Obs('127.0.0.1', 4455, password, on_event=print)   # Hello, Identify, Identified
>>> obs.request('GetSceneList')['currentProgramSceneName']
>>> obs.request('SetCurrentProgramScene', {'sceneName': 'Jeu'})

Messages: {"op": N, "d": {...}}; op 0 Hello, 1 Identify, 2 Identified, 5 Event, 6 Request,
7 RequestResponse. Password: base64(sha256(base64(sha256(password + salt)) + challenge)).
"""
import base64
import hashlib
import json
import os
import socket
import struct
import threading
import traceback
import uuid
from hsm01.i18n import tr

GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
OP_CONT, OP_TEXT, OP_BINARY, OP_CLOSE, OP_PING, OP_PONG = 0x0, 0x1, 0x2, 0x8, 0x9, 0xA
# event subscriptions: every usual category (general … ui), and the audio meters (20 a second)
ALL_EVENTS = (1 << 11) - 1
VOLUME_METERS = 1 << 16
# close codes of obs-websocket: no or wrong password, protocol too old or too new
AUTH_FAILED, UNSUPPORTED_RPC = 4009, 4010


class ObsError(Exception):
    """OBS refused a request (`code`: its status, the message: its comment), or the connection
    failed (code 0); `close`: the WebSocket close code when OBS closed it (4009: password)."""

    def __init__(self, code, message, close=None):
        super().__init__(f'{message} ({code})' if code else message)
        self.code, self.close = code, close


def answer(password, salt, challenge):
    """The `authentication` of Identify for OBS's `salt` and `challenge`."""
    secret = base64.b64encode(hashlib.sha256((password + salt).encode()).digest()).decode()
    return base64.b64encode(hashlib.sha256((secret + challenge).encode()).digest()).decode()


def _mask(data, key):
    n = len(data)
    if not n:
        return b''
    k = (key * (n // 4 + 1))[:n]
    return (int.from_bytes(data, 'big') ^ int.from_bytes(k, 'big')).to_bytes(n, 'big')


class WebSocket:
    """Client end of a WebSocket: `send(text)`; `recv()` returns the next text message (pings
    answered); ConnectionError at the end, the close code in `close_code`."""

    def __init__(self, host, port, path='/', protocol=None, timeout=5.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)
        self._write = threading.Lock()
        self.buf, self.close_code, self.close_reason = b'', None, ''
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [f'GET {path} HTTP/1.1', f'Host: {host}:{port}', 'Upgrade: websocket',
                 'Connection: Upgrade', f'Sec-WebSocket-Key: {key}', 'Sec-WebSocket-Version: 13']
        if protocol:
            lines.append(f'Sec-WebSocket-Protocol: {protocol}')
        try:
            self.sock.sendall(('\r\n'.join(lines) + '\r\n\r\n').encode())
            while b'\r\n\r\n' not in self.buf:
                if len(self.buf) > 16384:
                    raise ConnectionError(tr('HTTP answer too long'))
                self._fill()
            head, self.buf = self.buf.split(b'\r\n\r\n', 1)
            status, *headers = head.decode('latin-1').split('\r\n')
            fields = {}
            for h in headers:
                k, _, v = h.partition(':')
                fields[k.strip().lower()] = v.strip()
            accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
            if status.split()[1:2] != ['101'] or fields.get('sec-websocket-accept') != accept:
                raise ConnectionError(tr('not a WebSocket server ({status})', status=status.strip()))
        except BaseException:
            self.sock.close()
            raise

    def _fill(self):
        chunk = self.sock.recv(65536)
        if not chunk:
            raise ConnectionError(tr('connection closed'))
        self.buf += chunk

    def _take(self, n):
        while len(self.buf) < n:
            self._fill()
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def _send(self, op, data):
        n = len(data)
        if n < 126:
            head = struct.pack('>BB', 0x80 | op, 0x80 | n)
        elif n < 1 << 16:
            head = struct.pack('>BBH', 0x80 | op, 0x80 | 126, n)
        else:
            head = struct.pack('>BBQ', 0x80 | op, 0x80 | 127, n)
        key = os.urandom(4)  # a client masks what it sends
        with self._write:
            self.sock.sendall(head + key + _mask(data, key))

    def send(self, text):
        self._send(OP_TEXT, text.encode())

    def recv(self):
        parts, started = [], False
        while True:
            b0, b1 = self._take(2)
            fin, op, n = b0 & 0x80, b0 & 0x0F, b1 & 0x7F
            if n == 126:
                n = struct.unpack('>H', self._take(2))[0]
            elif n == 127:
                n = struct.unpack('>Q', self._take(8))[0]
            key = self._take(4) if b1 & 0x80 else None
            data = self._take(n)
            if key:
                data = _mask(data, key)
            if op == OP_PING:
                self._send(OP_PONG, data)
            elif op == OP_CLOSE:
                self.close_code = struct.unpack('>H', data[:2])[0] if len(data) >= 2 else 1005
                self.close_reason = data[2:].decode('utf-8', 'replace')
                try:
                    self._send(OP_CLOSE, data[:2])
                except OSError:
                    pass
                raise ConnectionError(tr('closed by OBS ({code} {reason})', code=self.close_code, reason=self.close_reason).strip())
            elif op in (OP_TEXT, OP_BINARY) or (op == OP_CONT and started):
                if op != OP_CONT:
                    parts, started = [], True
                parts.append(data)
                if fin:
                    return b''.join(parts).decode('utf-8')

    def close(self, code=1000):
        try:
            self._send(OP_CLOSE, struct.pack('>H', code))
        except OSError:
            pass
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.sock.close()


class Obs:
    """One connection to OBS. `on_event(type, data)` is called from the reader thread for each
    event (and ('CLOSED', {'code', 'message'}) once at the end): it must not block."""

    def __init__(self, host, port, password, on_event, events=ALL_EVENTS, timeout=5.0):
        self.on_event = on_event
        try:
            self.ws = WebSocket(host, port, protocol='obswebsocket.json', timeout=timeout)
        except OSError as e:
            raise ObsError(0, tr('OBS unreachable at {address} ({e})', address=f'{host}:{port}', e=e)) from None
        self._pending, self._lock, self.closed = {}, threading.Lock(), False
        try:
            hello = self._expect(0)
            ident = {'rpcVersion': 1, 'eventSubscriptions': events}
            auth = hello.get('authentication')
            if auth:
                if not password:
                    raise ObsError(0, tr('OBS asks for a password'), close=AUTH_FAILED)
                ident['authentication'] = answer(password, auth.get('salt', ''),
                                                 auth.get('challenge', ''))
            self.ws.send(json.dumps({'op': 1, 'd': ident}))
            self.identified = self._expect(2)
        except ConnectionError as e:
            self.ws.sock.close()
            if self.ws.close_code == AUTH_FAILED:
                raise ObsError(0, tr('password refused by OBS'), close=AUTH_FAILED) from None
            raise ObsError(0, tr('OBS refused the connection ({e})', e=e), close=self.ws.close_code) from None
        except (OSError, ValueError) as e:
            self.ws.sock.close()
            raise ObsError(0, tr('no answer from OBS ({e})', e=e)) from None
        except ObsError:
            self.ws.sock.close()
            raise
        self.version = hello.get('obsWebSocketVersion', '')
        self.ws.sock.settimeout(None)
        threading.Thread(target=self._read, name='obs-ws', daemon=True).start()

    def _expect(self, op):
        while True:
            msg = json.loads(self.ws.recv())
            if msg.get('op') == op:
                return msg.get('d') or {}

    def _read(self):
        reason = tr('closed')
        try:
            while True:
                msg = json.loads(self.ws.recv())
                op, d = msg.get('op'), msg.get('d') or {}
                if op == 5:
                    try:
                        self.on_event(d.get('eventType', ''), d.get('eventData') or {})
                    except Exception:  # a handler's bug must not close the connection
                        traceback.print_exc()
                elif op == 7:
                    with self._lock:
                        box = self._pending.get(d.get('requestId'))
                    if box:
                        box[1] = d
                        box[0].set()
        except (OSError, ValueError) as e:
            reason = str(e) or reason
        self.closed = True
        with self._lock:
            for box in self._pending.values():
                box[0].set()
        self.on_event('CLOSED', {'code': self.ws.close_code, 'message': reason})

    def request(self, kind, data=None, timeout=5.0):
        """Send request `kind` and return its `responseData` (ObsError when refused)."""
        if self.closed:
            raise ObsError(0, tr('connection to OBS closed'))
        rid = uuid.uuid4().hex
        box = [threading.Event(), None]
        with self._lock:
            self._pending[rid] = box
        try:
            d = {'requestType': kind, 'requestId': rid}
            if data is not None:
                d['requestData'] = data
            self.ws.send(json.dumps({'op': 6, 'd': d}))
            if not box[0].wait(timeout):
                raise ObsError(0, tr('{kind}: no answer from OBS', kind=kind))
        except OSError as e:
            raise ObsError(0, f'{kind}: {e}') from None
        finally:
            with self._lock:
                self._pending.pop(rid, None)
        reply = box[1]
        if reply is None:
            raise ObsError(0, tr('connection to OBS closed'))
        st = reply.get('requestStatus') or {}
        if not st.get('result'):
            raise ObsError(st.get('code', -1), st.get('comment') or kind)
        return reply.get('responseData') or {}

    def close(self):
        self.closed = True
        self.ws.close()
