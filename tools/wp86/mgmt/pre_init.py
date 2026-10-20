#!/usr/bin/env python3
"""Connects to a management server while it is still starting and records what it says (wp86).

    pre_init.py PORT SECRET SECONDS

Retries the connection until the port answers, asks three things at once, then prints every message for
SECONDS seconds, with the time (to the second) since the connection; run it right after starting a server
that has `status-heartbeat-interval=1`.
"""
import sys, time, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from wsrpc import Ws

port, secret, seconds = int(sys.argv[1]), sys.argv[2], float(sys.argv[3])
ws = None
end = time.time() + 60
while ws is None and time.time() < end:
    try:
        ws = Ws("127.0.0.1", port, secret)
    except OSError:
        time.sleep(0.02)
t0 = time.time()
print("connected")
for i, m in enumerate(["minecraft:allowlist", "rpc.discover", "minecraft:server/status"], 1):
    ws.send_json({"jsonrpc": "2.0", "id": i, "method": m})
until = t0 + seconds
while time.time() < until:
    try:
        t = ws.recv_text(max(0.05, until - time.time()))
    except EOFError:
        print("closed")
        break
    if t is None:
        continue
    short = t if len(t) < 300 else t[:120] + "...(%d bytes)" % len(t)
    print("%ds" % int(time.time() - t0), short)
