"""wp65: an iron golem farm: six villagers in 1x1 glass cells (they slept recently: `last_slept` set by NBT), a zombie in a cell next to them (the scare),
a floor around; how many golems appear in N minutes at a fast tick rate. Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p65_b2 import *
import sys, os

secs = int(sys.argv[1]) if len(sys.argv) > 1 else 90
X0, Z0, Y = int(os.environ.get("FX") or "8000"), int(os.environ.get("FZ") or "8000"), 200


def build():
    sv(f"forceload add {X0-20} {Z0-20} {X0+20} {Z0+20}", show=False)
    time.sleep(3)
    for _ in range(40):
        out = sv(f"setblock {X0} {Y} {Z0} minecraft:stone", show=False, wait=0.8)
        out += sv(f"setblock {X0+19} {Y} {Z0+19} minecraft:stone", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv(f"fill {X0-20} {Y-1} {Z0-20} {X0+20} {Y+12} {Z0+20} minecraft:air", show=False)
    sv(f"fill {X0-20} {Y} {Z0-20} {X0+20} {Y} {Z0+20} minecraft:stone", show=False)
    # a ring of cells: 6 villager cells (glass walls, roof) and one zombie cell, around (X0, Z0)
    cells = [(2, 0), (-2, 0), (0, 2), (0, -2), (2, 2), (-2, -2)]
    for (dx, dz) in cells:
        x, z = X0 + dx, Z0 + dz
        sv(f"fill {x-1} {Y+1} {z-1} {x+1} {Y+3} {z+1} minecraft:iron_bars", show=False)
        sv(f"fill {x} {Y+1} {z} {x} {Y+2} {z} minecraft:air", show=False)
    # the open space between the cells and around is the floor golems spawn on (air above, no roof)
    now = sv("time query gametime", show=False)
    m = re.search(r"time is ([0-9]+)", now) or re.search(r"gametime\[([0-9]+)\]", now)
    g = int(m.group(1)) if m else 0
    for (dx, dz) in cells:
        sv(f"summon minecraft:villager {X0 + dx}.5 {Y+1} {Z0 + dz}.5 {{Brain:{{memories:{{\"minecraft:last_slept\":{{value:{g}L}}}}}},PersistenceRequired:1b,VillagerData:{{profession:\"minecraft:none\",level:1,type:\"minecraft:plains\"}}}}", show=False)
    sv(f"fill {X0+5} {Y+1} {Z0-1} {X0+7} {Y+3} {Z0+1} minecraft:iron_bars", show=False)
    sv(f"fill {X0+6} {Y+1} {Z0} {X0+6} {Y+2} {Z0} minecraft:air", show=False)
    sv(f"summon minecraft:zombie {X0+6}.5 {Y+1} {Z0}.5 {{NoAI:1b,PersistenceRequired:1b,Silent:1b}}", show=False)


def golems():
    out = sv(f"execute as @e[type=minecraft:iron_golem,x={X0-20},y={Y-5},z={Z0-20},dx=40,dy=30,dz=40] run data get entity @s Pos", show=False, wait=2.0)
    return len(re.findall(r"\[-?[0-9.]+d, -?[0-9.]+d, -?[0-9.]+d\]", out))


def iron():
    prep("creative")
    sv("gamerule spawn_mobs false", show=False)
    sv("difficulty normal", show=False)
    sv("time set day", show=False)
    sv("kill @e[type=!player]", show=False)
    build()
    for nm in ("KilnTest", "Obs"):
        sv(f"gamemode creative {nm}", show=False)
        sv(f"tp {nm} {X0 + 10}.5 {Y + 1} {Z0 + 10}.5", show=False)
    time.sleep(6)
    vs = [e for e in a.state(40)["entities"] if e["t"] == "villager"]
    F("villagers seen", len(vs), "zombies", len([e for e in a.state(40)["entities"] if e["t"] == "zombie"]))
    sv("tick rate 100", show=False)
    t0 = time.time()
    while time.time() - t0 < secs:
        for _ in range(7):
            time.sleep(3)
            sv(f"damage @e[type=minecraft:villager,x={X0-5},y={Y},z={Z0-5},dx=10,dy=5,dz=10] 0.1 minecraft:player_attack by KilnTest", show=False, wait=0.2)
        F("t+%d golems %d" % (time.time() - t0, golems()))
    sv("tick rate 20", show=False)
    mem = sv(f"execute as @e[type=minecraft:villager,x={X0-5},y={Y},z={Z0-5},dx=10,dy=5,dz=10,limit=1] run data get entity @s Brain.memories", show=False, wait=1.5)
    F("a villager's memories", mem.strip()[-400:])
    shot("iron")


if __name__ == "__main__":
    iron()
