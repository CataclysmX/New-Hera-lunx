#!/usr/bin/env python3
"""Check the translation files of the shipped modules and of the Python library (hsm01).

Every text given to tr('…') in a module's code, and every text of its module.toml that the app
translates (name, description, guide line by line, form labels, helps, units, buttons, choice
labels, sensitive data), must have an entry in the module's i18n/<code>.toml (or in the
library's, for the library and the texts both use), with the same {placeholders}; entries no
longer used are reported too.

English has files too (`en.toml`: "text of the code" = "text shown", a value changed rewords
the English): `--fill` writes them, every text used, the values already there kept, the entries
no longer used dropped.

Usage: python3 -I tools/i18n_check.py [code]      (default: fr; exit code 1 on a problem)
       python3 -I tools/i18n_check.py en --fill  (write the English files, then check them)
"""
import ast
import json
import os
import re
import sys
import tomllib

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'app')
LIB = os.path.join(ROOT, 'clients', 'python', 'hsm01')
MODULES = os.path.join(ROOT, 'modules')
EXAMPLES = os.path.join(ROOT, 'clients', 'python', 'examples')
TOOLS_RS = os.path.join(ROOT, 'hsm01-app', 'src', 'tools.rs')


def tr_texts(path):
    """The literal first arguments of tr(…) in a Python file."""
    out = []
    for node in ast.walk(ast.parse(open(path, encoding='utf-8').read())):
        if isinstance(node, ast.Call) and getattr(node.func, 'id', None) == 'tr' and node.args:
            a = node.args[0]
            if isinstance(a, ast.Constant) and isinstance(a.value, str):
                out.append(a.value)
    return out


def toml_texts(path):
    """The texts of a module.toml the app translates (tools::translate)."""
    d = tomllib.load(open(path, 'rb'))
    out = [d.get('name', ''), d.get('description', '')]
    s = d.get('settings', {})
    out += s.get('guide', '').split('\n')

    def fields(fs):
        for f in fs:
            out.extend([f.get('label', ''), f.get('help', ''), f.get('unit', ''), f.get('add', '')])
            out.extend(f.get('choice_labels', []))
            fields(f.get('field', []))
    fields(s.get('field', []))
    for sec in d.get('secret', []):
        out += [sec.get('label', ''), sec.get('description', '')]
    return [t for t in out if t.strip()]


def catalog(folder, code):
    path = os.path.join(folder, 'i18n', f'{code}.toml')
    try:
        return tomllib.load(open(path, 'rb')).get('text', {})
    except FileNotFoundError:
        return {}


EN_HEAD = ('# English texts of {what} (hsm01/i18n.py): "text of the code" = "text shown", same\n'
           '# {{placeholders}}. Change a value to reword the English; copy this file to <code>.toml\n'
           '# to start another language. Written by tools/i18n_check.py en --fill.\n'
           'name = "English"\n\n')


def fill_english(folder, used, what):
    """`folder`/i18n/en.toml: every text used, the values already there kept."""
    old = catalog(folder, 'en')
    texts = {t: old.get(t, t) for t in sorted(set(used), key=str.casefold)}
    os.makedirs(os.path.join(folder, 'i18n'), exist_ok=True)
    with open(os.path.join(folder, 'i18n', 'en.toml'), 'w', encoding='utf-8') as f:
        f.write(EN_HEAD.format(what=what) + '[text]\n')
        for k, v in texts.items():
            f.write(f'{json.dumps(k, ensure_ascii=False)} = {json.dumps(v, ensure_ascii=False)}\n')


def placeholders(s):
    return sorted(re.findall(r'\{(\w+)\}', s))


def check(name, used, own, lib, optional=()):
    """Problems of one translation file: missing texts, other placeholders, unused entries."""
    problems = []
    for t in sorted(set(used)):
        got = own.get(t, lib.get(t))
        if got is None:
            if t not in optional:
                problems.append(f'{name}: no translation: {t!r}')
        elif placeholders(got) != placeholders(t):
            problems.append(f'{name}: placeholders differ: {t!r} → {got!r}')
    for t in sorted(set(own) - set(used)):
        problems.append(f'{name}: unused: {t!r}')
    return problems


def main():
    args = [a for a in sys.argv[1:] if not a.startswith('--')]
    code = args[0] if args else 'fr'
    fill = '--fill' in sys.argv
    if fill and code != 'en':
        raise SystemExit('--fill writes the English files only (the others are translations)')
    lib_used = [t for f in sorted(os.listdir(LIB)) if f.endswith('.py')
                for t in tr_texts(os.path.join(LIB, f))]
    if fill:
        fill_english(LIB, lib_used + ['.'], 'the Python library')
        for m in sorted(os.listdir(MODULES)):
            folder = os.path.join(MODULES, m)
            if os.path.isfile(os.path.join(folder, 'module.toml')):
                used = [t for f in sorted(os.listdir(folder)) if f.endswith('.py')
                        for t in tr_texts(os.path.join(folder, f))]
                fill_english(folder, used + toml_texts(os.path.join(folder, 'module.toml')),
                             'this module')
    lib = catalog(LIB, code)
    problems = check('hsm01', lib_used + ['.'], lib, {})
    total = len(lib)
    for m in sorted(os.listdir(MODULES)):
        folder = os.path.join(MODULES, m)
        if not os.path.isfile(os.path.join(folder, 'module.toml')):
            continue
        used = [t for f in sorted(os.listdir(folder)) if f.endswith('.py')
                for t in tr_texts(os.path.join(folder, f))]
        # module.toml texts that read the same in every language need no entry
        same = toml_texts(os.path.join(folder, 'module.toml'))
        own = catalog(folder, code)
        problems += check(m, used + same, own, lib, optional=set(same))
        total += len(own)
    # the template of « Create a module »: tool_template.py, the description of its module.toml
    # (TOOL_TOML in tools.rs), its tool_template_<code>.toml
    desc = re.search(r'^description = "(.*)"$', open(TOOLS_RS, encoding='utf-8').read(), re.M)
    used = tr_texts(os.path.join(EXAMPLES, 'tool_template.py')) + [desc.group(1)]
    try:
        own = tomllib.load(open(os.path.join(EXAMPLES, f'tool_template_{code}.toml'), 'rb'))['text']
    except FileNotFoundError:
        own = {}
    problems += check('template', used, own, lib)
    total += len(own)
    for p in problems:
        print(p)
    print(f'{code}: {total} texts, {len(problems)} problem(s)')
    return 1 if problems else 0


if __name__ == '__main__':
    sys.exit(main())
