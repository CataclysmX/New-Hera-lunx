# hsm01: the Python library of new hera-nux modules

[Version française](README_FR.md)

A **module** is a small program that adds pages, widgets, alerts and commands to new hera-nux,
the companion app of the Hercules Stream 100. The app starts it, gives it the device's knobs and
buttons, and draws what it asks for. This library (`hsm01`, Python 3.11 or later, **standard
library only**) is what the shipped modules use (Discord, Spotify, OBS, Motor Town, SimHub,
live scores…); anyone can use it to write new ones.

Contents: [Quick start](#quick-start) · [A module's folder](#a-modules-folder) ·
[What the app gives the program](#what-the-app-gives-the-program) · [The screen and its widgets](#the-screen-and-its-widgets) ·
[Client](#client) · [Events](#events) · [Threads](#threads) · [Sensitive data](#sensitive-data-hsm01secrets) ·
[Translations](#translations-hsm01i18n) · [Ready-made helpers](#ready-made-helpers) ·
[Good practice](#good-practice) · [Testing](#testing)

## Quick start
1. In the app's window, **Modules** tab: type a name, then **Create a module**. A folder opens
   with `module.toml`, `main.py` (a working example: a page, a counter on knob 1, an alert on
   button 1, two commands) and `i18n/en.toml`, `fr.toml`.
2. Switch the module on in its card. Hold knob 1 on the device: the **module menu** lists its page.
3. Change `main.py` and save: the app restarts the module by itself. What it prints goes to the
   **Log** of its card.

The smallest module:
```python
from hsm01 import Client
from hsm01 import widgets as W

client = Client('hello')                   # started by the app: the name of its card instead
client.page('main', [
    W.text('title', 0, 100, 480, 40, 'Hello', size=28, bold=True),
], name='Hello')


@client.on('click')
def click(ev):
    if ev['mine'] and ev['button'] == 'button1':   # button 1, while our page is shown
        client.pill('BUTTON 1')


client.run()                               # until the app closes
```
To try a program by hand while the app runs (outside the Modules tab):
`PYTHONPATH=<repository>/app/clients/python python3 main.py`.

## A module's folder
```
~/.local/share/hsm01/modules/          ($XDG_DATA_HOME/hsm01/modules)
└── my-module/                         ← its id (the folder's name)
    ├── module.toml                    ← its description
    ├── main.py                        ← its program
    ├── i18n/en.toml, fr.toml          ← its texts in English, in French
    ├── my-module.toml                 ← its settings, if it has some (any name)
    ├── .env                           ← its sensitive data, owner-only (written by the app)
    └── …                              ← anything else it needs (images, data)
```
`module.toml`:

| Key | Meaning |
|-----|---------|
| `name` | name shown in the Modules tab (and by the « show a module » action) |
| `description` | text under the name |
| `file` | Python program, relative to the folder (run by `python3`) |
| `command` | or any program: `["node", "index.js"]`, `["./my-module"]` (relative to the folder) |
| `args` | default arguments (editable in the Modules tab) |
| `[settings]` | `file` (its settings file, TOML, in the folder), `editor` (`formulaire`: a form of `field`s; `texte`: the TOML text; `deck`: the stream deck editor), `guide` (text at the top of its tab, one paragraph per line, addresses become links) — while the module is on, the window gives it **a tab of its own** |
| `[[settings.field]]` | one field of the form (below) |
| `[[secret]]` | one sensitive value: `name` (environment variable), `label`, `description`, `hidden` (shown as dots) — typed in its tab, kept in its `.env` |

Fields of the settings form (`[[settings.field]]`), saved in the settings file under `key`
(dotted for nested tables: `alerts.cpu.limit`), with `label` and `help` (on hover):

| `kind` | Extra keys | Shown as |
|--------|------------|----------|
| `number` | `min`, `max`, `step`, `unit`, `default`, `optional` (may be left empty) | a number |
| `text` | `default` | a line of text |
| `bool` | `default` | a switch |
| `choice` | `choices` (values saved) and `choice_labels` (texts shown, same order, translated), or `source` (a list the running module gives with `client.choices`; `{key}` = a sibling field's value: `source = "channels:{server}"`), `optional` | a drop-down list (typed by hand while the module is stopped) |
| `list` | `field` (the fields of each item), `add` (its button) | items added and removed (array of tables) |
| `section` | — | a heading |

Saving the form restarts the module: it reads its settings file once, at start
(`tomllib.load`). The shipped modules are full examples: `app/modules/*/module.toml`.

## What the app gives the program
| Environment variable | Content |
|----------------------|---------|
| `HSM01_API_SOCKET` | the API's socket (`Client` uses it) |
| `HSM01_MODULE` | the module's name in the Modules tab: `Client` says it in `hello`, so « show a module » and the module menu find its page |
| `HSM01_MODULE_DIR` | its folder |
| `HSM01_LANG` | the app's language (`en` by default, `fr`…) |
| the lines of its `.env` | its sensitive data, one variable each |
| `PYTHONPATH`, `HSM01_PYLIB` | this library: `import hsm01` works as it is |
| `PYTHONUNBUFFERED=1` | `print` reaches the Log at once |

Life of a module: started when its switch is on; started again if it stops (1 s, 2 s, 4 s…
30 s at most), when its code changes, when its settings or sensitive data are saved, and when
the app's language changes; stopped when it is switched off or removed, or when the app quits.
**It must stop when the connection closes**: `client.run()` returns then.
Its card also has **« no standby while it is shown »**: the screen does not turn off while its
page (or one of its widgets) is on screen.

## The screen and its widgets
480 × 272 pixels, origin at the top left. Colours `'#rrggbb'`; a **gradient** is a list of
colours (start → end) wherever a colour is accepted. `from hsm01 import widgets as W`; each
function returns a dict with an `id` (to change or remove it later):

| Function | Draws |
|----------|-------|
| `W.text(id, x, y, w, h, text, size=18, color, align='center', bold=False, scroll=True)` | one line of text; too long: scrolls back and forth (`scroll`), or is cut with « … » |
| `W.rect(id, x, y, w, h, color, radius=0)` | a rectangle, rounded corners |
| `W.separator(id, x, y, w, h, color, thickness=2, vertical=False, style='solid', fade=False)` | a line: `solid`, `dashed`, `dotted`; `fade`: faded ends |
| `W.icon(id, x, y, w, h, name, tint=None)` | an icon of the desktop theme by name (`'audio-input-microphone'`, `'firefox'`) or an image file; `tint`: monochrome in that colour |
| `W.image(id, x, y, w, h, path, radius=0)` | a PNG file scaled to the box; `radius` rounds its corners (half the box: a disc, for avatars). Images are cached **by path**: give a new path to change the picture |
| `W.deck(id, x, y, w, h, knob, tiles, vertical=True, color)` | a stream deck track run by the app: `tiles` (`W.tile(title, subtitle, icon, action)`) scroll endlessly with knob `knob`, its click runs the tile's action and sends you a `deck` event |

Every widget but `deck` accepts `frame=W.frame(border, width=2, fill=None, radius=8, padding=4,
sides=(top, right, bottom, left))`: a border, an optional background, the content inset.

## Client
`client = Client(name, path=None, timeout=5.0)` connects (raises `ConnectionError` if the app
is not running) and says hello; `client.welcome` holds the app's answer (`api`, `app`,
`screen` {width, height}, `firmware`). Requests refused by the app raise `ApiError`; `call`
waits `timeout` seconds at most (`TimeoutError`).

**Pages and widgets**

| Method | Does |
|--------|------|
| `page(page, widgets, name=None, background=None, show=False, grab_power=False, leds=None)` | creates or replaces your page `page` (shown in the module menu as `name`); `show`: on the device now; `leds`: 4 × % while it is shown; `grab_power`: power goes to you instead of « back » |
| `widget(id, w, h, widgets, name=None, background=None)` | a **widget of the module**: content of `w` × `h` px in its own coordinates, that users place on their own pages (Widgets tab → « module » → your module → show: this widget), shrunk or scrolled to fit their box; call it again for a new size |
| `update(page, {id: {field: value}})` | changes some fields of some widgets (text, colour, position…): cheap, for live values and animations |
| `set(page, *widgets)` | adds widgets, or replaces those with the same id |
| `remove(page, *ids)` | removes widgets |
| `leds(page, values)` | the 4 button LEDs (%) while the page is shown |
| `show(page)` | shows your page on the device |
| `close_page(page)` | removes the page |

**Alerts**

| Method | Does |
|--------|------|
| `alert(title, text='', icon=, image=, round=, app=, duration=, led=, priority='high', wake=)` | a card at the top of the screen; returns its id (`alert_closed` tells when it goes). `icon`: theme name or path; `image`: a picture (`round`: a disc); `app`: one alert per application in the queue; `led`: 1-4 blinks; `wake`: turns the screen on |
| `pill(text, border='#3ddc84', dot=None, color='#ffffff', duration=3, …)` | a short band at the bottom of the screen (one line, bold), outlined in `border` |

**Commands**: offered to the user while the module runs, usable on **every** page (a button's
action « module command », a knob's target « module command », a LED « state of a module
command »).

| Method | Does |
|--------|------|
| `@client.command(id, name, kind='button', description='')` | decorator: `fn(ev)` runs when it is used; `kind='knob'`: `ev['delta']` = signed steps |
| `commands([(id, name, fn), (id, name, fn, 'knob', description)…], replace=False)` | several at once; `replace=True`: lists that change (scenes, playlists) |
| `command_state(id, active=None, value=None, text=None)` | what the app shows of it: `active` (LED on; `False` greys a knob's widgets), `value` 0..1 (gauges), `text` (a knob's `{name}`) |

**Others**

| Method | Does |
|--------|------|
| `choices(**lists)` | the lists of your form's `choice` fields with `source = "name"`: `choices(servers=['A', 'B'])`; names Python does not accept: `choices(**{'channels:A': […]})` |
| `grab(*inputs)` / `ungrab(*inputs)` | take `'knob1'`…`'knob4'`, `'button1'`…`'button4'`, `'power'` on every page (`mine` true, the app's bindings no longer act) / give them back |
| `action(action)` | runs an action of the app, as in `config.toml`: `{'type': 'keys', 'keys': 'ctrl+alt+t'}`, `{'type': 'media', 'action': 'next'}`, `{'type': 'page', 'index': 2}`… (local programs only) |
| `call({'type': 'media'})` | the media player the app follows (`player`, `status`, `title`, `artist`, `album`, `art`, `length`, `position`, `volume`); `{'type': 'media', 'action': 'play_pause'}` acts on it |
| `run(tick=None, fps=60)` | waits until the app goes away (or Ctrl+C), calling `tick(dt)` `fps` times a second if given (clocks, animations) |
| `post(msg)` / `call(msg)` | any request of the protocol, without / with waiting for the reply |

## Events
`@client.on(event)` (or `client.on(event, fn)`) calls `fn(ev)` for each event; `'*'` for all.
**Every input reaches every program**, whatever the page: `ev['mine']` is true for the one that
takes it (its page is shown, or it grabbed the input), and only then do the app's own bindings
leave it alone. `ev['page']`: your page's id, or the name of a page of the user's config.

| Event | Fields | When |
|-------|--------|------|
| `turn` | `page`, `mine`, `knob` (1-4), `delta` (signed steps, ~21 a turn) | a knob turns |
| `press`, `release`, `click`, `hold` | `page`, `mine`, `button`: `knob1`…`knob4` (knob clicks), `button1`…`button4`, `power` | pressed, released, short click, held 0.5 s |
| `page_shown` | `page`, `shown` | your page appears / goes |
| `deck` | `page`, `widget`, `tile`, `title` | a tile of one of your `deck` widgets was run |
| `command` | `command`, `delta` | one of your commands was used (handled by `@client.command`) |
| `alert_closed` | `id`, `reason` (`timeout`, `dismissed`, `standby`, `disabled`, `replaced`) | one of your alerts went |
| `standby` | `asleep` | the screen goes to standby / wakes up |
| `media` | as `call({'type': 'media'})` | the followed player changes, and every second while playing |
| `error` | `message` | a request sent with `post` was refused |
| `disconnected` | — | the app went away |

On your page, the app keeps two inputs: **holding knob 1** opens the module menu, **power**
goes back (unless `grab_power` or `grab('power')`). A click while an alert is shown closes it
and is not sent.

## Threads
A reader thread receives the app's messages; **handlers run one at a time** in a dispatcher
thread, holding `client.lock`; `run`'s `tick` holds it too, so handlers and `tick` never run
together. Client methods can be called from any thread. If your own threads (a network loop,
a socket) change state the handlers read, guard it with a lock of your own, and register
handlers inside `with client.lock:` when events may already come in.

## Sensitive data: hsm01.secrets
Tokens, passwords, keys: declared by `[[secret]]` in `module.toml`, typed in the module's tab,
kept in its `.env` (owner-only), given as environment variables. Never in `config.toml`, the
code or the logs.

| Function | Does |
|----------|------|
| `secrets.get(name, default=None)` | a value: the environment, else the `.env` (saved meanwhile) |
| `secrets.save(name, value)` | stores a value obtained while running (an OAuth token…) for the next starts, without restarting; empty: removes it |
| `secrets.read()`, `secrets.path()` | all the values; the `.env`'s path |

## Translations: hsm01.i18n
Write the texts in English and give them to `tr`; `i18n/<code>.toml` in the module's folder
gives them in each language, chosen in the app (Device tab), given as `HSM01_LANG`:
```python
from hsm01.i18n import tr, point
tr('No live match')                # « Aucun match en cours » in French
tr('{n} members', n=3)             # same {placeholders} in every language
point('2.4 km')                    # decimals the language's way: « 2,4 km » in French
```
```toml
# i18n/fr.toml                       (en.toml: "text of the code" = "English shown")
name = "Français"

[text]
"No live match" = "Aucun match en cours"
"{n} members" = "{n} membres"
```
The app translates the module's `module.toml` with the same file: `name`, `description`, the
`guide` line by line, the form's `label`, `help`, `unit`, `add`, `choice_labels`, and the
`[[secret]]` `label`, `description`. Keep the values saved in settings (`choices`, keys) in one
language: they are identifiers. `en.toml` holds the English itself (change a value to reword
it); copy it to start another language. For the shipped modules,
`python3 -I tools/i18n_check.py fr` checks that nothing is missing, and
`python3 -I tools/i18n_check.py en --fill` writes the English files again.

## Ready-made helpers
| Module | For |
|--------|-----|
| `hsm01.menu` | `GridMenu(client, tiles, page='menu', title='Menu', columns=4, rows=2)`: a Stream Deck-like grid of `Tile(title, icon, action=fn(menu), hold=fn, folder=[…], color)`, one column per button, rows scrolled by the knobs, folders; `title` and `icon` may be functions (clocks, states). Example: `app/modules/deck/main.py` |
| `hsm01.dashboard` | `Dashboard(client, name, page='tableau', hints, options)`: a car dashboard (speed, revs, gear, pedals, fuel, lamps, race and vehicle screens, shift lights on the LEDs, pills); give it samples with `feed(sample)` (keys in its docstring), `message(status, lines)` while nothing comes, `client.run(tick=dash.tick, fps=20)` |
| `hsm01.udp` | game telemetry by UDP: `Receiver(host, port, relays)` (`recv(timeout)` → `(data, addr)` or None, each datagram also relayed to `relays`), `settings(table, default_port)` → listening IP, port, relays and the problems in words, `ip`, `port`, `this_pc()` |
| `hsm01.forza` | the « Forza Data Out » UDP format many games copy: `decode(data)` → fields, `sample(raw, game)` → a dashboard sample |

## Good practice
- **Send only what changed**: keep what you sent and give `update` the fields that differ
  (the shipped modules do it for every page and widget). The app only sends the device the
  pixels that change, but each request still costs.
- Build the page once with all its widgets (empty texts where needed), then `update` it.
- To change an image, write it to a new path (the app caches images by path).
- Start fast, then do slow work (network, devices) in a thread; show a « connecting… » text
  meanwhile.
- `print` short, useful lines: they are the module's Log. Never print a token.
- Quit when the connection closes (`client.run()` returns): the app starts the module again
  when it comes back.
- Ask web services sparingly (the live scores module reads its feed once a minute at most),
  and say who you are in a `User-Agent`.

## Testing
- `PYTHONPATH=<repository>/app/clients/python python3 main.py` while the app runs: errors in
  the terminal. `hsm01ctl events` shows every event the app sends.
- The shipped modules are tested against fake servers (`tools/fake_*.py`: Discord, Spotify,
  OBS, games, live scores) by the app's end-to-end tests (`app/hsm01-app/src/api.rs`,
  `cargo test --release -- --ignored`): a model to copy for yours.

Other languages than Python: the program speaks JSON lines on the Unix socket; the protocol is
described in [docs/app/api.md](../../../docs/app/api.md) (in French). Examples:
[examples/alert.py](examples/alert.py), [examples/tool_template.py](examples/tool_template.py),
and the shipped modules in [app/modules/](../../modules/).
