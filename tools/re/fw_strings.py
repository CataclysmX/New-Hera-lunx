#!/usr/bin/env python3
"""Print "word-like" NUL-terminated ASCII strings of a raw firmware image (base 0x08000000),
filtering out the RLE graphics noise. Usage: python3 -I fw_strings.py <fw_body.bin> [min_len=4]
"""
import re
import sys

data = open(sys.argv[1], 'rb').read()
min_len = int(sys.argv[2]) if len(sys.argv) > 2 else 4
for m in re.finditer(rb'([\x20-\x7e]{%d,})\x00' % min_len, data):
    s = m.group(1).decode()
    letters = sum(c.isalpha() for c in s)
    words = re.findall(r'[A-Za-z]{3,}', s)
    # keep strings that look like text: mostly letters/digits/spaces and at least one real word
    if words and letters >= 0.5 * len(s.replace(' ', '')) and not re.search(r'(.)\1\1\1', s):
        print(f'{0x08000000 + m.start():#010x}  {s}')
