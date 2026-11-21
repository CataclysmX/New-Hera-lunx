"""Connection to new hera-nux (open API, docs/app/api.md): requests, replies and events.

One JSON object per line on the Unix socket $XDG_RUNTIME_DIR/hsm01/api.sock. A reader thread
receives the lines: replies wake up `call`, events go to a dispatcher thread that runs the
handlers registered with `on` as soon as they arrive (also in an interactive session, without
`run`). Handlers and `run`'s `tick` never run at the same time (`client.lock`).
"""
import itertools
import json
import os
import queue
import socket
import threading
import time


def socket_path():
    """$HSM01_API_SOCKET, else $XDG_RUNTIME_DIR/hsm01/api.sock."""
    if os.environ.get('HSM01_API_SOCKET'):
        return os.environ['HSM01_API_SOCKET']
    base = os.environ.get('XDG_RUNTIME_DIR') or '/tmp'
    return os.path.join(base, 'hsm01', 'api.sock')


class ApiError(Exception):
    """The app refused a request (bad field, unknown page...)."""


class Client:
    """A program talking to the app. `name` is shown in the app's messages.

    >>> c = Client('my-tool')
    >>> c.on('turn', print)            # every knob turn, whatever the page ('mine': for you?)
    >>> c.alert('Hello', 'from Python', icon='dialog-information')
    """

    def __init__(self, name, path=None, timeout=5.0):
        # a tool started by the app is known by its name there (action « afficher un module »)
        name = os.environ.get('HSM01_MODULE') or os.environ.get('HSM01_TOOL') or name
        self.timeout = timeout
        self.lock = threading.RLock()  # held while a handler or `tick` runs
        self._sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            self._sock.connect(path or socket_path())
        except OSError as e:
            raise ConnectionError(f'{path or socket_path()}: {e} (is new hera-nux running?)') from e
        self._write = threading.Lock()
        self._ids = itertools.count(1)
        self._replies = {}
        self._replies_lock = threading.Lock()
        self._events = queue.Queue()
        self._handlers = {'command': [self._on_command]}
        self._commands = {}  # id -> (description sent to the app, handler)
        self.closed = False
        threading.Thread(target=self._read, name='hsm01-read', daemon=True).start()
        threading.Thread(target=self._dispatcher, name='hsm01-events', daemon=True).start()
        self.welcome = self.call({'type': 'hello', 'name': name})

    # --- low level -----------------------------------------------------------------------
    def post(self, msg):
        """Send a request without waiting (errors come back as 'error' events)."""
        data = (json.dumps(msg, ensure_ascii=False) + '\n').encode()
        with self._write:
            self._sock.sendall(data)

    def call(self, msg, timeout=None):
        """Send a request and return the app's reply (raises ApiError if refused)."""
        req = next(self._ids)
        box = queue.Queue(maxsize=1)
        with self._replies_lock:
            self._replies[req] = box
        try:
            self.post(dict(msg, req=req))
            reply = box.get(timeout=timeout or self.timeout)
        except queue.Empty:
            raise TimeoutError(f'no reply to {msg.get("type")}') from None
        finally:
            with self._replies_lock:
                self._replies.pop(req, None)
        if reply.get('type') == 'error':
            raise ApiError(reply.get('message', 'error'))
        return reply

    def _read(self):
        f = self._sock.makefile('r', encoding='utf-8')
        try:
            for line in f:
                try:
                    msg = json.loads(line)
                except ValueError:
                    continue
                req = msg.get('req')
                with self._replies_lock:
                    box = self._replies.get(req) if req is not None else None
                if box is not None:
                    box.put(msg)
                elif 'event' in msg:
                    self._events.put(msg)
                elif msg.get('type') == 'error':
                    self._events.put(dict(msg, event='error'))
        except OSError:
            pass
        self.closed = True
        self._events.put({'event': 'disconnected'})

    def _dispatcher(self):
        while True:
            msg = self._events.get()
            with self.lock:
                self._dispatch(msg)
            if msg['event'] == 'disconnected':
                return

    # --- events --------------------------------------------------------------------------
    def on(self, event, fn=None):
        """Call `fn(msg)` for each event `event`: 'turn', 'press', 'release', 'click', 'hold'
        (every input, on any page: `msg['mine']` is true when it is yours - your page is shown
        or you grabbed it - `msg['page']` names the page), 'page_shown', 'alert_closed',
        'standby', 'error', 'disconnected', or '*' for all. Usable as @client.on('click')."""
        if fn is None:
            return lambda f: self.on(event, f) or f
        self._handlers.setdefault(event, []).append(fn)
        return fn

    def _dispatch(self, msg):
        for fn in self._handlers.get(msg['event'], []) + self._handlers.get('*', []):
            try:
                fn(msg)
            except Exception as e:  # a handler's error must not stop the others
                print(f'hsm01: handler of {msg["event"]}: {e!r}', flush=True)
        if msg['event'] == 'error' and 'error' not in self._handlers:
            print(f'hsm01: {msg.get("message")}', flush=True)

    def run(self, tick=None, fps=60):
        """Wait until the app goes away (or Ctrl+C), calling `tick(dt)` `fps` times per second
        if given (animations, clocks). Handlers run meanwhile, never at the same time."""
        period, last = 1.0 / fps, time.monotonic()
        try:
            while not self.closed:
                time.sleep(period if tick else 0.2)
                now = time.monotonic()
                if tick and not self.closed:
                    with self.lock:
                        tick(now - last)
                last = now
        except KeyboardInterrupt:
            pass
        finally:
            self.close()

    def close(self):
        self.closed = True
        try:
            self._sock.close()
        except OSError:
            pass

    # --- requests ------------------------------------------------------------------------
    def alert(self, title, text='', **options):
        """Popup on the device; returns its id ('alert_closed' tells when it goes).
        Options: icon (theme name or path), image (picture path), round, app, duration (s),
        led (1-4, blinks), priority='high', wake (bool)."""
        return self.call(dict(options, type='alert', title=title, text=text))['id']

    def pill(self, text, border='#3ddc84', dot=None, color='#ffffff', **options):
        """Short banner at the top of the screen: `text` (one line, bold; « ACTION ACTIVÉE : EN
        DIRECT ») in a dark rounded pill outlined in `border` (glowing), a `dot` at each end
        (default: the border's colour), the text in `color`. Options as `alert`: duration (3 s by
        default), led, wake, priority. Returns its id."""
        return self.alert('', text, style='pill', border=border, dot=dot or border,
                          color=color, **options)

    def page(self, page, widgets=(), name=None, background=None, show=False, grab_power=False,
             leds=None):
        """Create (or replace) your page `page` with these widgets (see `widgets.py`)."""
        msg = {'type': 'page', 'page': page, 'widgets': list(widgets), 'show': show,
               'grab_power': grab_power}
        if name:
            msg['name'] = name
        if background:
            msg['background'] = background
        if leds is not None:
            msg['leds'] = list(leds)
        self.call(msg)

    def widget(self, id, w, h, widgets=(), name=None, background=None):
        """Create (or replace) your widget `id`: content of w×h px (its own coordinates), that
        the user places on their pages (Widgets tab → « module » → this widget), shrunk or
        scrolled when the box is smaller; never shown alone. Update it with `update`/`set` like
        a page; a new size: call `widget` again."""
        msg = {'type': 'page', 'page': id, 'widgets': list(widgets), 'widget': True,
               'size': {'w': int(w), 'h': int(h)}}
        if name:
            msg['name'] = name
        if background:
            msg['background'] = background
        self.call(msg)

    def set(self, page, *widgets):
        """Add widgets, or replace those with the same id."""
        self.post({'type': 'set', 'page': page, 'widgets': list(widgets)})

    def update(self, page, changes):
        """Change some fields of some widgets: {widget id: {field: value}} (cheap: for
        animations, texts, colours...)."""
        if changes:
            self.post({'type': 'update', 'page': page, 'widgets': changes})

    def remove(self, page, *ids):
        self.post({'type': 'remove', 'page': page, 'widgets': list(ids)})

    def leds(self, page, values):
        """Button LEDs (percent ×4) while your page is shown."""
        self.post({'type': 'leds', 'page': page, 'values': list(values)})

    def show(self, page):
        """Show your page `page` on the device now."""
        self.call({'type': 'show', 'page': page})

    def close_page(self, page):
        self.post({'type': 'close_page', 'page': page})

    def action(self, action):
        """Run one of the app's actions, written as in config.toml: {'type': 'keys', 'keys':
        'ctrl+alt+t'}, {'type': 'command', 'cmd': '…'}, {'type': 'media', 'action': 'next'},
        {'type': 'page', 'index': 2}, {'type': 'brightness_up'}… (local programs only)."""
        self.call({'type': 'action', 'action': action})

    # --- commands ------------------------------------------------------------------------
    def command(self, id, name, kind='button', description=''):
        """Offer a command while this program runs: the window lists it (button actions
        « commande de module »; kind='knob': knob target « commande de module », LEDs) and the
        handler runs when it is used: handler(ev), ev['delta'] = knob steps (signed).
        Decorator: @client.command('mute', 'Couper le micro')."""
        def register(fn):
            self._commands[id] = ({'id': id, 'name': name, 'kind': kind,
                                   'description': description}, fn)
            self.call({'type': 'commands', 'commands': [c for c, _ in self._commands.values()]})
            return fn
        return register

    def commands(self, entries, replace=False):
        """Offer several commands in one request: [(id, name, handler), (id, name, handler,
        'knob'), (id, name, handler, kind, description)…]. replace=True: the commands offered
        before and not listed go away (lists that change: scenes, playlists…; their bindings in
        the window stay, shown « (arrêté) » until the command comes back)."""
        if replace:
            self._commands.clear()
        for id, name, fn, *more in entries:
            kind = more[0] if more else 'button'
            desc = more[1] if len(more) > 1 else ''
            self._commands[id] = ({'id': id, 'name': name, 'kind': kind, 'description': desc}, fn)
        self.call({'type': 'commands', 'commands': [c for c, _ in self._commands.values()]})

    def command_state(self, id, active=None, value=None, text=None):
        """What the app shows of command `id`: `active` (LED « état d'une commande de module »;
        False greys the widgets of a knob on it), `value` 0..1 (gauge, {volume}), `text` (the
        knob's {name}). Only the given fields change."""
        msg = {'type': 'command_state', 'command': id}
        if active is not None:
            msg['active'] = bool(active)
        if value is not None:
            msg['value'] = float(value)
        if text is not None:
            msg['text'] = str(text)
        self.post(msg)

    def _on_command(self, ev):
        entry = self._commands.get(ev.get('command'))
        if entry:
            entry[1](ev)

    def choices(self, **lists):
        """Lists of values for the `choice` fields of your settings form (module.toml
        `[[settings.field]]` with `source = "name"`): choices(serveurs=['A', 'B']) ; a name with
        characters Python does not allow: choices(**{'salons:A': [...]}). Merged with the lists
        given before."""
        self.post({'type': 'choices', 'choices': {k: [str(v) for v in vs]
                                                  for k, vs in lists.items()}})

    def grab(self, *inputs):
        """Take inputs on every page ('knob1'..'knob4' - turns and clicks -, 'button1'..
        'button4', 'power'): their events come with 'mine': true and the app's own bindings
        no longer act on them, until `ungrab` or the end of the connection."""
        self.call({'type': 'grab', 'inputs': list(inputs)})

    def ungrab(self, *inputs):
        """Give inputs back (all of them without arguments)."""
        msg = {'type': 'ungrab'}
        if inputs:
            msg['inputs'] = list(inputs)
        self.call(msg)
