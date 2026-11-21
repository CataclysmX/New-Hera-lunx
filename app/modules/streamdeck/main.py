#!/usr/bin/env python3
"""Stream deck for new hera-nux: 4 tracks of tiles, one per knob, endless (turn: choose; click:
run the tile). The tracks and their tiles are in deck.toml (this folder); the app draws, scrolls
and runs them (`deck` widgets), this program only lays the page out.

Layout: `layout` of deck.toml (the « Stream deck » tab of the window edits it), else --layout:
colonnes (default: track k above knob k) | lignes (one row per knob). Other options: --file
DECK.toml, --show (page shown at start). Power: back to the page before.
Started by the app (tab « Modules »); by hand: PYTHONPATH=app/clients/python python3 main.py
"""
import argparse
import os
import tomllib

from hsm01 import Client
from hsm01 import widgets as W
from hsm01.i18n import tr

PAGE = 'deck'
BG, GREY = '#05070a', '#8a9696'
LAYOUTS = {'colonnes': 'columns', 'columns': 'columns', 'lignes': 'rows', 'rows': 'rows'}


def load(path):
    """deck.toml: its layout (or None) and its tracks (4 at most): [(name, colour, [tile...])]."""
    with open(path, 'rb') as f:
        data = tomllib.load(f)
    tracks = []
    for t in data.get('track', [])[:4]:
        tiles = [W.tile(c.get('title', ''), c.get('subtitle', ''), c.get('icon', ''),
                        c.get('action')) for c in t.get('tile', [])]
        tracks.append((t.get('name', ''), t.get('color', '#00b4a8'), tiles))
    return data.get('layout'), tracks


def columns(tracks):
    """Track k in the column above knob k: its name on top, underlined in its colour."""
    widgets = []
    for k, (name, color, tiles) in enumerate(tracks):
        x = k * 120
        widgets += [
            W.text(f'name{k}', x, 2, 120, 20, name, size=14, bold=True, color=color),
            W.separator(f'line{k}', x + 12, 21, 96, 4, color=color, fade=True),
            W.deck(f'deck{k}', x + 4, 28, 112, 244, k + 1, tiles, vertical=True, color=color),
        ]
    return widgets


def rows(tracks):
    """One row per knob: a label (knob number, name) on the left, the tiles on the right."""
    widgets = []
    for k, (name, color, tiles) in enumerate(tracks):
        y = k * 68
        widgets += [
            W.rect(f'bar{k}', 2, y + 8, 3, 52, color, radius=1.5),
            W.text(f'knob{k}', 10, y + 14, 70, 16, tr('Knob {n}', n=k + 1), size=11, color=GREY,
                   align='left'),
            W.text(f'name{k}', 10, y + 32, 70, 20, name, size=14, bold=True, color=color,
                   align='left'),
            W.deck(f'deck{k}', 82, y + 2, 396, 64, k + 1, tiles, vertical=False, color=color),
        ]
    return widgets


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--layout', choices=sorted(LAYOUTS),
                    help='if deck.toml says nothing: colonnes (default, track k above knob k) or lignes (one row per knob)')
    ap.add_argument('--file', default=os.path.join(here, 'deck.toml'), help='tracks and tiles')
    ap.add_argument('--show', action='store_true', help='show the page at start')
    args = ap.parse_args()

    in_file, tracks = load(args.file)
    layout = LAYOUTS.get(in_file or args.layout or 'colonnes', 'columns')
    client = Client('stream-deck')
    widgets = columns(tracks) if layout == 'columns' else rows(tracks)
    # started by the app: no jump at start (a button « afficher un module » shows it)
    client.page(PAGE, widgets, name='Stream deck', background=BG,
                show=args.show or not os.environ.get('HSM01_MODULE'))
    print(tr('{tracks} tracks, {tiles} tiles, layout {layout}', tracks=len(tracks), tiles=sum(len(t[2]) for t in tracks), layout=layout))

    @client.on('deck')
    def validated(ev):
        print(tr('tile "{title}" ({widget})', title=ev.get('title'), widget=ev.get('widget')))

    client.run()


if __name__ == '__main__':
    main()
