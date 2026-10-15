"""wp65: a mob tower (dark spawning platform with four water lanes into a central drop shaft, a fall that kills, hoppers into a chest),
built with commands, run at tick rate 100 for a while, then the chest is counted: Kiln (TARGET=K) vs vanilla (TARGET=V).
usage: p65_tower.py SECONDS [X0 Z0]"""
from p65_b2 import *
import sys, os

secs = int(sys.argv[1]) if len(sys.argv) > 1 else 120
X0 = int(sys.argv[2]) if len(sys.argv) > 2 else 3000
Z0 = int(sys.argv[3]) if len(sys.argv) > 3 else 3000
Y = 200
CX, CZ = X0 + 8, Z0 + 8


def build():
    sv(f"forceload add {X0-12} {Z0-12} {X0+30} {Z0+60}", show=False)
    time.sleep(3)
    for _ in range(40):
        out = sv(f"setblock {X0} {Y} {Z0} minecraft:stone", show=False, wait=0.8)
        out += sv(f"setblock {X0+30} {Y} {Z0+60} minecraft:stone", show=False, wait=0.8)
        if "unloaded" not in out and "not loaded" not in out:
            break
        time.sleep(1.5)
    sv(f"fill {X0-2} {Y-34} {Z0-2} {X0+18} {Y+8} {Z0+18} minecraft:air", show=False)
    # the platform, walls (3 high) and the roof
    sv(f"fill {X0-1} {Y} {Z0-1} {X0+17} {Y} {Z0+17} minecraft:stone", show=False)
    sv(f"fill {X0-1} {Y+1} {Z0-1} {X0+17} {Y+3} {Z0+17} minecraft:stone", show=False)
    sv(f"fill {X0} {Y+1} {Z0} {X0+16} {Y+3} {Z0+16} minecraft:air", show=False)
    sv(f"fill {X0-1} {Y+4} {Z0-1} {X0+17} {Y+4} {Z0+17} minecraft:glass", show=False)
    # the lanes: water sources at the far ends of the four lanes, the hole in the middle
    for (sx, sz) in ((X0, CZ), (X0 + 16, CZ), (CX, Z0), (CX, Z0 + 16)):
        sv(f"setblock {sx} {Y+1} {sz} minecraft:water", show=False)
    sv(f"setblock {CX} {Y} {CZ} minecraft:air", show=False)
    if os.environ.get("SPAWNERS"):
        sv(f"fill {X0-1} {Y+4} {Z0-1} {X0+17} {Y+4} {Z0+17} minecraft:stone", show=False)
        for sx in (CX - 4, CX + 4):
            for sz in (CZ - 4, CZ + 4):
                sv(f"setblock {sx} {Y+1} {sz} minecraft:spawner{{MinSpawnDelay:20s,MaxSpawnDelay:40s,SpawnCount:4s,SpawnRange:5s,RequiredPlayerRange:64s,MaxNearbyEntities:8s,SpawnData:{{entity:{{id:\"minecraft:zombie\"}}}}}}", show=False)
    # the shaft: 1x1 down 30 blocks, then a collection of hoppers into a chest
    sv(f"fill {CX} {Y-1} {CZ} {CX} {Y-30} {CZ} minecraft:air", show=False)
    sv(f"fill {CX-1} {Y-1} {CZ-1} {CX+1} {Y-30} {CZ+1} minecraft:stone", show=False)
    sv(f"fill {CX} {Y-1} {CZ} {CX} {Y-30} {CZ} minecraft:air", show=False)
    sv(f"setblock {CX} {Y-31} {CZ} minecraft:hopper[facing=down]", show=False)
    sv(f"setblock {CX} {Y-32} {CZ} minecraft:chest", show=False)
    sv(f"fill {CX-1} {Y-33} {CZ-1} {CX+1} {Y-31} {CZ+1} minecraft:stone replace minecraft:air", show=False)
    sv(f"setblock {CX} {Y-31} {CZ} minecraft:hopper[facing=down]", show=False)
    sv(f"setblock {CX} {Y-32} {CZ} minecraft:chest", show=False)


def chest_items():
    out = sv(f"data get block {CX} {Y-32} {CZ} Items", wait=1.0, show=False)
    totals = {}
    for comp in re.findall(r"\{[^{}]*\}", out):
        c = re.search(r"count: (\d+)", comp)
        i = re.search(r'id: "minecraft:([a-z_]+)"', comp)
        if c and i:
            totals[i.group(1)] = totals.get(i.group(1), 0) + int(c.group(1))
    for m in re.finditer(r'id: "minecraft:([a-z_]+)"[^}]*?count: (\d+)', out):
        pass
    return totals, out


def tower():
    prep("creative")
    sv("gamerule spawn_mobs true", show=False)
    sv("difficulty normal", show=False)
    sv("kill @e[type=!player]", show=False)
    sv("time set midnight", show=False)
    sv("gamerule advance_time false", show=False)
    build()
    # the players hang 36 blocks away (over a stone slab), well in range for spawning
    sv(f"fill {CX+4} 171 {CZ-2} {CX+9} 171 {CZ+2} minecraft:stone", show=False)
    sv(f"fill {CX+4} 172 {CZ-2} {CX+9} 175 {CZ+2} minecraft:air", show=False)
    for nm in ("KilnTest", "Obs"):
        sv(f"gamemode creative {nm}", show=False)
        sv(f"tp {nm} {CX + 6.5} 172 {CZ + 0.5}", show=False)
    time.sleep(8)
    F("A at", pos(a), "tower shaft blocks", a.block(CX, Y - 10, CZ))
    t0 = time.time()
    sv("tick rate 100", show=False)
    # periodic cleaning of the mobs that are not in the tower (they would fill the mob cap)
    log = []
    spawned = []
    while time.time() - t0 < secs:
        time.sleep(30)
        it, _ = chest_items()
        k = sv(f"kill @e[type=!player,type=!item,type=!experience_orb,x={X0},y={Y},z={Z0},dx=17,dy=5,dz=17]", show=False, wait=0.5)
        m = re.search(r"Killed (d+) entit", k) or re.search(r"success.multiple[(d+)]", k)
        spawned.append(int(m.group(1)) if m else (1 if ("kill.success.single" in k or "Killed " in k) else 0))
        log.append((round(time.time() - t0), sum(it.values())))
        sv(f"execute as @e[type=!player,type=!item,type=!experience_orb] unless entity @s[x={X0-1},y={Y-35},z={Z0-1},dx=19,dy=45,dz=19] run kill @s", show=False)
        F("t+%d s: chest items %d" % (time.time() - t0, sum(it.values())), "|", mspt()[:90])
    sv("tick rate 20", show=False)
    it, raw = chest_items()
    F("mobs found on the platform per sample:", spawned, "total", sum(spawned))
    F("chest", it)
    mobs = sv(f"execute as @e[type=!player,type=!item,type=!experience_orb,x={X0},y={Y},z={Z0},dx=17,dy=5,dz=17] run data get entity @s Pos", show=False)
    F("mobs alive on the platform now", len([l for l in mobs.splitlines() if "entity data" in l or "query" in l]))
    shot("tower")


if __name__ == "__main__":
    tower()
