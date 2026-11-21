#!/usr/bin/env python3
"""A fake Motor Town for the telemetry modules' tests: native v1.2 datagrams (858 bytes, offsets
of the game's MotorTownTelemetryProtocol.md, written here field by field) sent to 127.0.0.1:PORT
at 60 a second. A truck drives at 48 km/h in third gear, 3450 rpm of 7000, fuel 321.5 / 400 L,
headlights and ABS on, turn signal left, road limit 50; after --race seconds an event race runs
(lap 2/3, position 2 of 8, last lap 83.456 s, best 82.5 s). Prints `envoi N` once started.

Usage: python3 -I fake_motortown.py --port N [--race S] [--forza]
"""
import argparse
import socket
import struct
import time

ap = argparse.ArgumentParser()
ap.add_argument('--port', type=int, required=True)
ap.add_argument('--race', type=float, default=2.0, help='seconds before the race starts')
ap.add_argument('--forza', action='store_true', help='Forza-compatible 311-byte packets instead')
args = ap.parse_args()


def native(t, race):
    d = bytearray(858)
    struct.pack_into('<4sBBH', d, 0, b'MTTM', 1, 2, 850)
    struct.pack_into('<qIqI', d, 8, int(t * 1e6), int(t * 60), -1, 0x12345678)
    struct.pack_into('<ffiif', d, 108, 3450.0, 7000.0, 3, 6, 48.0)  # rpm, max, gear, gears, km/h
    struct.pack_into('<ffffB', d, 128, 0.6, 0.0, 0.0, 0.1, 0)  # throttle, brake, clutch, steering
    struct.pack_into('<If', d, 145, 0x02 | 0x08 | 0x10, 321.5 / 400)  # driven, lights, ABS; fuel
    struct.pack_into('<I9B', d, 153, 0x08 | 0x40, 0, 1, 0, 2, 4 if race else 0, 0, 0, 1 if race else 0, 1)
    struct.pack_into('<ffff', d, 166, 92.0, 820.0, 296.0, 0.4)  # coolant, torque, kW, boost
    struct.pack_into('<ffff', d, 202, 321.5, 400.0, 12.0, 4.0)  # fuel, capacity, l/h, km/l
    struct.pack_into('<d', d, 222, 12345.6)  # odometer
    struct.pack_into('<ffff', d, 230, 50.0, 2400.0, 18000.0, 6000.0)  # limit, route m, kg, cargo
    if race:
        struct.pack_into('<ffffHHBBhh', d, 246, 95.0, 11.5, 83.456, 82.5, 2, 3, 2, 8, 4, 12)
    else:
        struct.pack_into('<h', d, 268, -1)
    struct.pack_into('<fff', d, 272, 14.5, 18.0, 0.0)  # time of day, air, rain
    struct.pack_into('<q', d, 284, 123456)  # money
    d[292] = 4
    for i, (x, y) in enumerate(((2.0, -1.0), (2.0, 1.0), (-3.0, -1.0), (-3.0, 1.0))):
        struct.pack_into('<11fBB', d, 293 + 46 * i, 30.0, 0.0, 0.0, 0.05, 0.55, 64.0 + i,
                         120.0 + i, 40000.0, 0.02 * i, x, y, 5, 1)
    name = 'Camion test'.encode()
    d[753:753 + len(name)] = name
    d[817:817 + 6] = b'Truck1'
    struct.pack_into('<ffB', d, 849, 26000.0, 8000.0, 1)  # with one trailer
    return bytes(d)


def forza(t):
    d = bytearray(311)
    struct.pack_into('<iIfff', d, 0, 1, int(t * 1000), 7000.0, 0.0, 3450.0)
    struct.pack_into('<4f', d, 68, 0.4, 0.4, 0.5, 0.5)
    struct.pack_into('<fff', d, 244, 48.0 / 3.6, 296000.0, 820.0)
    struct.pack_into('<4f', d, 256, 147.2, 149.0, 150.8, 152.6)
    struct.pack_into('<ff', d, 272, 5.8, 0.8)
    d[303:308] = bytes((153, 0, 0, 0, 3))
    return bytes(d)


s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
t0 = time.monotonic()
print(f'envoi {args.port}', flush=True)
while True:
    t = time.monotonic() - t0
    s.sendto(forza(t) if args.forza else native(t, t > args.race), ('127.0.0.1', args.port))
    time.sleep(1 / 60)
