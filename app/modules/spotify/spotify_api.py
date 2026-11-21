"""Spotify's Web API for the « Spotify » module, standard library only: the OAuth2 authorization
code flow (the user's browser, a one-shot local server for the redirect), the token and its
refresh, the calls with their errors (Premium required, no active device, rate limit).

Tests: HSM01_SPOTIFY_ACCOUNTS / HSM01_SPOTIFY_API (base URLs of accounts and API).
"""
import base64
import http.server
import json
import os
import urllib.error
import urllib.parse
import urllib.request
from hsm01.i18n import tr

ACCOUNTS = os.environ.get('HSM01_SPOTIFY_ACCOUNTS') or 'https://accounts.spotify.com'
API = os.environ.get('HSM01_SPOTIFY_API') or 'https://api.spotify.com/v1'
SCOPES = ['user-read-playback-state', 'user-modify-playback-state',
          'user-read-currently-playing', 'user-library-read', 'user-library-modify',
          'playlist-read-private', 'playlist-modify-private', 'playlist-modify-public']
USER_AGENT = 'hsm01-spotify/1.0 (new hera-nux)'


class ApiError(Exception):
    """Spotify refused a call: HTTP `status`, its `reason` (PREMIUM_REQUIRED,
    NO_ACTIVE_DEVICE…), `retry` seconds when rate limited; `where`: the call ('GET /me/player')."""

    def __init__(self, status, message, reason='', retry=0):
        super().__init__(f'{message} (HTTP {status})' if status else message)
        self.status, self.reason, self.retry, self.where = status, reason, retry, ''


def authorize_url(client_id, redirect, state):
    q = urllib.parse.urlencode({'client_id': client_id, 'response_type': 'code',
                                'redirect_uri': redirect, 'scope': ' '.join(SCOPES),
                                'state': state})
    return f'{ACCOUNTS}/authorize?{q}'


def wait_code(redirect, state, timeout=300, on_ready=None):
    """Serve the redirect address once (127.0.0.1:port/path): the code Spotify sends back after
    the user agreed, or None (refused, other request, nothing in time). `on_ready()` once it
    listens (open the browser then)."""
    url = urllib.parse.urlparse(redirect)
    got = {}

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_GET(self):
            u = urllib.parse.urlparse(self.path)
            q = dict(urllib.parse.parse_qsl(u.query))
            ok = u.path == (url.path or '/') and q.get('state') == state and 'code' in q
            if ok:
                got['code'] = q['code']
            text = (tr("That's it: new hera-nux is connected to Spotify, you can close this tab.")
                    if ok else tr('Spotify: {error}.', error=q.get('error', tr('unexpected answer'))))
            body = f'<html><meta charset="utf-8"><body><p>{text}</p></body></html>'.encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    server = http.server.HTTPServer((url.hostname or '127.0.0.1', url.port or 80), Handler)
    server.timeout = timeout
    try:
        if on_ready:
            on_ready()
        server.handle_request()
    finally:
        server.server_close()
    return got.get('code')


def token(client_id, secret, **fields):
    """OAuth2 token: fields = code + redirect_uri, or refresh_token. Returns {access_token,
    expires_in, refresh_token (not always on a refresh)…}."""
    grant = 'refresh_token' if 'refresh_token' in fields else 'authorization_code'
    body = urllib.parse.urlencode(dict(fields, grant_type=grant)).encode()
    basic = base64.b64encode(f'{client_id}:{secret}'.encode()).decode()
    req = urllib.request.Request(f'{ACCOUNTS}/api/token', data=body, method='POST', headers={
        'Authorization': f'Basic {basic}', 'User-Agent': USER_AGENT,
        'Content-Type': 'application/x-www-form-urlencoded'})
    return _send(req)


def _send(req):
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            data = r.read()
        try:
            return json.loads(data) if data.strip() else None
        except ValueError:
            return None  # done; the player commands answer some text, not JSON, since 2026
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            err = json.loads(raw)
        except ValueError:
            err = {}
        inner = err.get('error')
        if isinstance(inner, dict):  # the API: {"error": {"status", "message", "reason"}}
            message, reason = inner.get('message') or str(e), inner.get('reason', '')
        else:  # accounts: {"error": "invalid_grant", "error_description": …}
            message, reason = err.get('error_description') or inner or str(e), inner or ''
        retry = int(e.headers.get('Retry-After') or 0) if e.code == 429 else 0
        raise ApiError(e.code, message, reason, retry) from None
    except (urllib.error.URLError, OSError, ValueError) as e:
        raise ApiError(0, tr('Spotify unreachable: {e}', e=e)) from None


def call(access, method, path, query=None, body=None):
    """One call of the Web API (`path` after /v1); its JSON, or None (204, empty)."""
    url = f'{API}{path}'
    if query:
        url += '?' + urllib.parse.urlencode(query)
    data = json.dumps(body).encode() if body is not None else (b'' if method in ('PUT', 'POST') else None)
    headers = {'Authorization': f'Bearer {access}', 'User-Agent': USER_AGENT}
    if body is not None:
        headers['Content-Type'] = 'application/json'
    try:
        return _send(urllib.request.Request(url, data=data, method=method, headers=headers))
    except ApiError as e:
        e.where = f'{method} {path}'
        raise
