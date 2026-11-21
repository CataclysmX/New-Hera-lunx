"""Stream Deck-like menu: a grid of tiles, one column per button (and knob), rows that scroll
(endlessly when there are more rows than the screen shows), folders.

Navigation on the Stream 100: turning a knob scrolls the rows (animated; the app sends a moving
area as a COPY + the part that appears), the top row is the active one (highlighted, its buttons'
LEDs on); button k - or a click of knob k - triggers tile k of that row, holding it triggers its
`hold` action. A folder opens a sub-grid whose first tile goes back.

Only what changed is sent (fields of the widgets that moved or changed). Change anything here:
it is a regular program using the public API, like yours.
"""
import math

from . import widgets as W
from .i18n import tr

BG = '#0b1010'
TILE = '#162020'
TILE_PRESSED = '#2c4242'
EDGE = '#283434'
ACCENT = '#00b4a8'
TEXT = '#e8eeee'
GREY = '#8a9696'


class Tile:
    """One tile. `title` / `icon` may be functions (re-read twice a second: clocks, states);
    `action(menu)` on a click, `hold(menu)` on a long press; `folder`: list of tiles, or a
    function returning one when it opens (dynamic content); `color`: background."""

    def __init__(self, title, icon=None, action=None, hold=None, folder=None, color=None):
        self.title, self.icon, self.action, self.hold = title, icon, action, hold
        self.folder, self.color = folder, color

    def value(self, what):
        v = getattr(self, what)
        return v() if callable(v) else v


class GridMenu:
    def __init__(self, client, tiles, page='menu', title='Menu', columns=4, rows=2,
                 steps_per_row=3, infinite=True, accent=ACCENT, show=True):
        self.c, self.page, self.title = client, page, title
        self.columns, self.rows = columns, rows
        self.steps_per_row, self.infinite, self.accent = steps_per_row, infinite, accent
        self.stack = []  # (tiles, top, title) of the parent folders
        self.tiles, self.top, self.path = list(tiles), 0, [title]
        self.offset = 0.0  # px, animated back to 0 after a scroll
        self.acc = 0  # knob steps not turned into rows yet
        self.pressed = None  # column held down
        self.sent = {}  # widget id -> fields last sent
        self.clock = 0.0
        # geometry: header, then rows of tiles
        gap, head = 8, 34
        self.tw = (480 - gap * (columns + 1)) // columns
        self.th = (272 - head - gap * rows) // rows
        self.pitch, self.x0, self.y0 = self.th + gap, gap, head + gap // 2
        self.slots = range(-1, rows + 1)  # one row above and below for the animation
        client.page(page, self._initial(), name=title, background=BG, show=show)
        with client.lock:  # events may already come in
            for ev in ('turn', 'press', 'release', 'click', 'hold'):
                client.on(ev, getattr(self, '_on_' + ev))
            self.refresh(force=True)

    # --- model ---------------------------------------------------------------------------
    @property
    def n_rows(self):
        return max(1, math.ceil(len(self.tiles) / self.columns))

    @property
    def scrolls(self):
        return self.n_rows > self.rows

    def row_tiles(self, row):
        """Tiles of row `row` (wrapped when scrolling endlessly), None outside the grid."""
        if self.scrolls and self.infinite:
            row %= self.n_rows
        elif not 0 <= row < self.n_rows:
            return [None] * self.columns
        cells = self.tiles[row * self.columns:(row + 1) * self.columns]
        return cells + [None] * (self.columns - len(cells))

    def scroll(self, rows):
        if not self.scrolls:
            return
        new = self.top + rows
        if not self.infinite:
            new = max(0, min(self.n_rows - self.rows, new))
        self.offset += (new - self.top) * self.pitch  # content starts where it was, slides
        self.top = new

    def open(self, tile):
        content = tile.folder() if callable(tile.folder) else tile.folder
        self.stack.append((self.tiles, self.top, self.path))
        back = Tile(tr('Back'), 'go-previous', action=lambda m: m.back())
        self.tiles, self.top, self.offset = [back] + list(content), 0, 0.0
        self.path = self.path + [tile.value('title')]
        self.refresh(force=True)

    def back(self):
        if self.stack:
            self.tiles, self.top, self.path = self.stack.pop()
            self.offset = 0.0
            self.refresh(force=True)

    def trigger(self, column, long=False):
        tile = self.row_tiles(self.top)[column]
        if tile is None:
            return
        fn = tile.hold if long else tile.action
        if fn:
            fn(self)
        elif tile.folder is not None and not long:
            self.open(tile)

    # --- events --------------------------------------------------------------------------
    def _mine(self, ev):
        """Inputs of this menu's page (every client sees every input: `mine` says it is ours)."""
        return ev.get('mine', True) and ev.get('page') == self.page

    @staticmethod
    def _column(ev):
        b = ev.get('button', '')
        for prefix in ('button', 'knob'):
            if b.startswith(prefix) and b[len(prefix):].isdigit():
                return int(b[len(prefix):]) - 1
        return None

    def _on_turn(self, ev):
        if self._mine(ev):
            self.acc += ev['delta']
            rows = int(self.acc / self.steps_per_row)
            if rows:
                self.acc -= rows * self.steps_per_row
                self.scroll(rows)

    def _on_press(self, ev):
        if self._mine(ev):
            self.pressed = self._column(ev)
            self.refresh()

    def _on_release(self, ev):
        if self._mine(ev):
            self.pressed = None
            self.refresh()

    def _on_click(self, ev):
        col = self._column(ev)
        if self._mine(ev) and col is not None and col < self.columns:
            self.trigger(col)

    def _on_hold(self, ev):
        col = self._column(ev)
        if self._mine(ev) and col is not None and col < self.columns:
            self.trigger(col, long=True)

    def tick(self, dt):
        """Call `fps` times a second (Client.run(tick=menu.tick)): animation, live titles."""
        moving = abs(self.offset) >= 0.5
        if moving:
            self.offset *= math.exp(-dt * 16)  # eased slide back to the rest position
            if abs(self.offset) < 0.5:
                self.offset = 0.0
        self.clock += dt
        live = self.clock >= 0.5
        if live:
            self.clock = 0.0
        if moving or live:
            self.refresh()

    # --- drawing -------------------------------------------------------------------------
    def _initial(self):
        out = []
        for r in self.slots:
            for c in range(self.columns):
                out += [W.rect(f'bg{r}_{c}', -999, 0, self.tw, self.th, TILE, radius=10),
                        W.icon(f'ic{r}_{c}', -999, 0, 56, 56, 'image-missing'),
                        W.text(f'lb{r}_{c}', -999, 0, self.tw - 8, 26, '', size=15,
                               color=TEXT)]
        for c in range(self.columns):
            out.append(W.text(f'key{c}', -999, 0, 18, 18, str(c + 1), size=12, bold=True,
                              color=BG))
        # header last: tiles sliding up pass under it
        out += [W.rect('head', 0, 0, 480, 32, BG),
                W.separator('head_line', 0, 29, 480, 6, color=EDGE, fade=True),
                W.text('title', 10, 2, 330, 28, '', size=17, bold=True, align='left',
                       color=TEXT),
                W.text('pos', 340, 2, 130, 28, '', size=14, align='right', color=GREY)]
        return out

    def refresh(self, force=False):
        """Send the changed fields of every widget."""
        changes = {}

        def put(wid, **fields):
            last = self.sent.setdefault(wid, {})
            diff = {k: v for k, v in fields.items() if force or last.get(k) != v}
            if diff:
                last.update(diff)
                changes[wid] = diff

        off = round(self.offset)
        for r in self.slots:
            row = self.top + r
            cells = self.row_tiles(row)
            for c, tile in enumerate(cells):
                x = self.x0 + c * (self.tw + 8)
                y = self.y0 + r * self.pitch + off
                if tile is None:
                    for p in ('bg', 'ic', 'lb'):
                        put(f'{p}{r}_{c}', x=-999)
                    continue
                active = r == 0
                pressed = active and self.pressed == c
                color = TILE_PRESSED if pressed else (tile.color or TILE)
                edge = {'border': self.accent if active else EDGE,
                        'width': 2.0 if active else 1.0, 'radius': 10.0, 'padding': 0}
                put(f'bg{r}_{c}', x=x, y=y, color=color, frame=edge)
                put(f'ic{r}_{c}', x=x + (self.tw - 56) // 2, y=y + 12,
                    name=tile.value('icon') or 'image-missing')
                put(f'lb{r}_{c}', x=x + 4, y=y + self.th - 34, text=str(tile.value('title')))
        # button numbers on the active row, LEDs of the buttons that do something
        active = self.row_tiles(self.top)
        for c, tile in enumerate(active):
            x = self.x0 + c * (self.tw + 8) + 6
            put(f'key{c}', x=x if tile else -999, y=self.y0 + 6 + off,
                frame={'border': self.accent, 'width': 0.0, 'radius': 9.0, 'padding': 0,
                       'fill': self.accent})
        leds = [100 if t else 0 for t in active[:4]] + [0] * (4 - min(4, len(active)))
        if self.sent.get('_leds') != leds:
            self.sent['_leds'] = leds
            self.c.leds(self.page, leds)
        put('title', text=' › '.join(self.path))
        rowno = self.top % self.n_rows + 1
        put('pos', text=f'{rowno} / {self.n_rows}' if self.scrolls else '')
        self.c.update(self.page, changes)
