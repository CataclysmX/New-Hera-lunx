#!/usr/bin/env python3
"""Module « Système » de new hera-nux: page « Système » (processor, memory, temperatures,
disks, batteries of the peripherals) and alerts (a temperature held above its threshold, disk
almost full, low battery), read from /proc and /sys (nvidia-smi for NVIDIA cards, if present).

Settings: systeme.toml of this folder (tab « Système » of the window): page lines, alert style
(card or pill), LED, wake; per alert: on/off, sensor (auto, or chip/label as the Journal lists
them at start), threshold, how long above it before alerting, hysteresis, reminder.
Options: --no-page (alerts only), --show (page shown at start), --name NAME; the former
--cpu-temp, --gpu-temp, --disk, --battery override the thresholds of systeme.toml.
"""
import argparse
import copy
import glob
import os
import re
import shutil
import subprocess
import time
import tomllib

from hsm01 import Client
from hsm01 import widgets as W
from hsm01.i18n import tr

PAGE = 'system'
HERE = os.environ.get('HSM01_MODULE_DIR') or os.path.dirname(os.path.abspath(__file__))
BG, TEXT, GREY, ACCENT, TRACK = '#0b1010', '#e8eeee', '#8a9696', '#00b4a8', '#1c2626'
WARN, HOT = '#e0b040', '#e05040'
ROWS = 6  # lines of the page
SCROLL_H = ROWS * 38 - 8  # height of the scroll bar's track
REAL_FS = {'ext4', 'ext3', 'btrfs', 'xfs', 'f2fs', 'zfs', 'jfs', 'bcachefs', 'ntfs3', 'vfat'}
TEMPS = ('processeur', 'gpu', 'ssd')
LABELS = {'processeur': tr('processor'), 'gpu': tr('graphics card'), 'ssd': 'SSD'}
# « auto » sensor of each kind: its chips, then the preferred labels in order (else the hottest
# of these chips). AMD graphics: edge, not junction (the hot spot, 90-105 °C in games: normal).
AUTO = {'processeur': (('k10temp', 'zenpower', 'coretemp', 'cpu_thermal'),
                       ('Tdie', 'Tctl', 'Package id 0')),
        'gpu': (('amdgpu', 'radeon', 'nouveau'), ('edge', 'temp1')),
        'ssd': (('nvme',), ('Composite',))}
DEFAULTS = {
    'page': {'afficher': True,
             'lignes': ['processeur', 'memoire', 'gpu', 'ssd', 'disques', 'batteries']},
    'alertes': {
        'style': 'carte', 'duree': 10, 'led': 4, 'reveil': True,
        'processeur': {'actif': True, 'sonde': 'auto', 'seuil': 90, 'pendant': 20, 'retour': 5,
                       'rappel': 0},
        'gpu': {'actif': True, 'sonde': 'auto', 'seuil': 90, 'pendant': 20, 'retour': 5,
                'rappel': 0},
        'ssd': {'actif': True, 'sonde': 'auto', 'seuil': 75, 'pendant': 60, 'retour': 5,
                'rappel': 0},
        'disque': {'actif': True, 'seuil': 95, 'ignorer': []},
        'batterie': {'actif': True, 'seuil': 15, 'ignorer': []},
    },
}


def read(path, default=''):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def merge(base, over):
    """`over` (systeme.toml) on top of `base` (defaults), tables merged key by key."""
    out = copy.deepcopy(base)
    for k, v in over.items():
        out[k] = merge(out[k], v) if isinstance(v, dict) and isinstance(out.get(k), dict) else v
    return out


def load_settings(path):
    try:
        with open(path, 'rb') as f:
            return merge(DEFAULTS, tomllib.load(f))
    except FileNotFoundError:
        return copy.deepcopy(DEFAULTS)
    except tomllib.TOMLDecodeError as e:
        print(tr('{path}: {e} (default settings)', path=path, e=e), flush=True)
        return copy.deepcopy(DEFAULTS)


# --- sensors -------------------------------------------------------------------------------
class Cpu:
    def __init__(self):
        self.last = self.times()

    @staticmethod
    def times():
        v = [int(x) for x in read('/proc/stat').splitlines()[0].split()[1:9]]
        return v[3] + v[4], sum(v)  # idle (+ iowait), total

    def percent(self):
        idle, total = self.times()
        di, dt = idle - self.last[0], total - self.last[1]
        self.last = (idle, total)
        return 100.0 * (1 - di / dt) if dt > 0 else 0.0


def memory():
    info = {}
    for line in read('/proc/meminfo').splitlines():
        k, _, v = line.partition(':')
        info[k] = int(v.split()[0]) * 1024
    total, avail = info.get('MemTotal', 1), info.get('MemAvailable', 0)
    return 100.0 * (total - avail) / total, (total - avail) / 2**30, total / 2**30


def number(path):
    m = re.search(r'\d+', os.path.basename(path))
    return int(m.group()) if m else 0


def sensors():
    """The hwmon temperatures: [{'key': 'chip/label', 'alias': 'hwmonN/tempM', 'chip', 'label',
    'path'}] (a chip seen twice, two NVMe drives: 'nvme#2/Composite')."""
    out, seen = [], {}
    for d in sorted(glob.glob('/sys/class/hwmon/hwmon*'), key=number):
        chip = read(f'{d}/name') or os.path.basename(d)
        seen[chip] = seen.get(chip, 0) + 1
        name = chip if seen[chip] == 1 else f'{chip}#{seen[chip]}'
        for t in sorted(glob.glob(f'{d}/temp*_input'), key=number):
            base = t[:-len('_input')]
            label = read(f'{base}_label') or os.path.basename(base)
            out.append({'key': f'{name}/{label}', 'chip': chip, 'label': label, 'path': t,
                        'alias': f'{os.path.basename(d)}/{os.path.basename(base)}'})
    return out


def celsius(s):
    try:
        c = int(read(s['path'], '0')) / 1000
    except ValueError:
        return None
    return c if 0 < c < 150 else None


def pick(all_sensors, kind, wanted):
    """The sensors of `kind` (several: the hottest counts): the one named, or « auto »."""
    if wanted and wanted != 'auto':
        return [s for s in all_sensors if wanted in (s['key'], s['alias'])]
    chips, labels = AUTO[kind]
    mine = [s for s in all_sensors if s['chip'] in chips]
    for label in labels:
        hit = [s for s in mine if s['label'] == label]
        if hit:
            return hit
    return mine


def nvidia_temp():
    if not shutil.which('nvidia-smi'):
        return None
    try:
        r = subprocess.run(['nvidia-smi', '--query-gpu=temperature.gpu',
                            '--format=csv,noheader,nounits'],
                           capture_output=True, text=True, timeout=3)
        return max(float(x) for x in r.stdout.split())
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def disks():
    """[(mount point, % used, free GiB)] of the real file systems (one per device)."""
    seen, out = set(), []
    for line in read('/proc/mounts').splitlines():
        dev, mnt, fs = line.split()[:3]
        if fs not in REAL_FS or not dev.startswith('/dev/') or dev in seen:
            continue
        seen.add(dev)
        mnt = mnt.replace('\\040', ' ')
        try:
            u = shutil.disk_usage(mnt)
        except OSError:
            continue
        out.append((mnt, 100.0 * u.used / u.total, u.free / 2**30))
    return out


def batteries():
    """[(name, %, charging)] of the batteries (peripherals: mouse, headset, pad... and laptop)."""
    out = []
    for d in sorted(glob.glob('/sys/class/power_supply/*')):
        if read(f'{d}/type') != 'Battery':
            continue
        cap = read(f'{d}/capacity')
        if not cap.isdigit():
            level = read(f'{d}/capacity_level')
            cap = {'Critical': '5', 'Low': '15', 'Normal': '60', 'High': '85', 'Full': '100'}.get(level)
            if cap is None:
                continue
        name = read(f'{d}/model_name') or os.path.basename(d)
        out.append((name, int(cap), read(f'{d}/status') == 'Charging'))
    return out


class Watch:
    """A temperature alert: once held `pendant` s at `seuil` or more, again after `rappel` min
    still there, re-armed `retour` °C under the threshold."""

    def __init__(self, cfg):
        self.cfg, self.since, self.alerted, self.last = cfg, None, False, 0.0

    def update(self, t, now):
        """True when an alert is due now."""
        c = self.cfg
        if t >= c['seuil']:
            if self.since is None:
                self.since = now
            if not self.alerted and now - self.since >= c['pendant']:
                self.alerted, self.last = True, now
                return True
            if self.alerted and c['rappel'] > 0 and now - self.last >= c['rappel'] * 60:
                self.last = now
                return True
            return False
        self.since = None
        if t < c['seuil'] - c['retour']:
            self.alerted = False
        return False


# --- module --------------------------------------------------------------------------------
class System:
    def __init__(self, client, args):
        self.c, self.args = client, args
        self.settings = s = load_settings(os.path.join(HERE, 'systeme.toml'))
        a = s['alertes']
        for kind, value in (('processeur', args.cpu_temp), ('gpu', args.gpu_temp)):
            if value is not None:
                a[kind]['seuil'] = value
        if args.disk is not None:
            a['disque']['seuil'] = args.disk
        if args.battery is not None:
            a['batterie']['seuil'] = args.battery
        self.page = s['page']['afficher'] and not args.no_page
        self.cpu = Cpu()
        self.alerting = set()  # disks and batteries alerted, until back to normal
        self.nvidia, self.nvidia_t = None, 5.0
        self.sent = {}
        self.chosen, self.use_nvidia = self.choose_sensors()
        self.watch = {k: Watch(a[k]) for k in TEMPS}
        self.all_rows, self.offset = [], 0  # every line, the first one shown
        if self.page:
            client.page(PAGE, self.widgets(), name=args.name, background=BG, show=args.show)
            with client.lock:
                client.on('turn', self.on_turn)

    def on_turn(self, ev):
        """Any knob turned on the page: the lines scroll (more of them than the page shows)."""
        if ev.get('mine') and ev.get('page') == PAGE:
            self.offset += ev.get('delta', 0)
            self.display()

    def display(self):
        """The lines from `offset`, kept within the list."""
        self.offset = max(0, min(self.offset, len(self.all_rows) - ROWS))
        self.show(self.all_rows[self.offset:self.offset + ROWS], len(self.all_rows))

    def choose_sensors(self):
        """The sensors of each temperature (journal: all of them, and the ones used)."""
        all_sensors = sensors()
        nvidia = shutil.which('nvidia-smi') is not None
        listed = ', '.join(f'{s["key"]} {celsius(s) or 0:.0f} °C' for s in all_sensors)
        print(tr('sensors: {list}', list=listed + (', nvidia-smi' if nvidia else '')), flush=True)
        # the « Sonde » choices of the settings form
        self.c.choices(sondes=['auto'] + [s['key'] for s in all_sensors]
                       + (['nvidia-smi'] if nvidia else []))
        chosen, use_nvidia = {}, False
        for kind in TEMPS:
            cfg = self.settings['alertes'][kind]
            wanted = str(cfg.get('sonde') or 'auto')
            chosen[kind] = [] if wanted == 'nvidia-smi' else pick(all_sensors, kind, wanted)
            if kind == 'gpu' and nvidia and (wanted == 'nvidia-smi' or wanted == 'auto' and not chosen[kind]):
                use_nvidia = True
            names = [s['key'] for s in chosen[kind]] + (['nvidia-smi'] if kind == 'gpu' and use_nvidia else [])
            if wanted != 'auto' and not names:
                print(tr('{label}: sensor "{wanted}" not found (see the list above)', label=LABELS[kind], wanted=wanted),
                      flush=True)
            elif names:
                state = (tr('alert at {limit} °C held {secs} s', limit=cfg['seuil'], secs=cfg['pendant']) if cfg['actif']
                         else tr('alert off'))
                print(tr('{label}: {names}; {state}', label=LABELS[kind], names=', '.join(names), state=state), flush=True)
        return chosen, use_nvidia

    @staticmethod
    def widgets():
        out = [W.text('head', 12, 4, 456, 26, tr('System'), size=15, align='left', color=GREY)]
        for r in range(ROWS):
            y = 38 + r * 38
            out += [W.text(f'l{r}', 12, y, 130, 30, '', size=15, align='left', color=TEXT),
                    W.rect(f'b{r}bg', 146, y + 11, 170, 8, TRACK, radius=4),
                    W.rect(f'b{r}', 146, y + 11, 1, 8, ACCENT, radius=4),
                    W.text(f'v{r}', 322, y, 148, 30, '', size=14, align='right', color=GREY)]
        # scroll bar, when there are more lines than the page shows
        out += [W.rect('sb_bg', -999, 38, 4, SCROLL_H, TRACK, radius=2),
                W.rect('sb', -999, 38, 4, SCROLL_H, GREY, radius=2)]
        return out

    def alert(self, title, text, icon, pill, color, high=False):
        a = self.settings['alertes']
        opts = {'app': tr('System'), 'duration': a['duree'], 'wake': bool(a['reveil'])}
        if a['led']:
            opts['led'] = int(a['led'])
        if high:
            opts['priority'] = 'high'
        if a['style'] == 'pilule':
            self.c.pill(pill, border=color, dot=color, **opts)
        else:
            self.c.alert(title, text, icon=icon, **opts)

    def check(self, key, bad, normal, what):
        """Disks, batteries: one alert when `bad` becomes true, again only after `normal`."""
        if bad and key not in self.alerting:
            self.alerting.add(key)
            self.alert(*what)
        elif normal:
            self.alerting.discard(key)

    def temperatures(self, dt):
        out = {}
        if self.use_nvidia:
            self.nvidia_t += dt
            if self.nvidia_t >= 5:
                self.nvidia_t, self.nvidia = 0.0, nvidia_temp()
        for kind in TEMPS:
            values = [v for v in map(celsius, self.chosen[kind]) if v is not None]
            if kind == 'gpu' and self.use_nvidia and self.nvidia is not None:
                values.append(self.nvidia)
            if values:
                out[kind] = max(values)
        return out

    def tick(self, dt):
        a = self.settings['alertes']
        temps = self.temperatures(dt)
        now = time.monotonic()
        for kind, t in temps.items():
            cfg = a[kind]
            if cfg['actif'] and self.watch[kind].update(t, now):
                label = LABELS[kind]
                self.alert(tr('High temperature'), tr('{label}: {t} °C (limit {limit} °C)', label=label, t=f'{t:.0f}', limit=cfg['seuil']),
                           'dialog-warning', f'{label.upper()}: {t:.0f} °C', HOT,
                           high=t >= cfg['seuil'] + 5)
        disk_list, bats = disks(), batteries()
        d = a['disque']
        for mnt, pct, free in disk_list:
            if d['actif'] and mnt not in d['ignorer']:
                self.check(f'disk-{mnt}', pct >= d['seuil'], pct < d['seuil'] - 2,
                           (tr('Disk almost full'), tr('{mount}: {pct} % used, {free} GiB free', mount=mnt, pct=f'{pct:.0f}', free=f'{free:.1f}'), 'drive-harddisk', tr('DISK {mount}: {pct} %', mount=mnt, pct=f'{pct:.0f}'), WARN))
        b = a['batterie']
        for name, pct, charging in bats:
            if b['actif'] and name not in b['ignorer']:
                self.check(f'bat-{name}', pct <= b['seuil'] and not charging,
                           pct > b['seuil'] + 5 or charging,
                           (tr('Low battery'), f'{name}: {pct} %', 'battery-caution',
                            tr('BATTERY {name}: {pct} %', name=name.upper(), pct=pct), WARN))
        if self.page:
            self.all_rows = self.rows(temps, disk_list, bats)
            self.display()

    def rows(self, temps, disk_list, bats):
        """Lines of the page, in the order of `lignes`: (label, %, value, colour)."""
        a = self.settings['alertes']

        def heat(kind, t):
            limit = a[kind]['seuil']
            return HOT if t >= limit else WARN if t >= limit - 10 else ACCENT

        def level(pct):
            return HOT if pct >= 90 else WARN if pct >= 75 else ACCENT

        out = []
        for line in self.settings['page']['lignes']:
            line = line.replace('é', 'e')
            if line == 'processeur':
                cpu = self.cpu.percent()
                t = temps.get('processeur')
                out.append((tr('Processor'), cpu, f'{cpu:.0f} %' + (f' · {t:.0f} °C' if t else ''),
                            heat('processeur', t) if t else level(cpu)))
            elif line == 'memoire':
                mem, used, total = memory()
                out.append((tr('Memory'), mem, tr('{pct} % · {used}/{total} GiB', pct=f'{mem:.0f}', used=f'{used:.1f}', total=f'{total:.0f}'), level(mem)))
            elif line in ('gpu', 'ssd') and line in temps:
                t = temps[line]
                name = tr('Graphics card') if line == 'gpu' else 'SSD'
                out.append((name, t, f'{t:.0f} °C', heat(line, t)))
            elif line == 'disques':
                out += [(tr('Disk {mount}', mount=mnt), pct, tr('{pct} % · {free} GiB free', pct=f'{pct:.0f}', free=f'{free:.0f}'), level(pct))
                        for mnt, pct, free in disk_list]
            elif line == 'batteries':
                out += [(name, pct, f'{pct} %' + (' · ' + tr('charging') if charging else ''),
                         HOT if pct <= 10 else WARN if pct <= 25 else ACCENT)
                        for name, pct, charging in bats]
        return out

    def show(self, rows, total):
        """`rows` on the page, of `total` lines: where we are in the header and the scroll bar."""
        changes = {}

        def put(wid, **fields):
            if self.sent.get(wid) != fields:
                self.sent[wid] = fields
                changes[wid] = fields

        if total > ROWS:
            put('head', text=tr('System · {first}-{last} of {total} (knob: scroll)', first=self.offset + 1, last=self.offset + len(rows), total=total))
            thumb = max(16, round(SCROLL_H * ROWS / total))
            y = 38 + round((SCROLL_H - thumb) * self.offset / (total - ROWS))
            put('sb_bg', x=474)
            put('sb', x=474, y=y, h=thumb)
        else:
            put('head', text=tr('System'))
            put('sb_bg', x=-999)
            put('sb', x=-999)

        for r in range(ROWS):
            if r < len(rows):
                label, pct, value, color = rows[r]
                put(f'l{r}', text=label)
                put(f'b{r}bg', x=146)
                put(f'b{r}', x=146, w=max(1, round(170 * min(pct, 100) / 100)), color=color)
                put(f'v{r}', text=value)
            else:
                for p in ('l', 'v'):
                    put(f'{p}{r}', text='')
                put(f'b{r}bg', x=-999)
                put(f'b{r}', x=-999)
        self.c.update(PAGE, changes)


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--cpu-temp', type=float, help='°C (otherwise systeme.toml)')
    ap.add_argument('--gpu-temp', type=float, help='°C (otherwise systeme.toml)')
    ap.add_argument('--disk', type=float, help='%% used (otherwise systeme.toml)')
    ap.add_argument('--battery', type=int, help='%% (otherwise systeme.toml)')
    ap.add_argument('--no-page', action='store_true', help='alerts only')
    ap.add_argument('--show', action='store_true', help='show the page at start')
    ap.add_argument('--name', default=tr('System'))
    args = ap.parse_args()
    client = Client(os.environ.get('HSM01_MODULE') or 'system')
    system = System(client, args)
    client.run(tick=system.tick, fps=1)


if __name__ == '__main__':
    main()
