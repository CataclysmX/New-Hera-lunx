#!/usr/bin/env python3
"""A fake OBS Studio WebSocket server (obs-websocket v5) for the OBS module's tests: password,
three scenes, audio inputs (with meters), a camera with filters, a media source, the stream, the
recording, the replay buffer, the virtual camera, the studio mode, profiles and scene collections.
Prints `port N`, then a line per request (`requête Type {data}`) and per event sent
(`évènement Type {data}`; not the meters, 20 a second).

Usage: python3 -I fake_obs.py [--password MOT] [--port N] [--no-replay]
"""
import argparse
import base64
import hashlib
import json
import math
import os
import socket
import struct
import sys
import threading
import time

GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
OUT = 'OBS_WEBSOCKET_OUTPUT_'

ap = argparse.ArgumentParser()
ap.add_argument('--password', default='motdepasse')
ap.add_argument('--port', type=int, default=0)
ap.add_argument('--no-replay', action='store_true', help='replay buffer off in the settings')
args = ap.parse_args()

lock = threading.RLock()
clients = []  # (connection, its subscriptions)
# the OBS window's order, top first (GetSceneList gives them the other way round)
scenes = ['Démarrage', 'Jeu', 'Pause']
state = {'program': 'Démarrage', 'preview': 'Jeu', 'studio': False,
         'stream': False, 'stream_at': 0.0, 'record': False, 'record_at': 0.0, 'paused': False,
         'replay': False, 'vcam': False, 'profile': 'Principal', 'collection': 'Scènes'}
profiles, collections = ['Principal', 'Test'], ['Scènes', 'Autre']
inputs = {  # name -> kind, audio volume (mul) or None, muted
    'Mic/Aux': ['pulse_input_capture', 1.0, False],
    'Audio du bureau': ['pulse_output_capture', 0.5, False],
    'Musique': ['ffmpeg_source', 0.25, False],
    'Caméra': ['v4l2_input', None, False],
    'Titre': ['text_ft2_source_v2', None, False],
}
items = {'Démarrage': [['Titre', True]], 'Jeu': [['Caméra', True], ['Titre', False], ['Musique', True]],
         'Pause': [['Musique', True]]}
filters = {'Caméra': [['Flou', False], ['Correction', True]]}


def log(text):
    print(text, flush=True)


# --- WebSocket (server end) ---------------------------------------------------------------------
def handshake(conn):
    data = b''
    while b'\r\n\r\n' not in data:
        chunk = conn.recv(4096)
        if not chunk:
            raise ConnectionError('closed')
        data += chunk
    head = data.split(b'\r\n\r\n', 1)[0].decode('latin-1')
    key = next(line.split(':', 1)[1].strip() for line in head.split('\r\n')
               if line.lower().startswith('sec-websocket-key:'))
    accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
    conn.sendall(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n'
                  f'Sec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n').encode())


def exact(conn, n):
    buf = b''
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            raise ConnectionError('closed')
        buf += chunk
    return buf


def recv(conn):
    b0, b1 = exact(conn, 2)
    op, n = b0 & 0x0F, b1 & 0x7F
    if n == 126:
        n = struct.unpack('>H', exact(conn, 2))[0]
    elif n == 127:
        n = struct.unpack('>Q', exact(conn, 8))[0]
    key = exact(conn, 4)
    data = bytes(b ^ key[i % 4] for i, b in enumerate(exact(conn, n)))
    if op == 0x8:
        raise ConnectionError('closed by the client')
    return data.decode()


def frame(op, data):
    n = len(data)
    if n < 126:
        head = struct.pack('>BB', 0x80 | op, n)
    elif n < 1 << 16:
        head = struct.pack('>BBH', 0x80 | op, 126, n)
    else:
        head = struct.pack('>BBQ', 0x80 | op, 127, n)
    return head + data


def send(conn, op, d):
    with lock:
        try:
            conn.sendall(frame(0x1, json.dumps({'op': op, 'd': d}).encode()))
        except OSError:
            pass


def close(conn, code, reason):
    try:
        conn.sendall(frame(0x8, struct.pack('>H', code) + reason.encode()))
    except OSError:
        pass
    conn.close()


def event(kind, data, bit=0):
    """Send event `kind` to the clients subscribed to its category."""
    if bit != 1 << 16:
        log(f'évènement {kind} {json.dumps(data, ensure_ascii=False)}')
    with lock:
        for conn, subs in list(clients):
            if not bit or subs & bit:
                send(conn, 5, {'eventType': kind, 'eventIntent': bit, 'eventData': data})


# --- requests -------------------------------------------------------------------------------------
class Refused(Exception):
    def __init__(self, code, comment):
        super().__init__(comment)
        self.code = code


def audio(name):
    if name not in inputs:
        raise Refused(600, f'No source was found by the name of `{name}`.')
    if inputs[name][1] is None:
        raise Refused(604, 'The specified input does not support audio.')
    return inputs[name]


def output(name, on, extra=None):
    key = {'stream': 'Stream', 'record': 'Record', 'replay': 'ReplayBuffer', 'vcam': 'Virtualcam'}[name]
    if state[name] == on:
        raise Refused(500 if on else 501, 'output already in that state')
    for step in ('STARTING', 'STARTED') if on else ('STOPPING', 'STOPPED'):
        if step == 'STARTED' or step == 'STOPPED':
            state[name] = on
            state[f'{name}_at'] = time.monotonic()
        event(f'{key}StateChanged', dict({'outputActive': state[name], 'outputState': OUT + step},
                                          **(extra or {})), 64)


def duration(name):
    return int((time.monotonic() - state[f'{name}_at']) * 1000) if state[name] else 0


def handle(kind, d):
    if kind == 'GetVersion':
        return {'obsVersion': '32.0.0', 'obsWebSocketVersion': '5.6.0', 'rpcVersion': 1}
    if kind == 'GetSceneList':
        n = len(scenes)
        lst = [{'sceneName': s, 'sceneIndex': n - 1 - i} for i, s in enumerate(scenes)][::-1]
        return {'currentProgramSceneName': state['program'],
                'currentPreviewSceneName': state['preview'] if state['studio'] else None, 'scenes': lst}
    if kind in ('SetCurrentProgramScene', 'SetCurrentPreviewScene'):
        if d.get('sceneName') not in scenes:
            raise Refused(600, 'No scene was found')
        if kind == 'SetCurrentPreviewScene' and not state['studio']:
            raise Refused(506, 'Studio mode is not active.')
        which = 'program' if kind == 'SetCurrentProgramScene' else 'preview'
        state[which] = d['sceneName']
        event('CurrentProgramSceneChanged' if which == 'program' else 'CurrentPreviewSceneChanged',
              {'sceneName': d['sceneName']}, 4)
        return {}
    if kind == 'GetStudioModeEnabled':
        return {'studioModeEnabled': state['studio']}
    if kind == 'SetStudioModeEnabled':
        state['studio'] = bool(d.get('studioModeEnabled'))
        event('StudioModeStateChanged', {'studioModeEnabled': state['studio']}, 1024)
        return {}
    if kind == 'TriggerStudioModeTransition':
        if not state['studio']:
            raise Refused(506, 'Studio mode is not active.')
        state['program'], state['preview'] = state['preview'], state['program']
        event('CurrentProgramSceneChanged', {'sceneName': state['program']}, 4)
        event('CurrentPreviewSceneChanged', {'sceneName': state['preview']}, 4)
        return {}
    if kind == 'GetInputList':
        return {'inputs': [{'inputName': n, 'inputKind': v[0], 'unversionedInputKind': v[0]}
                           for n, v in inputs.items()]}
    if kind == 'GetSpecialInputs':
        return {'desktop1': 'Audio du bureau', 'desktop2': None, 'mic1': 'Mic/Aux', 'mic2': None,
                'mic3': None, 'mic4': None}
    if kind == 'GetInputVolume':
        mul = audio(d.get('inputName'))[1]
        return {'inputVolumeMul': mul, 'inputVolumeDb': 20 * math.log10(mul) if mul > 0 else None}
    if kind == 'SetInputVolume':
        a = audio(d.get('inputName'))
        mul = d['inputVolumeMul'] if 'inputVolumeMul' in d else 10 ** (d['inputVolumeDb'] / 20)
        a[1] = max(0.0, min(20.0, float(mul)))
        event('InputVolumeChanged', {'inputName': d['inputName'], 'inputVolumeMul': a[1],
                                     'inputVolumeDb': 20 * math.log10(a[1]) if a[1] > 0 else None}, 8)
        return {}
    if kind == 'GetInputMute':
        return {'inputMuted': audio(d.get('inputName'))[2]}
    if kind in ('SetInputMute', 'ToggleInputMute'):
        a = audio(d.get('inputName'))
        a[2] = (not a[2]) if kind == 'ToggleInputMute' else bool(d.get('inputMuted'))
        event('InputMuteStateChanged', {'inputName': d['inputName'], 'inputMuted': a[2]}, 8)
        return {'inputMuted': a[2]} if kind == 'ToggleInputMute' else {}
    if kind == 'GetStreamStatus':
        return {'outputActive': state['stream'], 'outputReconnecting': False,
                'outputDuration': duration('stream'), 'outputTimecode': '00:00:00.000'}
    if kind in ('StartStream', 'StopStream', 'ToggleStream'):
        on = {'StartStream': True, 'StopStream': False}.get(kind, not state['stream'])
        output('stream', on)
        return {'outputActive': on} if kind == 'ToggleStream' else {}
    if kind == 'GetRecordStatus':
        return {'outputActive': state['record'], 'outputPaused': state['paused'],
                'outputDuration': duration('record'), 'outputTimecode': '00:00:00.000'}
    if kind in ('StartRecord', 'StopRecord', 'ToggleRecord'):
        on = {'StartRecord': True, 'StopRecord': False}.get(kind, not state['record'])
        state['paused'] = False
        output('record', on, {'outputPath': '/tmp/enregistrement.mkv'} if not on else None)
        return {'outputPath': '/tmp/enregistrement.mkv'} if kind == 'StopRecord' else {}
    if kind in ('PauseRecord', 'ResumeRecord', 'ToggleRecordPause'):
        if not state['record']:
            raise Refused(501, 'The output is not running.')
        state['paused'] = {'PauseRecord': True, 'ResumeRecord': False}.get(kind, not state['paused'])
        event('RecordStateChanged', {'outputActive': True, 'outputState':
                                     OUT + ('PAUSED' if state['paused'] else 'RESUMED')}, 64)
        return {}
    if kind in ('GetReplayBufferStatus', 'StartReplayBuffer', 'StopReplayBuffer', 'ToggleReplayBuffer',
                'SaveReplayBuffer'):
        if args.no_replay:
            raise Refused(604, 'Replay buffer is not available.')
        if kind == 'GetReplayBufferStatus':
            return {'outputActive': state['replay']}
        if kind == 'SaveReplayBuffer':
            if not state['replay']:
                raise Refused(501, 'The output is not running.')
            event('ReplayBufferSaved', {'savedReplayPath': '/tmp/replay.mkv'}, 64)
            return {}
        output('replay', {'StartReplayBuffer': True, 'StopReplayBuffer': False}.get(kind, not state['replay']))
        return {}
    if kind == 'GetVirtualCamStatus':
        return {'outputActive': state['vcam']}
    if kind in ('StartVirtualCam', 'StopVirtualCam', 'ToggleVirtualCam'):
        output('vcam', {'StartVirtualCam': True, 'StopVirtualCam': False}.get(kind, not state['vcam']))
        return {'outputActive': state['vcam']} if kind == 'ToggleVirtualCam' else {}
    if kind == 'GetSceneItemList':
        if d.get('sceneName') not in items:
            raise Refused(600, 'No scene was found')
        return {'sceneItems': [{'sceneItemId': i + 1, 'sceneItemIndex': i, 'sourceName': s, 'sceneItemEnabled': on,
                                'isGroup': False, 'inputKind': inputs[s][0]}
                               for i, (s, on) in enumerate(items[d['sceneName']])][::-1]}
    if kind in ('GetSceneItemId', 'GetSceneItemEnabled', 'SetSceneItemEnabled'):
        lst = items.get(d.get('sceneName'))
        if lst is None:
            raise Refused(600, 'No scene was found')
        if kind == 'GetSceneItemId':
            for i, (s, _) in enumerate(lst):
                if s == d.get('sourceName'):
                    return {'sceneItemId': i + 1}
            raise Refused(600, 'No scene items were found')
        i = int(d.get('sceneItemId', 0)) - 1
        if not 0 <= i < len(lst):
            raise Refused(600, 'No scene item was found')
        if kind == 'GetSceneItemEnabled':
            return {'sceneItemEnabled': lst[i][1]}
        lst[i][1] = bool(d.get('sceneItemEnabled'))
        event('SceneItemEnableStateChanged', {'sceneName': d['sceneName'], 'sceneItemId': i + 1,
                                              'sceneItemEnabled': lst[i][1]}, 128)
        return {}
    if kind == 'GetSourceFilterList':
        if d.get('sourceName') not in inputs and d.get('sourceName') not in scenes:
            raise Refused(600, 'No source was found')
        return {'filters': [{'filterName': f, 'filterEnabled': on, 'filterIndex': i, 'filterKind': 'x'}
                            for i, (f, on) in enumerate(filters.get(d['sourceName'], []))]}
    if kind in ('GetSourceFilter', 'SetSourceFilterEnabled'):
        for f in filters.get(d.get('sourceName'), []):
            if f[0] == d.get('filterName'):
                if kind == 'GetSourceFilter':
                    return {'filterEnabled': f[1], 'filterIndex': 0, 'filterKind': 'x', 'filterSettings': {}}
                f[1] = bool(d.get('filterEnabled'))
                event('SourceFilterEnableStateChanged', {'sourceName': d['sourceName'],
                                                         'filterName': f[0], 'filterEnabled': f[1]}, 32)
                return {}
        raise Refused(600, 'No filter was found')
    if kind == 'TriggerMediaInputAction':
        if inputs.get(d.get('inputName'), [''])[0] != 'ffmpeg_source':
            raise Refused(604, 'not a media input')
        event('MediaInputActionTriggered', {'inputName': d['inputName'], 'mediaAction': d.get('mediaAction')}, 256)
        event('MediaInputPlaybackStarted', {'inputName': d['inputName']}, 256)
        return {}
    if kind == 'GetProfileList':
        return {'currentProfileName': state['profile'], 'profiles': profiles}
    if kind == 'SetCurrentProfile':
        if d.get('profileName') not in profiles:
            raise Refused(600, 'No profile was found')
        state['profile'] = d['profileName']
        event('CurrentProfileChanged', {'profileName': state['profile']}, 2)
        return {}
    if kind == 'GetSceneCollectionList':
        return {'currentSceneCollectionName': state['collection'], 'sceneCollections': collections}
    if kind == 'SetCurrentSceneCollection':
        if d.get('sceneCollectionName') not in collections:
            raise Refused(600, 'No scene collection was found')
        state['collection'] = d['sceneCollectionName']
        event('CurrentSceneCollectionChanged', {'sceneCollectionName': state['collection']}, 2)
        return {}
    raise Refused(204, f'Your request type is not valid: {kind}')


# --- one client --------------------------------------------------------------------------------
def serve(conn):
    try:
        handshake(conn)
        challenge, salt = base64.b64encode(os.urandom(12)).decode(), base64.b64encode(os.urandom(12)).decode()
        hello = {'obsWebSocketVersion': '5.6.0', 'rpcVersion': 1}
        if args.password:
            hello['authentication'] = {'challenge': challenge, 'salt': salt}
        send(conn, 0, hello)
        ident = json.loads(recv(conn))
        if ident.get('op') != 1:
            return close(conn, 4008, 'not identified')
        d = ident.get('d') or {}
        if args.password:
            secret = base64.b64encode(hashlib.sha256((args.password + salt).encode()).digest()).decode()
            want = base64.b64encode(hashlib.sha256((secret + challenge).encode()).digest()).decode()
            if d.get('authentication') != want:
                log('connexion refusée : mot de passe')
                return close(conn, 4009, 'Authentication failed.')
        subs = int(d.get('eventSubscriptions', (1 << 11) - 1))
        log(f'connecté (abonnements {subs})')
        send(conn, 2, {'negotiatedRpcVersion': 1})
        with lock:
            clients.append((conn, subs))
        while True:
            msg = json.loads(recv(conn))
            if msg.get('op') != 6:
                continue
            r = msg['d']
            kind, data = r.get('requestType', ''), r.get('requestData') or {}
            log(f'requête {kind} {json.dumps(data, ensure_ascii=False)}')
            try:
                with lock:
                    out = handle(kind, data)
                status = {'result': True, 'code': 100}
            except Refused as e:
                out, status = None, {'result': False, 'code': e.code, 'comment': str(e)}
            reply = {'requestType': kind, 'requestId': r.get('requestId'), 'requestStatus': status}
            if out is not None:
                reply['responseData'] = out
            send(conn, 7, reply)
    except (ConnectionError, OSError, ValueError, StopIteration):
        pass
    finally:
        with lock:
            clients[:] = [c for c in clients if c[0] is not conn]
        conn.close()


def meters():
    """Levels of the audio inputs, 20 a second (a slow sine wave, nothing when muted)."""
    t0 = time.monotonic()
    while True:
        time.sleep(0.05)
        t = time.monotonic() - t0
        with lock:
            lst = [{'inputName': n, 'inputLevelsMul': [[0.0, 0.0 if v[2] else v[1] * (0.3 + 0.25 * math.sin(t)), 0.0]] * 2}
                   for n, v in inputs.items() if v[1] is not None]
        event('InputVolumeMeters', {'inputs': lst}, 1 << 16)


srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(('127.0.0.1', args.port))
srv.listen(4)
log(f'port {srv.getsockname()[1]}')
threading.Thread(target=meters, daemon=True).start()
try:
    while True:
        c, _ = srv.accept()
        threading.Thread(target=serve, args=(c,), daemon=True).start()
except KeyboardInterrupt:
    sys.exit(0)
