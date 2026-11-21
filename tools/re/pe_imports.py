#!/usr/bin/env python3
"""List imported DLLs (+ optionally functions) and exports of PE files.
Usage: python3 -I pe_imports.py [--funcs] [--exports] file...
"""
import sys

import pefile

args = sys.argv[1:]
funcs = '--funcs' in args
exps = '--exports' in args
for path in [a for a in args if not a.startswith('-')]:
    pe = pefile.PE(path, fast_load=True)
    pe.parse_data_directories(directories=[
        pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_IMPORT'],
        pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_EXPORT'],
        pefile.DIRECTORY_ENTRY['IMAGE_DIRECTORY_ENTRY_DELAY_IMPORT']])
    print(f'== {path}')
    for attr in ('DIRECTORY_ENTRY_IMPORT', 'DIRECTORY_ENTRY_DELAY_IMPORT'):
        for e in getattr(pe, attr, []):
            names = [i.name.decode() if i.name else f'#{i.ordinal}' for i in e.imports]
            if funcs:
                print(f'  {e.dll.decode()}: {" ".join(names)}')
            else:
                print(f'  {e.dll.decode()} ({len(names)})')
    if exps and hasattr(pe, 'DIRECTORY_ENTRY_EXPORT'):
        print('  exports:', ' '.join(
            (s.name.decode() if s.name else f'#{s.ordinal}') for s in pe.DIRECTORY_ENTRY_EXPORT.symbols))
