#!/usr/bin/env python3
"""Show an alert on the Stream 100 and wait until it closes.

Usage: python3 -I app/clients/python/examples/alert.py "Titre" ["Texte"] [icône]
e.g.   make && python3 -I …/alert.py "Compilation finie" "0 erreur" dialog-ok
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..'))

from hsm01 import Client  # noqa: E402


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    title = sys.argv[1]
    text = sys.argv[2] if len(sys.argv) > 2 else ''
    icon = sys.argv[3] if len(sys.argv) > 3 else 'dialog-information'
    client = Client('alert-example')
    alert_id = client.alert(title, text, icon=icon, led=2, duration=8)

    @client.on('alert_closed')
    def closed(ev):
        if ev['id'] == alert_id:
            print('fermée :', ev['reason'])
            client.close()

    client.run()


if __name__ == '__main__':
    main()
