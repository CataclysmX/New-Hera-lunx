"""A game's telemetry received by UDP, for the dashboard modules (Motor Town, SimHub): the
listening IP and port checked, each datagram relayed as it is to other IP:port (a game sends to
one destination only: the SimHub program, another PC, another tool can still get it).

>>> rx = Receiver('0.0.0.0', 33330, relays=[('127.0.0.1', 9999)])   # OSError: port taken…
>>> data, addr = rx.recv(timeout=1.0)          # None on timeout; relayed meanwhile

`settings(data, default_port)` reads `ecoute`, `port` and `[[relais]]` (`ip`, `port`) of a
module's settings, with the problems found in words (shown on its page).
"""
import ipaddress
import socket
from .i18n import tr

ANY, LOCAL = '0.0.0.0', '127.0.0.1'
RELAY_PORT = 9999  # a relay's port when the form left it at its default


def ip(text, default=ANY):
    """An IPv4 address, or ValueError in words."""
    text = str(text or '').strip() or default
    if text == 'localhost':
        return LOCAL
    try:
        return str(ipaddress.IPv4Address(text))
    except ValueError:
        raise ValueError(tr('"{text}" is not an IPv4 address (0.0.0.0, 127.0.0.1, 192.168.1.20…)', text=text)) from None


def port(value, default):
    """A UDP port 1024..65535 (the range games accept), or ValueError in words."""
    if value in (None, ''):
        return default
    try:
        p = int(value)
    except (TypeError, ValueError):
        raise ValueError(tr('port "{value}": a number between 1024 and 65535', value=value)) from None
    if not 1024 <= p <= 65535:
        raise ValueError(tr('port {p}: between 1024 and 65535', p=p))
    return p


def this_pc():
    """This computer's address on the network (the one a game on another PC sends to), or ''."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(('192.0.2.1', 9))  # a documentation address: nothing is sent
        return s.getsockname()[0]
    except OSError:
        return ''
    finally:
        s.close()


def settings(data, default_port, listen_key='ecoute', fallback_keys=()):
    """(listen ip, port, relays [(ip, port)], problems [text]) from a module's settings table;
    a wrong value: its default, and the problem said."""
    problems = []
    raw_ip = data.get(listen_key)
    for k in fallback_keys:  # former key names
        raw_ip = raw_ip if raw_ip not in (None, '') else data.get(k)
    try:
        host = ip(raw_ip)
    except ValueError as e:
        problems.append(tr('listening: {e}', e=e))
        host = ANY
    try:
        p = port(data.get('port'), default_port)
    except ValueError as e:
        problems.append(str(e))
        p = default_port
    relays = []
    for r in data.get('relais') or []:
        if not isinstance(r, dict):
            continue
        try:  # the form's defaults (shown, not written until changed): 127.0.0.1:9999
            dest = (ip(r.get('ip'), LOCAL), port(r.get('port'), RELAY_PORT))
        except ValueError as e:
            problems.append(tr('relay: {e}', e=e))
            continue
        if dest[1] == p and (dest[0] in (LOCAL, ANY, this_pc()) or dest[0].startswith('127.')):
            problems.append(tr("relay {dest}: it is the module's own listening address (loop), ignored", dest=f'{dest[0]}:{dest[1]}'))
            continue
        if dest not in relays:
            relays.append(dest)
    return host, p, relays, problems


class Receiver:
    """The socket listening on (host, port); `recv` gives each datagram and relays it."""

    def __init__(self, host, port, relays=()):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            self.sock.bind((host, port))
        except OSError:
            self.sock.close()
            raise
        self.out = socket.socket(socket.AF_INET, socket.SOCK_DGRAM) if relays else None
        self.relays, self.relayed, self.errors = list(relays), 0, 0

    def recv(self, timeout=1.0):
        """(data, (ip, port)) of the next datagram, None after `timeout` s without one."""
        self.sock.settimeout(timeout)
        try:
            data, addr = self.sock.recvfrom(65535)
        except socket.timeout:
            return None
        for dest in self.relays:
            try:
                self.out.sendto(data, dest)
                self.relayed += 1
            except OSError:  # nobody there (ICMP refused on loopback…): next datagram anyway
                self.errors += 1
        return data, addr

    def close(self):
        for s in (self.sock, self.out):
            if s is not None:
                s.close()
