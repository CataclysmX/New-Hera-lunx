#!/usr/bin/env python3
"""Stream Deck-like menu on the Stream 100 (new hera-nux must be running).

Turn a knob: scroll the rows (endless). Buttons 1-4 (or knob clicks): tiles of the highlighted
row. Power: next page (back to the app's pages). Ctrl+C: quit (the page goes away).

Started by the app (tab « Modules »); by hand: PYTHONPATH=app/clients/python python3 main.py
"""
import os
import shutil
import subprocess
import time


from hsm01 import Client
from hsm01.menu import GridMenu, Tile
from hsm01.i18n import tr


def launch(*cmd):
    """Action starting a program (an alert says so if it is not installed)."""
    def run(menu):
        if shutil.which(cmd[0]) is None:
            menu.c.alert(tr('Not found'), tr('{program} is not installed', program=cmd[0]), icon='dialog-warning')
            return
        subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL, start_new_session=True)
    return run


def media(action):
    """Action on the player that plays (the app follows the players: no playerctl needed)."""
    return lambda menu: menu.c.post({'type': 'media', 'action': action})


def main():
    client = Client(os.environ.get('HSM01_MODULE') or 'deck-example')
    tiles = [
        Tile(tr('Browser'), 'firefox', action=launch('xdg-open', 'https://www.kde.org')),
        Tile('Terminal', 'utilities-terminal', action=launch('konsole')),
        Tile(tr('Files'), 'system-file-manager', action=launch('dolphin')),
        Tile(tr('Screenshot'), 'spectacle', action=launch('spectacle')),
        Tile(tr('Media'), 'multimedia-player', folder=[
            Tile(tr('Play'), 'media-playback-start', action=media('play_pause')),
            Tile(tr('Next'), 'media-skip-forward', action=media('next')),
            Tile(tr('Previous'), 'media-skip-backward', action=media('previous')),
        ]),
        Tile(lambda: time.strftime('%H:%M:%S'), 'preferences-system-time',
             action=lambda m: m.c.alert(tr('Time'), time.strftime('%A %d %B, %H:%M'),
                                        icon='preferences-system-time')),
        Tile(tr('Alert'), 'dialog-information',
             action=lambda m: m.c.alert(tr('Hello'), tr('Sent by the example menu'),
                                        icon='dialog-information', led=1)),
        Tile(tr('Red'), 'color-management', color=['#5a1010', '#2a0808'],
             action=lambda m: m.c.alert(tr('Red'), tr('A tile with a gradient'))),
        Tile(tr('Settings'), 'preferences-system', action=launch('systemsettings')),
        Tile(tr('Calculator'), 'accessories-calculator', action=launch('kcalc')),
        Tile(tr('Editor'), 'accessories-text-editor', action=launch('kate')),
    ]
    # started by the app: no jump to the menu at start (a button or a `tool` widget shows it)
    menu = GridMenu(client, tiles, page='deck', title='Menu',
                    show=not os.environ.get('HSM01_MODULE'))
    client.on('error', lambda e: print(tr('error:'), e.get('message')))
    client.run(tick=menu.tick)


if __name__ == '__main__':
    main()
