#!/usr/bin/env python3
"""Convert a Wireshark *text* export of a USBPcap capture into one TSV line per URB.

Columns: frame  time  src  dst  func  ep  payload_hex
payload = bytes of the hex dump after the USBPcap pseudo-header.
For ISO URBs the payload is the concatenation of all ISO packets.

Usage: python3 -I wsdump.py capture.txt > capture.tsv
"""
import re
import sys

HEX_RE = re.compile(r'^([0-9a-f]{4})  ((?:[0-9a-f]{2} ){1,16})')


def flush(cur, out):
    if not cur or 'hex' not in cur:
        return
    raw = bytes.fromhex(''.join(cur['hex']))
    pl = raw[cur.get('plen', 0):]
    func = cur.get('func', '?')
    out.write('\t'.join([cur.get('frame', '?'), cur.get('time', '?'), cur.get('src', '?'),
                         cur.get('dst', '?'), func, cur.get('ep', '?'), pl.hex()]) + '\n')


def main():
    out = sys.stdout
    cur = None
    with open(sys.argv[1], errors='replace') as f:
        for line in f:
            if line.startswith('Frame ') and ': Packet' in line:
                flush(cur, out)
                cur = {'frame': line.split()[1].rstrip(':'), 'hex': []}
                continue
            if cur is None:
                continue
            s = line.strip()
            m = HEX_RE.match(line)
            if m:
                cur['hex'].append(m.group(2).replace(' ', ''))
            elif s.startswith('[Time since reference or first frame:'):
                cur['time'] = s.split(':', 1)[1].split()[0]
            elif s.startswith('[Source:'):
                cur['src'] = s[8:].strip(' ]')
            elif s.startswith('[Destination:'):
                cur['dst'] = s[13:].strip(' ]')
            elif s.startswith('URB Function:'):
                cur['func'] = s.split(':', 1)[1].split('(')[0].strip().replace('URB_FUNCTION_', '')
            elif s.startswith('Endpoint: 0x') and 'ep' not in cur:
                cur['ep'] = s.split()[1].rstrip(',')
            elif s.startswith('USBPcap pseudoheader length:'):
                cur['plen'] = int(s.split(':')[1])
    flush(cur, out)


if __name__ == '__main__':
    main()
