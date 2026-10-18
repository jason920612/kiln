"""wp78: the End as both servers generate it: crystals (positions, cages = beam/bottom flags), dragon, bossbar, spikes; lock step census with a creative client in each."""
from lock import *
respawn_all()
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, "execute in minecraft:the_end run tp KilnTest 0 80 0"); w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
time.sleep(25)
for c in ["execute if entity @e[type=end_crystal]", "execute if entity @e[type=ender_dragon]", "data get entity @e[type=ender_dragon,limit=1] Health", "data get entity @e[type=ender_dragon,limit=1] Pos", "data get entity @e[type=ender_dragon,limit=1] DragonPhase"]:
    r = ask_both("execute in minecraft:the_end run " + c if c.startswith("execute") else c, wait=1.0)
    print(c, {t: r[t].strip().replace(chr(10), " ")[-110:] for t in "KV"})
res = {}
for t in "KV":
    out = []
    cl = C(25999 if t == "K" else 26000)
    es = [e for e in cl.state(150)["entities"] if e["t"] == "end_crystal"]
    res[t] = sorted((round(e["x"] - 0.5), round(e["y"]), round(e["z"] - 0.5)) for e in es)
print("crystals K", res["K"])
print("crystals V", res["V"])
print("same crystal positions:", res["K"] == res["V"])
for t in "KV":
    cl = C(25999 if t == "K" else 26000)
    bars = cl.findbox("iron_bars", -50, 60, -50, 50, 125, 50, 400)
    print(t, "iron_bars blocks:", len(bars), "bossbars", cl.cmd("bossbars")[:160])
