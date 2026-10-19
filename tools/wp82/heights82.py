"""wp82 item 5: the top (world surface) of every column of a square of the End on Kiln and vanilla, to replay `findTallestBlock`. usage: heights82.py X0 X1 Z0 Z1"""
import threading, json
from lab import *
X0, X1, Z0, Z1 = (int(a) for a in sys.argv[1:5])
E = "execute in minecraft:the_end run "
out = {"K": {}, "V": {}}
def run(t):
    w(t, E + f"forceload add {X0} {Z0} {X1} {Z1}")
    time.sleep(8)
    w(t, E + "kill @e[tag=probe82]")
    w(t, E + f"summon minecraft:marker {X0} 200 {Z0} {{Tags:[\"probe82\"]}}")
    time.sleep(1)
    for x in range(X0, X1 + 1):
        for z in range(Z0, Z1 + 1):
            w(t, E + f"execute positioned {x}.5 0 {z}.5 positioned over world_surface run tp @e[tag=probe82,limit=1] ~ ~ ~")
            o = ask(t, E + "data get entity @e[tag=probe82,limit=1] Pos", wait=0.12)
            m = re.search(r"\[(-?[\d.]+)d, (-?[\d.]+)d, (-?[\d.]+)d\]", o)
            out[t][f"{x},{z}"] = float(m.group(2)) if m else None
ths = [threading.Thread(target=run, args=(t,)) for t in "KV"]
[th.start() for th in ths]; [th.join() for th in ths]
json.dump(out, open("/tmp/heights82.json", "w"))
diff = [(k, out["K"][k], out["V"][k]) for k in out["K"] if out["K"][k] != out["V"][k]]
F("columns", len(out["K"]), "differing", len(diff), diff[:20])
