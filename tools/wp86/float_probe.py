"""Hovers a survival player in the air and prints when and why the server kicks (allow-flight).

usage: python3 float_probe.py <host> <port> [y offset] [seconds]
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcclient as m

host, port = sys.argv[1], int(sys.argv[2])
dy = float(sys.argv[3]) if len(sys.argv) > 3 else 10.0
seconds = float(sys.argv[4]) if len(sys.argv) > 4 else 8.0
c = m.Client(host, port, "Floater").login()
res = c.hover(seconds, y_offset=dy)
if not res:
    print(f"hovered {seconds}s at +{dy}: not kicked")
for t, pid, body in res:
    print(f"kicked after about {t}s: {body.hex()}")
