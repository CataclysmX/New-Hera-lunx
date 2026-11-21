#!/usr/bin/env python3
"""Menu driven by the Rust test `api::tests::python_menu_end_to_end` (HSM01_API_SOCKET set by
it): 12 tiles T0..T11 on 3 rows; tile i's action shows the alert "tile i".
Usage: python3 -I e2e_menu.py [seconds]
"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))

from hsm01 import Client  # noqa: E402
from hsm01.menu import GridMenu, Tile  # noqa: E402


def main():
    seconds = float(sys.argv[1]) if len(sys.argv) > 1 else 4.0
    client = Client('e2e')

    def action(i):
        return lambda menu: menu.c.alert(f'tile {i}')

    tiles = [Tile(f'T{i}', 'dialog-information', action=action(i)) for i in range(12)]
    menu = GridMenu(client, tiles, page='deck', title='E2E')
    end = time.monotonic() + seconds

    def tick(dt):
        menu.tick(dt)
        if time.monotonic() > end:
            client.close()

    client.run(tick=tick)


if __name__ == '__main__':
    main()
