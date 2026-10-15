from p65_b2 import *
alive(b)
sv("kill @e[type=ghast]", show=False)
sv("execute in minecraft:the_nether run summon minecraft:ghast 800.5 106 818.5", show=False)
for i in range(6):
    time.sleep(1.0)
    es = [(e["t"], round(e["x"]), round(e["y"]), round(e["z"]), e.get("hp")) for e in a.state(80)["entities"] if e["t"] != "item"]
    print(i, es, "A hp", a.state(0)["health"], flush=True)
