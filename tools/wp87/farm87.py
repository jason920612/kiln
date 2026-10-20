"""wp87: natural spawning decisions of Kiln (K) and vanilla (V) on identical worlds (seed 77, noise generator). One thread per server runs the same
console commands and `tick sprint` rounds; the mobs inside the farm's box are counted and killed after every round, the ones outside are cleared
(so the mob caps stay comparable). Both servers count the spawner's decisions: vanilla through the java agent (`tools/wp74/agent`, `W72_OUT`),
Kiln through `KILN_SPAWN_COUNTERS`; the box of the `validPosition IN-BOX` counts is `BOX` at server start (see sv.sh).
usage: farm87.py SCENARIO [ROUNDS] [ROUND_TICKS]    SCENARIO: nether | end | open
Prints a RESULT line {K: {...}, V: {...}} with the kill counts, then the counters' chi-square (cmp_counts style) from the two files."""
import json, os, re, sys, threading, time
from lk import *

ME = os.environ.get("ME", "W0")
SEED = int(os.environ.get("SEED", "77"))


def F(*a):
    print(*a, flush=True)


def cmd(t, c, wait=0.15):
    return ask(t, c, wait=wait)


def kill_count(t, dim, selector):
    out = ask(t, f"execute in minecraft:{dim} run kill {selector}", wait=0.25)
    m = re.search(r"success\.multiple\[(\d+)\]|Killed (\d+) entities", out)
    if m:
        return int(m.group(1) or m.group(2))
    if "success.single" in out or re.search(r"Killed [A-Za-z]", out):
        return 1
    return 0


class Farm:
    def __init__(self, t, rounds, ticks):
        self.t, self.rounds, self.ticks = t, rounds, ticks
        self.counts = []

    def prep(self, time_of_day):
        t = self.t
        if t == "V":
            w(t, f"op {ME}")
        for g in ("spawn_mobs true", "spawn_monsters true", "advance_time false", "advance_weather false", "spawn_phantoms false", "mob_griefing false",
                  "random_tick_speed 3", "spawn_wardens false", "command_block_output false", "log_admin_commands false"):
            w(t, "gamerule " + g)
        w(t, "difficulty normal")
        w(t, "weather clear")
        w(t, f"time set {time_of_day}")
        w(t, f"gamemode creative {ME}")
        w(t, f"effect give {ME} minecraft:resistance 99999 4 true")
        time.sleep(0.5)

    def tp(self, dim, x, y, z):
        w(self.t, f"execute in minecraft:{dim} run tp {ME} {x} {y} {z}")
        time.sleep(1.0)

    def run(self, dim, types, box):
        bx = (min(box[0], box[3]), min(box[1], box[4]), min(box[2], box[5]), abs(box[3] - box[0]), abs(box[4] - box[1]), abs(box[5] - box[2]))
        inbox = "x=%d,y=%d,z=%d,dx=%d,dy=%d,dz=%d" % bx
        for r in range(self.rounds):
            sprint(self.t, self.ticks)
            counts = {}
            for ty in types:
                counts[ty] = kill_count(self.t, dim, f"@e[type=minecraft:{ty},{inbox}]")
            for _ in range(2):  # children of what was just killed
                sprint(self.t, 2)
                for ty in types:
                    kill_count(self.t, dim, f"@e[type=minecraft:{ty},{inbox}]")
            # the mob cap: nothing stays outside the box (the player is no mob)
            n_out = 0
            for d in ("overworld", "the_nether", "the_end"):
                out = ask(self.t, f"execute in minecraft:{d} as @e[type=!player] unless entity @s[{inbox}] run kill @s", wait=0.25)
            self.counts.append(counts)
            F(self.t, "round", r, {k: v for k, v in counts.items() if v})
        return {ty: sum(c.get(ty, 0) for c in self.counts) for ty in types}


def scen_nether(f):
    """A nether-roof farm: a 15x15 stone floor at y 128 on the bedrock roof, walls to y 133, the player 60 blocks above it."""
    f.prep(18000)
    x0, z0 = -64, 64
    f.tp("the_nether", x0 + 0.5, 191, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:the_nether run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    time.sleep(25)
    cmd(f.t, f"execute in minecraft:the_nether run fill {x0 - 2} 189 {z0 - 2} {x0 + 2} 190 {z0 + 2} minecraft:glass")
    f.tp("the_nether", x0 + 0.5, 191, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 133 {z0 + 7} minecraft:air")
    cmd(f.t, f"execute in minecraft:the_nether run fill {x0 - 7} 128 {z0 - 7} {x0 + 7} 128 {z0 + 7} minecraft:stone")
    for a, b, c, d in ((-8, -8, 8, -8), (-8, 8, 8, 8), (-8, -8, -8, 8), (8, -8, 8, 8)):
        cmd(f.t, f"execute in minecraft:the_nether run fill {x0 + a} 129 {z0 + b} {x0 + c} 132 {z0 + d} minecraft:stone")
    cmd(f.t, f"execute in minecraft:the_nether run fill {x0 - 8} 133 {z0 - 8} {x0 + 8} 133 {z0 + 8} minecraft:stone")
    cmd(f.t, "kill @e[type=!player]")
    return f.run("the_nether", ["zombified_piglin", "piglin", "ghast", "magma_cube", "enderman", "hoglin", "strider", "skeleton", "wither_skeleton", "blaze"],
                 (x0 - 7, 129, z0 - 7, x0 + 7, 132, z0 + 7))


def scen_end(f):
    """Endermen on a stone floor in the outer End islands, the player 55 blocks above."""
    f.prep(6000)
    x0, z0 = 1200, 0
    f.tp("the_end", x0 + 0.5, 101, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:the_end run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    time.sleep(25)
    cmd(f.t, f"execute in minecraft:the_end run fill {x0 - 2} 99 {z0 - 2} {x0 + 2} 100 {z0 + 2} minecraft:glass")
    f.tp("the_end", x0 + 0.5, 101, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:the_end run fill {x0 - 7} 40 {z0 - 7} {x0 + 7} 46 {z0 + 7} minecraft:air")
    cmd(f.t, f"execute in minecraft:the_end run fill {x0 - 7} 40 {z0 - 7} {x0 + 7} 40 {z0 + 7} minecraft:stone")
    cmd(f.t, "kill @e[type=!player]")
    return f.run("the_end", ["enderman", "endermite", "shulker"], (x0 - 7, 41, z0 - 7, x0 + 7, 46, z0 + 7))


def scen_portal(f):
    """Zombified piglins from the portal blocks of a frame in the overworld (`NetherPortalBlock.randomTick`: one random tick in 2000 times the difficulty id),
    with every random tick of a section (`random_tick_speed 4096`) and the player beside it."""
    f.prep(6000)
    w(f.t, "gamerule random_tick_speed 4096")
    x0, z0 = 0, 300
    f.tp("overworld", x0 + 0.5, 220, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:overworld run forceload add {x0 - 16} {z0 - 16} {x0 + 32} {z0 + 32}")
    time.sleep(25)
    cmd(f.t, f"execute in minecraft:overworld run fill {x0 - 4} 147 {z0 - 4} {x0 + 7} 147 {z0 + 4} minecraft:stone")
    cmd(f.t, f"execute in minecraft:overworld run fill {x0 - 4} 148 {z0 - 4} {x0 + 7} 160 {z0 + 4} minecraft:air")
    cmd(f.t, f"execute in minecraft:overworld run fill {x0} 148 {z0} {x0 + 3} 152 {z0} minecraft:obsidian")
    cmd(f.t, f"execute in minecraft:overworld run fill {x0 + 1} 149 {z0} {x0 + 2} 151 {z0} minecraft:nether_portal[axis=x]")
    cmd(f.t, f"execute in minecraft:overworld run tp {ME} {x0 + 1.5} 153 {z0 + 3.5}")
    time.sleep(2)
    cmd(f.t, "kill @e[type=!player]")
    return f.run("overworld", ["zombified_piglin"], (x0 - 4, 148, z0 - 4, x0 + 7, 160, z0 + 4))




CENSUS = ["zombie", "husk", "drowned", "skeleton", "stray", "bogged", "creeper", "spider", "cave_spider", "enderman", "witch", "slime", "zombie_villager", "phantom",
          "bat", "squid", "glow_squid", "cod", "salmon", "tropical_fish", "pufferfish", "dolphin", "axolotl", "cow", "sheep", "pig", "chicken", "rabbit", "horse", "donkey",
          "llama", "wolf", "fox", "goat", "frog", "turtle", "parrot", "panda", "polar_bear", "armadillo", "camel", "mooshroom", "ocelot", "cat", "bee", "villager"]


def scen_ocean(f):
    """wp76's `mobfarm`: a creative player on the water surface at 2008,2008 (seed 76: ocean), night, `tick sprint` rounds, the census of the whole
    overworld after every round (what is counted is killed). """
    f.prep(18000)
    w(f.t, "gamerule advance_time false")
    x0, z0 = 2008, 2008
    f.tp("overworld", x0 + 0.5, 140, z0 + 0.5)
    cmd(f.t, f"execute in minecraft:overworld run forceload add {x0 - 16} {z0 - 16} {x0 + 16} {z0 + 16}")
    time.sleep(20)
    cmd(f.t, f"execute in minecraft:overworld positioned {x0} 0 {z0} positioned over motion_blocking run tp {ME} ~ ~ ~")
    time.sleep(2)
    cmd(f.t, "kill @e[type=!player]")
    totals = {}
    for r in range(f.rounds):
        sprint(f.t, f.ticks)
        census = {}
        for ty in CENSUS:
            n = kill_count(f.t, "overworld", f"@e[type=minecraft:{ty}]")
            if n:
                census[ty] = n
        cmd(f.t, "execute in minecraft:overworld run kill @e[type=!player]")
        for ty, n in census.items():
            totals[ty] = totals.get(ty, 0) + n
        F(f.t, "round", r, census)
    return totals

NETHER_CENSUS = ["zombified_piglin", "piglin", "piglin_brute", "ghast", "magma_cube", "enderman", "hoglin", "strider", "skeleton", "wither_skeleton", "blaze", "zoglin",
                 "zombie", "creeper", "spider", "slime", "bat"]


def scen_nbiome(f):
    """wp87 item 5: natural spawning in a nether biome without any platform: a creative player at BIOME_AT (env X,Y,Z; default the soul sand valley the wp78 seed has at
    -32,~,32) high above the ground, `tick sprint` rounds, the census of the whole nether after every round (what is counted is killed)."""
    f.prep(18000)
    x, y, z = (int(v) for v in os.environ.get("BIOME_AT", "-32,64,32").split(","))
    f.tp("the_nether", x + 0.5, y, z + 0.5)
    cmd(f.t, f"execute in minecraft:the_nether run forceload add {x - 16} {z - 16} {x + 16} {z + 16}")
    time.sleep(25)
    f.tp("the_nether", x + 0.5, y, z + 0.5)
    cmd(f.t, "kill @e[type=!player]")
    totals = {}
    for r in range(f.rounds):
        sprint(f.t, f.ticks)
        census = {}
        for ty in NETHER_CENSUS:
            n = kill_count(f.t, "the_nether", f"@e[type=minecraft:{ty}]")
            if n:
                census[ty] = n
        cmd(f.t, "execute in minecraft:the_nether run kill @e[type=!player]")
        for ty, n in census.items():
            totals[ty] = totals.get(ty, 0) + n
        F(f.t, "round", r, census)
    return totals


def scen_wither(f):
    """wp87 item 5: two survival players 10 blocks apart, a wither summoned between and above them, `tick sprint` rounds, then it is killed and a new one
    comes: the wither's side heads pick their targets (the vanilla agent tools/wp87/agent and KILN_WITHER_TRACE write "tick head type x z" per pick)."""
    f.prep(18000)
    t = f.t
    w(t, "gamerule spawn_mobs false")
    w(t, "gamerule spawn_monsters false")
    w(t, "gamemode survival W0")
    w(t, "gamemode survival W1")
    time.sleep(0.5)
    cmd(t, "execute at W0 run tp W1 ~10 ~ ~")
    time.sleep(1.5)
    for p in ("W0", "W1"):
        cmd(t, f"effect give {p} minecraft:resistance 99999 4 true")
        cmd(t, f"effect give {p} minecraft:regeneration 99999 30 true")
    time.sleep(1)
    for r in range(f.rounds):
        cmd(t, "execute at W0 run summon minecraft:wither ~5 ~4 ~")
        sprint(t, f.ticks)
        n = kill_count(t, "overworld", "@e[type=minecraft:wither]")
        cmd(t, "kill @e[type=!player]")
        sprint(t, 20)
        F(t, "round", r, "withers killed", n)
    return {"rounds": f.rounds}


def main():
    name = sys.argv[1]
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    ticks = int(sys.argv[3]) if len(sys.argv) > 3 else 1200
    results = {}

    def work(t):
        f = Farm(t, rounds, ticks)
        try:
            results[t] = {"nether": scen_nether, "end": scen_end, "portal": scen_portal, "ocean": scen_ocean, "nbiome": scen_nbiome, "wither": scen_wither}[name](f)
        except Exception as e:
            results[t] = f"ERROR {e!r}"
            raise

    threads = [threading.Thread(target=work, args=(t,)) for t in os.environ.get("ONLY", "KV")]
    for th in threads:
        th.start()
    for th in threads:
        th.join()
    print("RESULT", name, json.dumps(results, default=str))


if __name__ == "__main__":
    main()
