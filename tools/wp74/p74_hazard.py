"""wp74: a hazard matrix, Kiln against vanilla: every mob (NoAI, persistent) in its own cell with one hazard at its feet; health / death after N ticks.
usage: p74_hazard.py [HAZARDS comma list] [TICKS]   hazards: lava fire magma_block cactus sweet_berry_bush campfire powder_snow wither_rose sun cobweb"""
from lock import *
import threading, json, sys

MOBS = """allay armadillo axolotl bat bee blaze bogged breeze camel camel_husk cat cave_spider chicken cod copper_golem cow creaking creeper dolphin donkey drowned elder_guardian
enderman endermite evoker fox frog ghast giant glow_squid goat guardian happy_ghast hoglin horse husk illusioner iron_golem llama magma_cube mooshroom mule nautilus ocelot panda
parched parrot phantom pig piglin piglin_brute pillager polar_bear pufferfish rabbit ravager salmon sheep shulker silverfish skeleton slime sniffer snow_golem spider squid
stray strider tadpole trader_llama tropical_fish turtle vex villager vindicator wandering_trader warden witch wither wither_skeleton wolf zombie zombie_nautilus zombie_villager zombified_piglin""".split()
HAZ = (sys.argv[1] if len(sys.argv) > 1 else "lava,fire,magma_block,cactus,sweet_berry_bush,campfire,powder_snow,wither_rose,sun,cobweb").split(",")
TICKS = int(sys.argv[2]) if len(sys.argv) > 2 else 200
X0, Y0, Z0 = 100, 200, 100


def setup_cell(t, x, z, haz):
    w(t, f"fill {x} {Y0 - 1} {z} {x + 3} {Y0 - 1} {z + 3} minecraft:stone")
    w(t, f"fill {x} {Y0} {z} {x + 3} {Y0 + 4} {z + 3} minecraft:air")
    if haz == "sun":
        return
    blk = {"lava": "lava", "fire": "fire", "magma_block": "magma_block", "cactus": "cactus", "sweet_berry_bush": "sweet_berry_bush[age=3]", "campfire": "campfire[lit=true]",
           "powder_snow": "powder_snow", "wither_rose": "wither_rose", "cobweb": "cobweb"}[haz]
    if haz == "magma_block":
        w(t, f"setblock {x + 1} {Y0 - 1} {z + 1} minecraft:magma_block")
    else:
        w(t, f"setblock {x + 1} {Y0} {z + 1} minecraft:{blk}")


def go(t, out):
    for g in ("spawn_mobs false", "advance_time false", "advance_weather false", "mob_griefing false"):
        w(t, "gamerule " + g)
    w(t, "difficulty normal"); w(t, "gamemode creative KilnTest"); w(t, "time set 6000"); w(t, "weather clear")
    w(t, f"forceload add {X0 - 4} {Z0 - 4} {X0 + 60} {Z0 + 60}")
    w(t, f"execute in minecraft:overworld run tp KilnTest {X0 + 30} {Y0 + 30} {Z0 + 30}")
    w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
    wait_loaded(t, X0 + 60, Y0, Z0 + 60); wait_loaded(t, X0 - 2, Y0, Z0 - 2)
    time.sleep(2)
    out[t] = {}
    for haz in HAZ:
        w(t, "kill @e[type=!player]")
        w(t, f"fill {X0 - 2} {Y0 - 3} {Z0 - 2} {X0 + 60} {Y0 + 8} {Z0 + 60} minecraft:air")
        time.sleep(0.5)
        cells = {}
        for i, m in enumerate(MOBS):
            x, z = X0 + 4 * (i % 12), Z0 + 4 * (i // 12)
            cells[m] = (x, z)
            setup_cell(t, x, z, haz)
        time.sleep(1)
        for m, (x, z) in cells.items():
            w(t, f"summon minecraft:{m} {x + 1}.5 {Y0} {z + 1}.5 {{NoAI:1b,PersistenceRequired:1b,Tags:[\"h_{m}\"]}}")
        time.sleep(1)
        sprint(t, TICKS)
        w(t, "tick freeze")
        time.sleep(0.5)
        res = {}
        for m in MOBS:
            alive = count(t, f"@e[tag=h_{m}]")
            if not alive:
                res[m] = "dead/gone"
                continue
            hp = ask(t, f"data get entity @e[tag=h_{m},limit=1] Health", wait=0.12)
            mm = re.findall(r"(-?\d+\.\d+)", hp)
            fz = ask(t, f"data get entity @e[tag=h_{m},limit=1] Fire", wait=0.12)
            fire = re.findall(r"(-?\d+)s?\b", fz[-30:])
            res[m] = "hp " + (mm[-1] if mm else "?")
        out[t][haz] = res
        w(t, "tick unfreeze")
        print(t, haz, "done", flush=True)


out = {}
th = [threading.Thread(target=go, args=(t, out)) for t in "KV"]
[x.start() for x in th]; [x.join() for x in th]
diff = {}
for haz in HAZ:
    d = {m: (out["K"][haz][m], out["V"][haz][m]) for m in MOBS if out["K"][haz][m] != out["V"][haz][m]}
    if d:
        diff[haz] = d
print("DIFF", json.dumps(diff))
print("RESULT", json.dumps(out))
