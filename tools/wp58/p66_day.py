"""wp66 item 2, second half: after the victory (the hero effect is still on) it is made day again and the player stays with the villagers: the gifts
they throw to the hero (the inventory changes; the item entities of the level), and the trade prices with and without the hero effect.
usage: TARGET=K|V [ACTOR=.. AGENT_PORT=.. SMALL=1] python3 p66_day.py OUT.json"""
import sys
from p66_raid import *

if __name__ == "__main__":
    outp = sys.argv[1]
    a = client(PORT)
    try:
        S("gamerule advance_time false", "tick rate 20", "time set 1000", f"effect give {NAME} minecraft:hero_of_the_village 48000 1 true")
        vs = sorted([e for e in a.state(60)["entities"] if e["t"] == "villager"], key=lambda e: e["y"])
        if os.environ.get("AT") == "bell":
            S(f"tp {NAME} 1463.5 112 -1040.5")
        elif vs:
            S(f"tp {NAME} {vs[0]['x']-2:.2f} {vs[0]['y']:.2f} {vs[0]['z']:.2f}")
        else:
            S(f"tp {NAME} {VX}.5 112 {VZ+3}.5")
        time.sleep(4)
        F("effects", effects(a), "villagers the client sees:", [(round(e["x"]), round(e["y"]), round(e["z"])) for e in a.state(60)["entities"] if e["t"] == "villager"])
        inv0 = inventory(a)
        t0 = time.time()
        while time.time() - t0 < int(os.environ.get("DAY_S", "150")):
            time.sleep(10)
            cur = inventory(a)
            ch = {k: cur.get(k, 0) - inv0.get(k, 0) for k in set(inv0) | set(cur) if cur.get(k, 0) != inv0.get(k, 0)}
            out = S(f"execute as @e[type=minecraft:item,{BOX}] run data get entity @s Item")
            items = [m.group(1) for l in out for m in [re.search(r'id: "minecraft:([a-z_]+)"', l)] if m]
            near = S(f"execute as {NAME} at @s as @e[type=minecraft:villager,distance=..16] run data get entity @s Health")
            F("day", round(time.time() - t0), "s: inventory changes", ch, "items on the ground", items, "villagers near", len([l for l in near if "query[" in l or "entity data" in l]))
            if ch:
                LOG["gifts"] = ch
        try:
            trade_prices(a)
        except Exception as ex:
            F("trade check failed:", ex)
    finally:
        json.dump(LOG, open(outp, "w"), indent=1)
        F("log written", outp)
