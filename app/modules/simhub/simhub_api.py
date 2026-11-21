"""SimHub's web API (its dashboard server, port 8888 by default): one property a request,
`GET /api/getproperty/<name>` (`DataCorePlugin.GameData.SpeedKmh`…), standard library only, on
one kept-alive connection.

The answer's form is read leniently: JSON (number, text, true / false, null), else plain text
(a number, True / False, a duration hh:mm:ss.fffffff). Tests: HSM01_SIMHUB_URL (base address).
"""
import http.client
import json
import re
import urllib.parse
from hsm01.i18n import tr

DURATION = re.compile(r'^(-)?(?:(\d+)\.)?(\d+):(\d+):(\d+(?:[.,]\d+)?)$')


class SimHubError(Exception):
    """SimHub unreachable (status 0) or refusing (HTTP status)."""

    def __init__(self, status, message):
        super().__init__(f'{message} (HTTP {status})' if status else message)
        self.status = status


def value(text):
    """A property's value from SimHub's answer: number, bool, seconds (durations), text, None."""
    text = text.strip()
    if not text:
        return None
    try:
        v = json.loads(text)
    except ValueError:
        v = text
    if isinstance(v, str):
        t = v.strip()
        if t.lower() in ('true', 'false'):
            return t.lower() == 'true'
        if t.lower() in ('null', 'none'):
            return None
        m = DURATION.match(t)
        if m:
            neg, days, h, mi, s = m.groups()
            secs = int(days or 0) * 86400 + int(h) * 3600 + int(mi) * 60 + float(s.replace(',', '.'))
            return -secs if neg else secs
        try:
            return float(t.replace(',', '.'))
        except ValueError:
            return t
    return v


class SimHub:
    def __init__(self, base='http://127.0.0.1:8888', timeout=2.0):
        u = urllib.parse.urlsplit(base)
        self.host, self.port = u.hostname or '127.0.0.1', u.port or 8888
        self.prefix = u.path.rstrip('/')
        self.timeout, self.conn = timeout, None

    def get(self, name):
        """The value of property `name`; SimHubError when SimHub is unreachable or refuses."""
        path = f'{self.prefix}/api/getproperty/{urllib.parse.quote(name)}'
        for attempt in (1, 2):  # a kept-alive connection SimHub closed: once again on a new one
            if self.conn is None:
                self.conn = http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)
            try:
                self.conn.request('GET', path, headers={'Accept': 'application/json, text/plain'})
                r = self.conn.getresponse()
                body = r.read(65536).decode('utf-8', 'replace')
            except (OSError, http.client.HTTPException) as e:
                self.close()
                if attempt == 2:
                    raise SimHubError(0, tr('SimHub unreachable at {address} ({e})', address=f'{self.host}:{self.port}', e=e)) from None
                continue
            if r.status != 200:
                raise SimHubError(r.status, body.strip()[:80] or r.reason)
            return value(body)

    def close(self):
        if self.conn is not None:
            self.conn.close()
            self.conn = None
