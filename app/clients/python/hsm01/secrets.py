"""A module's sensitive data (tokens, passwords, keys): its `.env`, owner-only (0600).

The app gives the `.env` of the module's folder to its program as environment variables (the
fields declared by `[[secret]]` in `module.toml` are typed in the module's tab of the window).
`save` stores a value obtained while running (an OAuth token...) for the next starts, without
restarting the module. Never put such values in config.toml, the module's code or its logs.

>>> from hsm01 import secrets
>>> secrets.get('DISCORD_CLIENT_ID')
>>> secrets.save('DISCORD_REFRESH_TOKEN', token)
"""
import os
import re
from .i18n import tr

_LINE = re.compile(r'^\s*(?:export\s+)?([A-Za-z0-9_]+)\s*=\s*(.*?)\s*$')


def path():
    """`.env` of the module's folder ($HSM01_MODULE_DIR, else the current folder)."""
    return os.path.join(os.environ.get('HSM01_MODULE_DIR') or os.getcwd(), '.env')


def _unquote(v):
    if len(v) >= 2 and v[0] == v[-1] == '"':
        out, chars = [], iter(v[1:-1])
        for c in chars:
            if c == '\\':
                c = next(chars, '')
                out.append('\n' if c == 'n' else c)
            else:
                out.append(c)
        return ''.join(out)
    if len(v) >= 2 and v[0] == v[-1] == "'":
        return v[1:-1]
    return v


def read():
    """All the values of the `.env` ({} if there is none)."""
    try:
        with open(path(), encoding='utf-8') as f:
            lines = f.read().splitlines()
    except FileNotFoundError:
        return {}
    out = {}
    for line in lines:
        if line.strip().startswith('#'):
            continue
        m = _LINE.match(line)
        if m:
            out[m.group(1)] = _unquote(m.group(2))
    return out


def get(name, default=None):
    """A value: from the environment the app gave, else from the `.env` (saved meanwhile)."""
    return os.environ.get(name) or read().get(name, default)


def save(name, value):
    """Store `value` as `name` in the `.env` (empty: remove it), owner-only, atomically; also
    in this process's environment."""
    values = read()
    if value:
        values[name] = str(value)
        os.environ[name] = str(value)
    else:
        values.pop(name, None)
        os.environ.pop(name, None)
    text = tr('# Sensitive data of the module (new hera-nux): do not share.\n')
    for k, v in values.items():
        v = v.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n')
        text += f'{k}="{v}"\n'
    tmp = path() + '.tmp'
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w', encoding='utf-8') as f:
        os.fchmod(f.fileno(), 0o600)
        f.write(text)
    os.replace(tmp, path())
