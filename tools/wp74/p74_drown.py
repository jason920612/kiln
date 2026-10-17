"""wp74: which mobs drown? Every mob in its own water tank (NoAI), 700 ticks, Kiln against vanilla: survivors with their health."""
from lock import *
import threading, json
MOBS = """allay armadillo axolotl bat bee blaze bogged breeze camel camel_husk cat cave_spider chicken cod copper_golem cow creaking creeper dolphin donkey drowned elder_guardian
enderman endermite evoker fox frog ghast giant glow_squid goat guardian happy_ghast hoglin horse husk illusioner iron_golem llama magma_cube mooshroom mule nautilus ocelot panda
parched parrot phantom pig piglin piglin_brute pillager polar_bear pufferfish rabbit ravager salmon sheep shulker silverfish skeleton skeleton_horse slime sniffer snow_golem spider squid
stray strider tadpole trader_llama tropical_fish turtle vex villager vindicator wandering_trader warden witch wither wither_skeleton wolf zoglin zombie zombie_horse zombie_nautilus zombie_villager zombified_piglin""".split()
X0, Y0, Z0 = 100, 200, 100


def go(t, out):
    for g in ("spawn_mobs false", "advance_time false", "mob_griefing false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "gamemode creative KilnTest")
    w(t, f"forceload add {X0 - 4} {Z0 - 4} {X0 + 60} {Z0 + 60}")
    w(t, f"execute in minecraft:overworld run tp KilnTest {X0 + 30} {Y0 + 30} {Z0 + 30}")
    wait_loaded(t, X0 + 60, Y0, Z0 + 60); wait_loaded(t, X0 - 2, Y0, Z0 - 2)
    time.sleep(2)
    w(t, f"fill {X0 - 2} {Y0 - 6} {Z0 - 2} {X0 + 60} {Y0 + 8} {Z0 + 60} minecraft:air")
    w(t, "kill @e[type=!player]")
    cells = {}
    for i, m in enumerate(MOBS):
        x, z = X0 + 4 * (i % 12), Z0 + 4 * (i // 12)
        cells[m] = (x, z)
        w(t, f"fill {x} {Y0 - 6} {z} {x + 3} {Y0 + 4} {z + 3} minecraft:stone")
        w(t, f"fill {x + 1} {Y0 - 5} {z + 1} {x + 2} {Y0 + 3} {z + 2} minecraft:water")
    time.sleep(1)
    for m, (x, z) in cells.items():
        w(t, f"summon minecraft:{m} {x + 1}.5 {Y0 - 2} {z + 1}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"d_{m}\"]}}")
    time.sleep(1)
    start = {m: count(t, f"@e[tag=d_{m}]") for m in MOBS}
    sprint(t, 700)
    res = {}
    for m in MOBS:
        if not start[m]:
            res[m] = "not summoned"
            continue
        alive = count(t, f"@e[tag=d_{m}]")
        hp = ask(t, f"data get entity @e[tag=d_{m},limit=1] Health", wait=0.15) if alive else ""
        mm = re.findall(r"(-?\d+\.\d+)", hp)
        res[m] = ("alive hp " + mm[-1]) if alive and mm else ("dead" if not alive else "alive")
    out[t] = res


out = {}
th = [threading.Thread(target=go, args=(t, out)) for t in "KV"]
[x.start() for x in th]; [x.join() for x in th]
diff = {m: (out["K"][m], out["V"][m]) for m in MOBS if out["K"][m] != out["V"][m]}
print("DIFF", json.dumps(diff))
print("RESULT", json.dumps(out))
