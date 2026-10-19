"""wp82 item 5: replay `TheEndGatewayBlockEntity.findValidSpawnInChunk` on both servers' terrain: every end stone block (y 30..60) of the chunk, then the
candidate nearest the origin. usage: spawn82.py CX CZ"""
import threading, json
from lab import *
CX, CZ = int(sys.argv[1]), int(sys.argv[2])
E = "execute in minecraft:the_end run "
stone = {"K": set(), "V": set()}
def run(t):
    for z in range(CZ * 16, CZ * 16 + 16):
        for x in range(CX * 16, CX * 16 + 16):
            # one fill-count per column and y range is not enough (we need the y): test y by y
            n = count_blocks(t, x, 30, z, x, 60, z, "minecraft:end_stone", dim="the_end")
            if n == 0:
                continue
            for y in range(30, 61):
                if count_blocks(t, x, y, z, x, y, z, "minecraft:end_stone", dim="the_end"):
                    stone[t].add((x, y, z))
ths = [threading.Thread(target=run, args=(t,)) for t in "KV"]
[th.start() for th in ths]; [th.join() for th in ths]
json.dump({t: sorted(v) for t, v in stone.items()}, open("/tmp/spawn82.json", "w"))
for t in "KV":
    s = stone[t]
    cands = [(x, y, z) for (x, y, z) in s if (x, y + 1, z) not in s and (x, y + 2, z) not in s]
    best = min(cands, key=lambda p: (p[0] + .5) ** 2 + (p[1] + .5) ** 2 + (p[2] + .5) ** 2) if cands else None
    F(t, "end stone blocks", len(s), "candidates", len(cands), "nearest", best)
F("same terrain:", stone["K"] == stone["V"], len(stone["K"] ^ stone["V"]))
