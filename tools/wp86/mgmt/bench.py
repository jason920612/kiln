#!/usr/bin/env python3
"""Round-trip time of management requests (wp86): bench.py PORT SECRET [N]

Sends N (default 2000) requests one after the other on one connection, for a getter that does not
touch the game (`serversettings/motd`), one that asks the game thread (`players` is answered from the
roster; `serversettings/autosave` asks the game) and a setter, and prints requests per second and
the median and 99th percentile in microseconds.
"""
import sys, time, os, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from wsrpc import Ws

port, secret = int(sys.argv[1]), sys.argv[2]
n = int(sys.argv[3]) if len(sys.argv) > 3 else 2000
ws = Ws("127.0.0.1", port, secret)
cases = [
    ("serversettings/motd (properties)", "minecraft:serversettings/motd", None),
    ("players (roster)", "minecraft:players", None),
    ("serversettings/autosave (game thread)", "minecraft:serversettings/autosave", None),
    ("serversettings/motd/set (properties file)", "minecraft:serversettings/motd/set", ["x"]),
]
for name, method, params in cases:
    times = []
    t_all = time.perf_counter()
    for i in range(n):
        t0 = time.perf_counter()
        ws.call(method, params, id=i + 1)
        times.append((time.perf_counter() - t0) * 1e6)
    total = time.perf_counter() - t_all
    times.sort()
    print(f"{name:45s} {n / total:8.0f} req/s   p50 {times[n // 2]:7.0f} us   p99 {times[int(n * 0.99)]:7.0f} us")
