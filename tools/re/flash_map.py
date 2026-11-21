#!/usr/bin/env python3
"""Compare a full flash dump with an official firmware body, page by page (4 KiB).
Prints differing / non-erased regions beyond the image. Usage:
  python3 -I flash_map.py <flash_dump.bin> <fw_body.bin> [base=0x08000000]
"""
import sys

dump, body = open(sys.argv[1], 'rb').read(), open(sys.argv[2], 'rb').read()
base = int(sys.argv[3], 16) if len(sys.argv) > 3 else 0x08000000
PAGE = 4096
same = dump[:len(body)] == body
print(f'dump {len(dump)} B, image {len(body)} B, image region identical: {same}')
if not same:
    diffs = [i for i in range(len(body)) if dump[i] != body[i]]
    print(f'  {len(diffs)} differing bytes, first {base + diffs[0]:#x}, last {base + diffs[-1]:#x}')
runs, cur = [], None
for p in range(len(body) // PAGE, len(dump) // PAGE):
    chunk = dump[p * PAGE:(p + 1) * PAGE]
    state = 'erased' if chunk == b'\xff' * PAGE else 'data'
    if cur and cur[2] == state:
        cur[1] = p
    else:
        cur = [p, p, state]
        runs.append(cur)
for a, b, s in runs:
    start, end = base + a * PAGE, base + (b + 1) * PAGE
    sample = '' if s == 'erased' else dump[a * PAGE:a * PAGE + 32].hex(' ')
    print(f'{start:#010x}-{end:#010x} ({(b - a + 1) * 4:>4} KiB) {s} {sample}')
