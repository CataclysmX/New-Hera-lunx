#!/usr/bin/env python3
"""Analyse a tools/fw_knobs.py capture: activity windows per knob, A/B shape, and which
geometric invariant (diamond |a|+|b| for triangle wipers, circle a²+b² for sine wipers) is
constant while the knob turns.
Usage: python3 -I tools/re/knob_analyze.py re_work/knobs/turns1.csv [knob 1-4 for a trace]
"""
import csv
import math
import sys

MID = 2048


def load(path):
    with open(path) as f:
        rows = list(csv.DictReader(f))
    t = [float(r['t']) for r in rows]
    w = [[int(r[f'k{k}{s}']) for r in rows] for k in range(1, 5) for s in 'ab']
    return t, w


def windows(t, a, b, thr=40, gap=0.5):
    """Time windows where (a, b) moves more than thr per 100 ms."""
    act, out, start, last = [], [], None, None
    for i in range(10, len(t)):
        if abs(a[i] - a[i - 10]) + abs(b[i] - b[i - 10]) > thr:
            if start is None:
                start = t[i - 10]
            last = t[i]
        elif start is not None and t[i] - last > gap:
            out.append((start, last))
            start = None
    if start is not None:
        out.append((start, last))
    return out


def phase(a, b):
    """Position on the diamond in cycles [0, 1): quadrature triangle wipers."""
    x, y = a - MID, b - MID
    s = abs(x) + abs(y) or 1
    if x >= 0 and y >= 0:
        p = y / s
    elif x < 0 <= y:
        p = 1 + -x / s
    elif x < 0 and y < 0:
        p = 2 + -y / s
    else:
        p = 3 + x / s
    return p / 4


def unwrap_turns(a, b, i0, i1):
    """Net and absolute travel (cycles) between samples i0..i1."""
    net = path = 0.0
    prev = phase(a[i0], b[i0])
    for i in range(i0 + 1, i1 + 1):
        p = phase(a[i], b[i])
        d = (p - prev + 0.5) % 1.0 - 0.5
        net += d
        path += abs(d)
        prev = p
    return net, path


class StockKnob:
    """Official firmware 1.42 decoder (FUN_0800e128 + FUN_08000b2c), on 8-bit wiper values."""

    def __init__(self, a, b):
        self.fa, self.fb = a >> 4, b >> 4  # hysteresis filter state
        self.pa, self.pb = self.fa, self.fb
        self.acc = 0

    @staticmethod
    def hyst(state, v):
        if v > state + 2:
            state = v - 2
        if v < state - 2:
            state = v + 2
        return state

    def feed(self, a, b):
        self.fa = self.hyst(self.fa, a >> 4)
        self.fb = self.hyst(self.fb, b >> 4)
        a, b = self.fa, self.fb
        if a == self.pa or b == self.pb:  # both wipers must have moved
            return
        sa, sb = (1 if a > self.pa else -1), (1 if b > self.pb else -1)
        da, db = abs(a - self.pa), abs(b - self.pb)
        if sa < 0 and sb < 0:
            dec = b < a
        elif sa > 0 and sb > 0:
            dec = a < b
        elif sa > 0 > sb:
            dec = a > 0x7f or b > 0x7f
        else:
            dec = a < 0x7f or b < 0x7f
        d = da if 51 <= a <= 204 else db
        self.acc += -d if dec else d
        self.pa, self.pb = a, b


def stats(v):
    m = sum(v) / len(v)
    return m, math.sqrt(sum((x - m) ** 2 for x in v) / len(v))


def main():
    t, w = load(sys.argv[1])
    trace = int(sys.argv[2]) if len(sys.argv) > 2 else None
    for k in range(4):
        a, b = w[2 * k], w[2 * k + 1]
        ws = windows(t, a, b)
        print(f'knob{k + 1}: windows ' + ', '.join(f'{s:.1f}-{e:.1f}s' for s, e in ws))
        for s, e in ws:
            i0 = next(i for i in range(len(t)) if t[i] >= s)
            i1 = max(i for i in range(len(t)) if t[i] <= e)
            net, path = unwrap_turns(a, b, i0, i1)
            sk = StockKnob(a[i0], b[i0])
            for i in range(i0 + 1, i1 + 1):
                sk.feed(a[i], b[i])
            print(f'   {s:5.1f}-{e:5.1f}s  net {net:+.2f} cycles  path {path:.2f}'
                  f'  | stock decoder {sk.acc:+5d} counts ({sk.acc / 512:+.2f} x 512)')
        idx = [i for i in range(len(t)) if any(s <= t[i] <= e for s, e in ws)]
        if not idx:
            continue
        da = [a[i] - MID for i in idx]
        db = [b[i] - MID for i in idx]
        diamond = [abs(x) + abs(y) for x, y in zip(da, db)]
        circle = [math.hypot(x, y) for x, y in zip(da, db)]
        md, sd = stats(diamond)
        mc, sc = stats(circle)
        print(f'   moving samples {len(idx)}: |a|+|b| = {md:.0f} ± {sd:.0f}   '
              f'hypot = {mc:.0f} ± {sc:.0f}   a+b mean {stats([x + y for x, y in zip(a, b)])[0]:.0f}')
        # 2D occupancy of the (a, b) plane, 16x16 cells, to see the curve shape
        grid = [[0] * 16 for _ in range(16)]
        for i in idx:
            grid[15 - b[i] * 16 // 4096][a[i] * 16 // 4096] += 1
        for row in grid:
            print('   ' + ''.join('#' if c > 20 else '+' if c > 3 else '.' if c else ' ' for c in row))
    if trace:
        a, b = w[2 * (trace - 1)], w[2 * trace - 1]
        print(f'--- knob{trace} cumulative phase (cycles), every 0.5 s')
        cum, prev, line = 0.0, phase(a[0], b[0]), []
        for i in range(1, len(t)):
            p = phase(a[i], b[i])
            cum += (p - prev + 0.5) % 1.0 - 0.5
            prev = p
            if i % 50 == 0:
                line.append(f'{t[i]:5.1f}:{cum:+.2f}')
        for j in range(0, len(line), 8):
            print('   ' + '  '.join(line[j:j + 8]))
        ws = windows(t, a, b)
        for s, e in ws[:2]:
            print(f'--- knob{trace} trace {s:.1f}-{e:.1f}s (every 5th report: t a b)')
            for i in range(0, len(t), 5):
                if s <= t[i] <= e:
                    print(f'{t[i]:7.2f} {a[i]:5d} {b[i]:5d}')


if __name__ == '__main__':
    main()
