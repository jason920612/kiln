"""wp74: mob farms on identical worlds, Kiln against vanilla. One thread per server runs the same scenario (same seed, same commands,
`tick sprint` rounds); the mobs inside the farm's box are counted and killed after every round (a killed slime splits: the children are
killed in two more passes and not counted).
usage: p72_farm.py SCENARIO [ROUNDS] [ROUND_TICKS]   SCENARIO: slime | slime_control | witch | outpost | monument | creeper | gold | enderman"""
import re, sys, threading, json, math
from lock import *

SEED = int(os.environ.get("FARM_SEED", "72"))
ROOM_H = int(os.environ.get("ROOM_H", "3"))  # height of the rooms of the slime farm
ME = "KilnTest"


def is_slime_chunk(seed, cx, cz):
    """WorldgenRandom.seedSlimeChunk(cx, cz, seed, 987234911).nextInt(10) == 0 with Java int wrap-around."""
    def i32(v):
        v &= 0xFFFFFFFF
        return v - (1 << 32) if v & 0x80000000 else v
    s = seed + i32(i32(cx * cx) * 4987142) + i32(cx * 5947611) + i32(cz * cz) * 4392871 + i32(cz * 389711)
    s ^= 987234911
    # java.util.Random
    s = (s ^ 0x5DEECE66D) & ((1 << 48) - 1)
    bound = 10
    while True:
        s = (s * 0x5DEECE66D + 0xB) & ((1 << 48) - 1)
        bits = s >> (48 - 31)
        val = bits % bound
        if bits - val + (bound - 1) < (1 << 31):
            return val == 0


def sprint(t, n):
    pos = logpos(t)
    w(t, f"tick sprint {n}")
    t0 = time.time()
    while time.time() - t0 < 3000:
        time.sleep(1)
        txt = since(t, pos)
        if "sprint.report" in txt or "Sprint completed" in txt:
            return
    raise RuntimeError("sprint did not finish on " + t)


def kill_count(t, selector):
    out = ask(t, f"kill {selector}", wait=0.2)
    m = re.search(r"success\.multiple\[(\d+)\]|Killed (\d+) entities", out)
    if m:
        return int(m.group(1) or m.group(2))
    if "success.single" in out or re.search(r"Killed [A-Za-z]", out):
        return 1
    return 0


def locate(t, what, dim="overworld"):
    kind = "structure" if what.startswith("minecraft:") and not what.startswith("minecraft:biome") else "biome"
    out = ask(t, f"execute in minecraft:{dim} positioned 0 64 0 run locate {kind} {what}", wait=8)
    m = re.search(r"\[(-?\d+), (-?\d+|~), (-?\d+)\]", out)
    return (int(m.group(1)), int(m.group(3))) if m else None


class Farm:
    def __init__(self, t, name, rounds, round_ticks):
        self.t, self.name, self.rounds, self.round_ticks = t, name, rounds, round_ticks
        self.counts = []  # per round {type: n}

    def cmd(self, c, wait=0.15):
        return ask(self.t, c, wait=wait)

    def prep(self, dim="overworld", time_of_day=18000, extra=()):
        t = self.t
        if t == "V":
            w(t, f"op {ME}")
        for g in ("spawn_mobs true", "spawn_monsters true", "do_daylight_cycle false", "do_weather_cycle false", "spawn_phantoms false", "mob_griefing false", "random_tick_speed 3", "spawn_wardens false"):
            w(t, "gamerule " + g)
        w(t, "difficulty normal")
        w(t, "weather clear")
        w(t, f"time set {time_of_day}")
        w(t, f"gamemode creative {ME}")
        w(t, f"effect give {ME} minecraft:resistance 99999 4 true")
        for c in extra:
            w(t, c)
        time.sleep(0.5)

    def tp(self, dim, x, y, z):
        w(self.t, f"execute in minecraft:{dim} run tp {ME} {x} {y} {z}")
        time.sleep(1.0)

    def wait_chunks(self, secs=20):
        time.sleep(secs)

    def box(self, types, x0, y0, z0, x1, y1, z1):
        """Selector text for the mobs of `types` in the box."""
        return [f"@e[type=minecraft:{ty},x={min(x0, x1)},y={min(y0, y1)},z={min(z0, z1)},dx={abs(x1 - x0)},dy={abs(y1 - y0)},dz={abs(z1 - z0)}]" for ty in types]

    def measure(self, types, box):
        sels = self.box(types, *box)
        counts = {}
        for ty, sel in zip(types, sels):
            counts[ty] = kill_count(self.t, sel)
            # children of what was just killed (slimes, magma cubes) are dead on the next ticks
        return counts

    def run(self, types, box, clear_all=True):
        for r in range(self.rounds):
            sprint(self.t, self.round_ticks)
            counts = self.measure(types, box)
            # splits: kill the children without counting them
            for _ in range(2):
                sprint(self.t, 2)
                self.measure(types, box)
            if clear_all:
                self.cmd(f"kill @e[type=!player,type=!item]", wait=0.2)
            self.counts.append(counts)
            F(self.t, self.name, "round", r, {k: v for k, v in counts.items() if v})
        total = {ty: sum(c.get(ty, 0) for c in self.counts) for ty in types}
        F(self.t, self.name, "TOTAL", json.dumps(total))
        return total


def F(*a):
    print(*a, flush=True)


# ------------------------------------------------------------------ scenarios

def scen_slime(f, control=False):
    """Sealed rooms (13 flat floors, 2 high, a floor every 3 blocks up to y 38) in NCH slime chunks around the origin, or, as a control, in
    chunks that are not slime chunks. The player stands 100 blocks above the middle. Every slime of y 0..40 is counted."""
    nch = int(os.environ.get("NCH", "9"))
    chunks = []
    for r in range(0, 8):
        for cx in range(-r, r + 1):
            for cz in range(-r, r + 1):
                if max(abs(cx), abs(cz)) == r and is_slime_chunk(SEED, cx, cz) != control and len(chunks) < nch and (abs(cx) > 1 or abs(cz) > 1):
                    chunks.append((cx, cz))
    F(f.t, "chunks", chunks, "slime chunks" if not control else "NOT slime chunks")
    f.prep(time_of_day=6000)
    f.tp("overworld", 8.5, 100, 8.5)
    time.sleep(0.5)
    f.cmd(f"effect give {ME} minecraft:slow_falling 99999 0 true")
    f.cmd("forceload add -112 -112 127 127")
    f.wait_chunks(40)
    for cx, cz in chunks:
        x0, z0 = cx * 16, cz * 16
        f.cmd(f"fill {x0 - 1} -1 {z0 - 1} {x0 + 16} 40 {z0 + 16} minecraft:stone")
        step_y = ROOM_H + 1
        for i in range(37 // step_y):
            f.cmd(f"fill {x0 + 1} {step_y * i + 1} {z0 + 1} {x0 + 14} {step_y * i + ROOM_H} {z0 + 14} minecraft:air")
    f.cmd(f"tp {ME} 8.5 120 8.5")
    f.cmd("kill @e[type=!player]")
    return f.run(["slime", "zombie", "skeleton", "creeper", "spider"], (-120, 0, -120, 240, 40, 240))


def scen_creeper(f):
    """Dark sealed 14x14x4 platforms at y 200 in a grid of NPLAT x NPLAT chunks, the player 60 blocks above the middle."""
    n = int(os.environ.get("NPLAT", "3"))
    bx = 288  # chunk 18
    mid = bx + 8 * n
    f.prep(time_of_day=18000)
    f.tp("overworld", mid + 0.5, 261, mid + 0.5)
    f.cmd(f"forceload add {bx} {bx} {bx + 16 * n - 1} {bx + 16 * n - 1}")
    f.wait_chunks(25 + 6 * n)
    f.cmd(f"fill {mid - 2} 259 {mid - 2} {mid + 2} 260 {mid + 2} minecraft:glass")  # the player stands high above the platforms
    for i in range(n):
        for k in range(n):
            x0, z0 = bx + 16 * i, bx + 16 * k
            f.cmd(f"fill {x0} 199 {z0} {x0 + 15} 205 {z0 + 15} minecraft:air")
            f.cmd(f"fill {x0} 199 {z0} {x0 + 15} 204 {z0 + 15} minecraft:stone outline")
            f.cmd(f"fill {x0 + 1} 200 {z0 + 1} {x0 + 14} 203 {z0 + 14} minecraft:air")
    f.tp("overworld", mid + 0.5, 261, mid + 0.5)
    f.cmd("kill @e[type=!player]")
    return f.run(["creeper", "zombie", "skeleton", "spider", "enderman", "witch", "slime", "cave_spider"], (bx, 200, bx, bx + 16 * n, 203, bx + 16 * n))


def locate_biome_xz(t, biome, dim):
    out = ask(t, f"execute in minecraft:{dim} positioned 0 64 0 run locate biome {biome}", wait=6)
    m = re.search(r"\[(-?\d+), (-?\d+|~), (-?\d+)\]", out)
    return int(m.group(1)), int(m.group(3))


def scen_structure(f, struct, types, dim="overworld", half=12, up=10, time_of_day=18000):
    f.prep(time_of_day=time_of_day)
    xz = locate(f.t, f"minecraft:{struct}", dim)
    F(f.t, struct, "located", xz)
    if xz is None:
        return None
    x, z = xz
    f.tp(dim, x + 0.5, 150 if dim == "overworld" else 100, z + 0.5)
    f.cmd(f"effect give {ME} minecraft:slow_falling 99999 0 true")
    f.wait_chunks(40)
    f.cmd("kill @e[type=!player]")
    box = tuple(int(v) for v in os.environ["BOX"].split(",")) if os.environ.get("BOX") else (x - half, -60 if dim == "overworld" else 0, z - half, x + half, 150, z + half)
    if os.environ.get("KILLBLOCK"):
        # a repeating command block that kills everything outside the box every tick (keeps the mob cap free for the farm's own spawns)
        bx0, by0, bz0, bx1, by1, bz1 = box
        f.cmd("gamerule command_block_output false")
        f.cmd("gamerule log_admin_commands false")
        f.cmd(f'execute in minecraft:{dim} run setblock {bx0 - 12} {by0 + 8} {bz0} minecraft:repeating_command_block{{auto:1b,TrackOutput:0b,Command:"execute as @e[type=!player] unless entity @s[x={bx0},y={by0},z={bz0},dx={bx1 - bx0},dy={by1 - by0},dz={bz1 - bz0}] run kill @s"}}')
        F(f.t, "kill block", f.cmd(f"execute in minecraft:{dim} run data get block {bx0 - 12} {by0 + 8} {bz0} auto")[-60:])
    return f.run(types, box)


def scen_gold(f):
    """A nether-roof farm: a 15x15 slab floor at y 129 on the bedrock roof, the player 50 blocks away above it."""
    f.prep(time_of_day=18000)
    xz = locate_biome_xz(f.t, "minecraft:nether_wastes", "the_nether")
    x0, z0 = xz
    F(f.t, "nether_wastes located", xz)
    f.tp("the_nether", x0 + 0.5, 191, z0 + 0.5)
    f.cmd(f"execute in minecraft:the_nether run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    f.wait_chunks(30)
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 2} 189 {z0 - 2} {x0 + 2} 190 {z0 + 2} minecraft:glass")  # the player stands high above the farm
    f.tp("the_nether", x0 + 0.5, 191, z0 + 0.5)
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 133 {z0 + 7} minecraft:air")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 128 {z0 + 7} minecraft:stone")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 8} 129 {z0 - 8} {x0 + 8} 132 {z0 - 8} minecraft:stone")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 8} 129 {z0 + 8} {x0 + 8} 132 {z0 + 8} minecraft:stone")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 8} 129 {z0 - 8} {x0 - 8} 132 {z0 + 8} minecraft:stone")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 + 8} 129 {z0 - 8} {x0 + 8} 132 {z0 + 8} minecraft:stone")
    f.cmd(f"execute in minecraft:the_nether run fill {x0 - 8} 133 {z0 - 8} {x0 + 8} 133 {z0 + 8} minecraft:stone")
    f.cmd("kill @e[type=!player]")
    F(f.t, "biome at the farm", f.cmd(f"execute in minecraft:the_nether run locate biome minecraft:nether_wastes", wait=3)[-120:])
    box = (x0 - 7, 129, z0 - 7, x0 + 7, 132, z0 + 7) if os.environ.get("GOLDBOX") != "big" else (x0 - 120, 0, z0 - 120, x0 + 120, 255, z0 + 120)
    return f.run(["zombified_piglin", "piglin", "ghast", "magma_cube", "enderman", "hoglin", "strider", "skeleton", "wither_skeleton", "blaze"], box)


def scen_enderman(f):
    f.prep(time_of_day=6000)
    # the outer island ring around 1000, 1000 (end_midlands or end_highlands): 7x7 platform
    x0, z0 = 1200, 0
    f.tp("the_end", x0 + 0.5, 101, z0 + 0.5)
    f.cmd(f"execute in minecraft:the_end run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    f.wait_chunks(30)
    f.cmd(f"execute in minecraft:the_end run fill {x0 - 2} 99 {z0 - 2} {x0 + 2} 100 {z0 + 2} minecraft:glass")  # the player stands 55 blocks above the farm
    f.tp("the_end", x0 + 0.5, 101, z0 + 0.5)
    f.cmd(f"execute in minecraft:the_end run fill {x0 - 7} 40 {z0 - 7} {x0 + 7} 46 {z0 + 7} minecraft:air")
    f.cmd(f"execute in minecraft:the_end run fill {x0 - 7} 40 {z0 - 7} {x0 + 7} 40 {z0 + 7} minecraft:stone")
    f.cmd("kill @e[type=!player]")
    return f.run(["enderman", "endermite", "shulker"], (x0 - 7, 41, z0 - 7, x0 + 7, 46, z0 + 7))


def main():
    name = sys.argv[1]
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    ticks = int(sys.argv[3]) if len(sys.argv) > 3 else 1200
    results = {}

    def work(t):
        f = Farm(t, name, rounds, ticks)
        try:
            if name == "slime":
                results[t] = scen_slime(f)
            elif name == "slime_control":
                results[t] = scen_slime(f, control=True)
            elif name == "creeper":
                results[t] = scen_creeper(f)
            elif name == "witch":
                results[t] = scen_structure(f, "swamp_hut", ["witch", "cat"], half=6, time_of_day=18000)
            elif name == "outpost":
                results[t] = scen_structure(f, "pillager_outpost", ["pillager", "vindicator", "ravager"], half=24)
            elif name == "monument":
                results[t] = scen_structure(f, "monument", ["guardian", "elder_guardian"], half=30)
            elif name == "gold":
                results[t] = scen_gold(f)
            elif name == "enderman":
                results[t] = scen_enderman(f)
            else:
                raise SystemExit("unknown scenario")
        except Exception as e:
            results[t] = f"ERROR {e!r}"
            raise

    threads = [threading.Thread(target=work, args=(t,)) for t in "KV"]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    print("RESULT", name, json.dumps(results, default=str))


main()
