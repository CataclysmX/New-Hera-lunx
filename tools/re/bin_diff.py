#!/usr/bin/env python3
"""Compare two firmware images: differing byte count, first/last differing offsets,
and the printable strings present in only one of them.
Usage: python3 -I bin_diff.py <a> <b> [min_str_len=8]
"""
import re
import sys

a, b = open(sys.argv[1], 'rb').read(), open(sys.argv[2], 'rb').read()
n = int(sys.argv[3]) if len(sys.argv) > 3 else 8
diff = [i for i in range(min(len(a), len(b))) if a[i] != b[i]]
print(f'sizes {len(a)} / {len(b)}, differing bytes {len(diff)}')
if diff:
    print(f'first diff {diff[0]:#x}, last diff {diff[-1]:#x}')
pat = re.compile(rb'[\x20-\x7e]{%d,}' % n)
sa, sb = set(pat.findall(a)), set(pat.findall(b))
print('only in A:', sorted(s.decode() for s in sa - sb)[:40])
print('only in B:', sorted(s.decode() for s in sb - sa)[:40])
