"""Python client of the new hera-nux open API (Hercules Stream 100), standard library only.

- `Client`: connection, alerts, pages of widgets, knob / button events (`client.py`);
- `widgets`: the widgets of a page (`widgets.py`);
- `menu.GridMenu`: a Stream Deck-like menu built on the above (`menu.py`), to use or copy;
- `i18n.tr`: the module's texts in the app's language (`i18n.py`).

Protocol and examples: docs/app/api.md.
"""
from .client import ApiError, Client, socket_path
from . import i18n, menu, widgets

__all__ = ['ApiError', 'Client', 'socket_path', 'i18n', 'menu', 'widgets']
