"""wp72: K-only (or V with TARGET=V) console timing probe: lamp off delay in steps, at a given x z (no client needed).
usage: p72_kt.py X Z"""
import sys, time, re
import p61
from p61 import *
from ctl import PLAY
p61.VDIR = HOME / "wp72" / "vanilla"
T = os.environ.get("TARGET", "K")
LOGP = (PLAY if T == "K" else p61.VDIR) / "server.log"
CON = (PLAY if T == "K" else p61.VDIR) / "console"
def w(cmd):
    with open(CON, "w") as f:
        f.write(cmd + "\n")
def logsize():
    return LOGP.stat().st_size
def since(n):
    with open(LOGP, "rb") as f:
        f.seek(n)
        return re.sub(r"\x1b\[[0-9;]*m", "", f.read().decode(errors="replace"))
from ctl import C
ca = C(25999 if T == 'K' else 26000)
x, z = int(sys.argv[1]), int(sys.argv[2])
y = 200
w(f"forceload add {x} {z}"); time.sleep(1.5)
w("gamerule random_tick_speed 0")
w("tick freeze"); time.sleep(0.3)
w(f"fill {x-2} {y-1} {z-2} {x+4} {y+3} {z+2} minecraft:air")
w(f"fill {x-2} {y-1} {z-2} {x+4} {y-1} {z+2} minecraft:stone")
w("tick step 30"); time.sleep(2)
w(f"setblock {x} {y} {z} minecraft:redstone_lamp"); w(f"setblock {x+1} {y} {z} minecraft:redstone_block"); time.sleep(0.3)
w("tick step 30"); time.sleep(2)
w(f"setblock {x+1} {y} {z} minecraft:air"); time.sleep(0.5)
res = None
for k in range(1, 9):
    n = logsize()
    w("tick step 1"); time.sleep(0.3)
    w(f"execute if block {x} {y} {z} minecraft:redstone_lamp[lit=false] run say DARK"); time.sleep(0.3)
    out = since(n)
    cl = ca.cmd(f"block {x} {y} {z}")
    print("  step", k, "server dark" if "DARK" in out else "server lit ", "| client:", cl[-30:])
    if "DARK" in out and res is None:
        res = k
print(T, "lamp dark at step", res)
w("tick unfreeze")
