#!/usr/bin/env python3
"""My module for new hera-nux: a page « My module » on the Stream 100.

- knob 1: changes the counter;
- button 1: sends an alert;
- power: back to the previous page (done by the app).
It also adds two commands to the app, usable on every page while it runs (Controls tab):
« Reset » (a button's action) and « Counter » (a knob's target).

Texts: written in English, translated by i18n/fr.toml (tr(), from hsm01.i18n) into the language
chosen in the app (Device tab); copy fr.toml to add another language.

Started by the app (« Modules » tab): `hsm01` can be imported as it is and what is printed goes
to the module's « Log ». To try it by hand while the app runs:
    PYTHONPATH=<app folder>/clients/python python3 main.py
Docs: docs/app/modules.md (making a module) and docs/app/api.md (all the API allows).
"""
from hsm01 import Client
from hsm01 import widgets as W
from hsm01.i18n import tr

PAGE = 'main'  # id of the page, this module's own
client = Client('my-module')  # started by the app, it takes the name given in the Modules tab
count = 0

# The page: same widgets as in the app's config (text, rectangle, icon, image...).
client.page(PAGE, [
    W.text('title', 0, 10, 480, 30, tr('My module'), size=20, bold=True, color='#9ad8d0'),
    W.text('count', 0, 80, 480, 90, '0', size=72, bold=True),
    W.text('help', 0, 222, 480, 30, tr('knob 1: count · button 1: alert'), size=15,
           color='#8a9696'),
], name=tr('My module'))


def set_count(value):
    """The counter, on our page and for the widgets of a knob set to « Counter »."""
    global count
    count = value
    client.update(PAGE, {'count': {'text': str(count)}})
    client.command_state('counter', text=tr('Counter {n}', n=count), value=min(abs(count), 100) / 100)


@client.on('turn')
def turn(ev):
    """A knob turns. `ev['mine']`: the input is for us (our page is shown)."""
    if ev['mine'] and ev['page'] == PAGE and ev['knob'] == 1:
        set_count(count + ev['delta'])


# Commands: offered in the app's Controls tab while the module runs.
@client.command('zero', tr('Reset'), description=tr('the counter starts again from 0'))
def zero(ev):
    set_count(0)


@client.command('counter', tr('Counter'), kind='knob')
def knob(ev):
    """A knob set to « Counter » turns (on any page)."""
    set_count(count + ev['delta'])


set_count(0)


@client.on('click')
def click(ev):
    """A click of a button or a knob."""
    if ev['mine'] and ev['page'] == PAGE and ev['button'] == 'button1':
        client.alert(tr('My module'), tr('The counter is {n}', n=count), icon='dialog-information', led=1)
        print('alert sent, counter', count)


client.run()  # until the app closes (or Ctrl+C)
