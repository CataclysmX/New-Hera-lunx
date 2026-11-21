"""Widgets of a page, as the app expects them (same fields as in config.toml).

Each function returns a dict with an `id` (to update or remove it later). Colours: '#rrggbb';
gradients: a list of colours (start -> end) wherever a colour is accepted (`color`, frames).
"""


def _with(d, frame):
    if frame is not None:
        d['frame'] = frame
    return d


def frame(border='#00b4a8', width=2.0, fill=None, radius=8.0, padding=4, sides=None):
    """Decorative frame of a widget; `sides` = (top, right, bottom, left) booleans."""
    f = {'border': border, 'width': width, 'radius': radius, 'padding': padding}
    if fill is not None:
        f['fill'] = fill
    if sides is not None:
        t, r, b, l = sides
        f['sides'] = {'top': t, 'right': r, 'bottom': b, 'left': l}
    return f


def text(id, x, y, w, h, text, size=18, color='#ffffff', align='center', bold=False,
         scroll=True, frame=None):
    """One line of text; too long: scrolls back and forth (`scroll`) or is cut with '…'."""
    return _with({'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'text', 'text': text,
                  'size': size, 'color': color, 'align': align, 'bold': bold,
                  'scroll': scroll}, frame)


def rect(id, x, y, w, h, color, radius=0.0, frame=None):
    return _with({'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'rect', 'color': color,
                  'radius': radius}, frame)


def icon(id, x, y, w, h, name, tint=None, frame=None):
    """Icon of the desktop theme by name ('audio-input-microphone', 'firefox'...) or an image
    file (absolute path), fitted in the box; `tint`: monochrome version in that colour."""
    d = {'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'theme_icon', 'name': name}
    if tint:
        d['tint'] = tint
    return _with(d, frame)


def image(id, x, y, w, h, path, radius=0.0, frame=None):
    """PNG file scaled to the box (use a new path to change it: images are cached by path, a
    missing file too); `radius` rounds its corners, half the box or more: a disc (avatar)."""
    d = {'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'image', 'path': path}
    if radius:
        d['radius'] = radius
    return _with(d, frame)


def tile(title, subtitle='', icon='', action=None):
    """A tile of a `deck`: `action` as in config.toml ({'type': 'media', 'action': 'next'}...)."""
    t = {'title': title, 'subtitle': subtitle, 'icon': icon}
    if action:
        t['action'] = action
    return t


def deck(id, x, y, w, h, knob, tiles, vertical=True, color='#00b4a8'):
    """« Stream deck » track run by the app: `tiles` (see `tile`) endlessly, the selected one in
    the middle; knob `knob` scrolls it, its click runs the tile's action (and sends you a 'deck'
    event). Column (`vertical`) or row."""
    return {'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'deck', 'knob': knob,
            'vertical': vertical, 'color': color, 'tile': list(tiles)}


def separator(id, x, y, w, h, color='#5a6666', thickness=2.0, vertical=False, style='solid',
              fade=False):
    """Line centred in the box; style 'solid', 'dashed' or 'dotted'; `fade`: faded ends."""
    return {'id': id, 'x': x, 'y': y, 'w': w, 'h': h, 'kind': 'separator', 'color': color,
            'thickness': thickness, 'vertical': vertical, 'style': style, 'fade': fade}
