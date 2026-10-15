"""wp65: natural spawns around a given spot, Kiln (TARGET=K) vs vanilla (TARGET=V): the players stand at the spot (creative, so nothing hurts them),
the server runs fast (tick rate 100) for N seconds, then the entities of each type are counted with /kill ("Killed N entities").
usage: p65_spawn.py NAME DIM X Y Z [SAMPLES [SECONDS]]   (types listed in TYPES)"""
from p65 import *
import sys, os

name, dim_, X, Y, Z = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], sys.argv[5]
samples = int(sys.argv[6]) if len(sys.argv) > 6 else 4
secs = int(sys.argv[7]) if len(sys.argv) > 7 else 30
TYPES = os.environ.get("TYPES", "blaze,wither_skeleton,zombified_piglin,magma_cube,ghast,piglin,piglin_brute,hoglin,strider,skeleton,enderman,zoglin").split(",")
a, b = clients()
if T == "V":
    sv("op KilnTest", show=False)
    sv("op Obs", show=False)
alive(a)
alive(b)
sv("gamerule spawn_mobs true", show=False)
sv("difficulty normal", show=False)
sv("kill @e[type=!player]", show=False)
if os.environ.get("TIME"):
    sv("time set " + os.environ["TIME"], show=False)
    sv("gamerule advance_time false", show=False)
for p in ("KilnTest", "Obs"):
    sv(f"gamemode creative {p}", show=False)
    sv(f"execute in minecraft:{dim_} run tp {p} {X} {Y} {Z}", show=False)
time.sleep(10)
print("A", a.state(0)["dimension"], [round(a.state(0)[k], 1) for k in ("x", "y", "z")], flush=True)


def killed(t):
    out = sv(f"kill @e[type=minecraft:{t}]", show=False, wait=0.4)
    m = re.search(r"Killed (\d+) entit", out)
    if m:
        return int(m.group(1))
    m = re.search(r"kill\.success\.multiple\[(\d+)\]", out)
    if m:
        return int(m.group(1))
    return 1 if "kill.success.single" in out or "Killed " in out else 0


for t in TYPES:
    killed(t)
tot = {t: 0 for t in TYPES}
sv("tick rate 100", show=False)
for i in range(samples):
    time.sleep(secs)
    row = {}
    for t in TYPES:
        row[t] = killed(t)
        tot[t] += row[t]
    print("sample", i, {k: v for k, v in row.items() if v}, "|", mspt() if T == "K" else "", flush=True)
sv("tick rate 20", show=False)
print("TOTAL", name, T, {k: v for k, v in tot.items() if v}, flush=True)
json.dump(tot, open(OUT / f"spawn_{name}_{T}.json", "w"))
