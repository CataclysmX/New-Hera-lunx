"""Translations of a module. Its texts are written in English (the default language); a file
`i18n/<code>.toml` in the module's folder gives them in another language, in the same format as
the app's translation files:

    name = "Français"

    [text]
    "Not in a voice channel" = "Pas dans un salon vocal"
    "{n} members" = "{n} membres"        # same {placeholders}

`i18n/en.toml` holds the English texts themselves ("text of the code" = "text shown"): change a
value to change the English wording, copy the file to start a new language.
This library has its own file for its texts (hsm01/i18n/<code>.toml); the module's file comes on
top of it. The app gives its language to the modules it starts (`HSM01_LANG`, `en` by default) and starts
them again when it changes. It also translates the texts of the module's `module.toml` (name,
description, guide line by line, form labels, helps, units, buttons, sensitive data) with the
same file.

    from hsm01.i18n import tr
    tr('Not in a voice channel')
    tr('{n} members', n=3)
"""
import os
import re
import sys
import tomllib

DEFAULT = 'en'


def language():
    """The language the app asks for (`en` without the app)."""
    code = os.environ.get('HSM01_LANG') or DEFAULT
    return code if re.fullmatch(r'[A-Za-z0-9_-]{1,16}', code) else DEFAULT


def module_dir():
    """The module's folder: given by the app, else the folder of the program run."""
    given = os.environ.get('HSM01_MODULE_DIR')
    if given:
        return given
    main = getattr(sys.modules.get('__main__'), '__file__', None)
    return os.path.dirname(os.path.abspath(main)) if main else os.getcwd()


def read(path):
    """The [text] table of a translation file (English → translation), empty if none."""
    try:
        with open(path, 'rb') as f:
            return {str(k): str(v) for k, v in tomllib.load(f).get('text', {}).items()}
    except FileNotFoundError:
        return {}
    except (OSError, tomllib.TOMLDecodeError) as e:
        print(f'{path}: {e}', file=sys.stderr, flush=True)
        return {}


def load(folder=None, code=None):
    """The texts of this library, then of `<folder>/i18n/<code>.toml` (the module's). English
    too: `en.toml` gives the wording shown (the text of the code when it has none)."""
    code = code or language()
    texts = read(os.path.join(os.path.dirname(os.path.abspath(__file__)), 'i18n', f'{code}.toml'))
    texts.update(read(os.path.join(folder or module_dir(), 'i18n', f'{code}.toml')))
    return texts


_texts = None


def point(text):
    """`text` with its decimal points written the language's way (« 2,4 km » in French)."""
    sep = tr('.')
    return text if sep == '.' else text.replace('.', sep)


def tr(text, /, **values):
    """`text` (English) in the app's language, its `{name}` placeholders filled with `values`."""
    global _texts
    if _texts is None:
        _texts = load()
    out = _texts.get(text, text)
    for name, value in values.items():
        out = out.replace('{' + name + '}', str(value))
    return out
