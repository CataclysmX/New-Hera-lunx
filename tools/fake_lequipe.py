#!/usr/bin/env python3
"""A fake L'Équipe live feed for the live scores module's test, shaped like
https://dwh.lequipe.fr/api/live/lives (listings sport → competition → events, same keys):
Football / Ligue 1: Nice - Paris-SG under way (1-0, 45'), Lens - Lille to come; Basket / NBA:
Atlanta Hawks - Charlotte Hornets at half-time (56-57); a TV programme (left out).
GET /api/live/lives: the feed (gzip if asked), one line « GET lives » per request.
GET /control/goal: Nice scores. Prints `port N` first.

Usage: python3 -I fake_lequipe.py
"""
import gzip
import http.server
import json
import threading

state = {'nice': 1}
lock = threading.Lock()


def team(name, short=None):
    return {'__type': 'effectif_sport_collectif', 'equipe': {'__type': 'equipe', 'id': '1', 'nom': name,
                                                            'rang': '1', 'short_name': short or name,
                                                            'url_image': ''}}


def match(id, sport, home, away, score, status, kind, live):
    w = {'__type': 'sport_event_widget', 'hash': id, 'plugins': [],
         'event': {'__type': 'rencontre_sport_collectif', 'id': id, 'commente': True,
                   'date': '2026-11-21T17:15:00+01:00', 'sport': {'__type': 'sport', 'id': '1', 'nom': sport},
                   'statut': {'__type': 'evenement_statut', 'libelle': status, 'periode': 'X', 'type': kind},
                   'specifics': {'__type': 'specifics_sport_collectif', 'domicile': team(*home),
                                 'exterieur': team(*away), 'vainqueur': None}}}
    if score:
        w['event']['specifics']['score'] = {'__type': 'score', 'domicile': score[0], 'exterieur': score[1]}
    if live:
        w['content_filters_matching'] = {'__type': 'content_filters_matching', 'filters': [
            {'__type': 'content_filter_matching', 'code': 'live-active', 'values': ['is_live']}]}
    return w


def listing(title, items):
    return {'__type': 'live_listing_widget', 'title': title, 'items': items}


def feed():
    with lock:
        nice = str(state['nice'])
    football = listing('Football', [listing('Ligue 1', [listing('13e journée', [
        match('1', 'Football', ('Nice',), ('Paris-SG',), (nice, '0'), "45'", 'encours', True),
        match('2', 'Football', ('Lens',), ('Lille', 'LOSC'), None, '21h05', 'avenir', False)])])])
    basket = listing('Basket', [listing('NBA', [listing('Saison régulière', [
        match('3', 'Basket', ('Atlanta Hawks',), ('Charlotte Hornets',), ('56', '57'), 'mi-temps',
              'encours', True)])])])
    tv = {'__type': 'sport_event_widget', 'event': {'__type': 'allo', 'titre': 'Magazine',
                                                    'statut': {'type': 'encours', 'libelle': ''}}}
    return {'__type': 'base_response', 'data': [{'__type': 'page_descriptor', 'content': {
        '__type': 'page_descriptor_content', 'feed': {'__type': 'page_descriptor_feed',
                                                      'items': [football, basket, tv]}}}]}


class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.startswith('/control/goal'):
            with lock:
                state['nice'] += 1
            body = b'ok'
        elif self.path.startswith('/api/live/lives'):
            print('GET lives', flush=True)
            body = json.dumps(feed(), ensure_ascii=False).encode()
            if 'gzip' in (self.headers.get('Accept-Encoding') or ''):
                body = gzip.compress(body)
                gz = True
            else:
                gz = False
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header('Content-Type', 'application/vnd.lequipe.mt+json; charset=utf-8')
        if self.path.startswith('/api/') and gz:
            self.send_header('Content-Encoding', 'gzip')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


srv = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
print(f'port {srv.server_address[1]}', flush=True)
srv.serve_forever()
