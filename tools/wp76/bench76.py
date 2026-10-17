"""wp76 item-entity benchmark: the same item scenes on Kiln and on vanilla 26.3 (same seed 76, noise terrain,
same commands, same gamerules), MSPT read with `/tick query` (average and P50/P95/P99 of the last 100 ticks).

On the VM:  python3 bench76.py pack                     write the datapack to ~/wp76/pack/wp76
            python3 bench76.py run K|V [-k SCENES] [--out FILE] [--bin PATH] [--window S]
Servers come from servers76.sh (Kiln 25590, vanilla 25597); an idle kiln-bot (spectator, a few blocks above
each scene) keeps the chunks and receives the item entities, so tracking and packets are part of the cost.

Scenes (each in its own place, all forceloaded from the start):
  pile1000/pile5000/pile10000  a kill chamber (3x3 floor, glass walls, in the air) with drops that cannot merge:
                               70% full stacks, 15% partial stacks over 32 (no two fit together), 15% damaged or
                               enchanted gear
  farm5000                     pile5000 plus a farm's inflow: 4 single drops per tick fall into the chamber
  merge5000                    5000 single drops of 5 kinds summoned at once on a 6x6 floor: the merge storm
  tnt                          405 TNT blocks under the surface, lit: the crater's drops scattered around
  stream                       a 128-block water channel (16 steps), 4 items per tick at its head for 75 s
  hopper5000                   5000 non-matching drops lying on a 16x16 hopper floor (every hopper looks for
                               items every tick and finds nothing it can take)
  reload5000                   pile5000 in chunks that unload (bot away, forceload removed) and load again
"""

import argparse
import json
import random
import re
import subprocess
import sys
import time
from pathlib import Path

HOME = Path.home()
W = HOME / "wp76"
TOOLS = Path(__file__).resolve().parent
P99 = re.compile(r"P50: ([\d.]+) ?ms P95: ([\d.]+) ?ms P99: ([\d.]+) ?ms")
AVG = re.compile(r"Average time per tick: ([\d.]+) ?ms")
COUNT = re.compile(r"Test passed[.,] [Cc]ount: (\d+)")
BOT = HOME / "wp76/bin/base/kiln-bot"

FULL = ["rotten_flesh", "bone", "arrow", "string", "gunpowder", "spider_eye", "iron_ingot", "redstone", "glass_bottle", "stick"]
CENSUS = ["zombie", "husk", "drowned", "skeleton", "stray", "bogged", "creeper", "spider", "cave_spider", "enderman", "witch", "slime",
          "zombie_villager", "phantom", "bat", "squid", "glow_squid", "cod", "salmon", "tropical_fish", "pufferfish", "dolphin", "axolotl",
          "cow", "sheep", "pig", "chicken", "rabbit", "horse", "donkey", "llama", "wolf", "fox", "goat", "frog", "turtle", "parrot", "panda",
          "polar_bear", "armadillo", "camel", "mooshroom", "ocelot", "cat", "bee", "villager"]
GEAR = ["bow", "golden_sword", "iron_helmet", "golden_chestplate", "iron_shovel", "crossbow", "fishing_rod"]

# scene: (x, y, z) of its place (the chamber's floor centre), chunks to forceload around it
PLACES = {
    "pile": (8, 150, 8),
    "farm": (264, 150, 8),
    "merge": (520, 150, 8),
    "tnt": (8, None, 520),
    "stream": (264, 150, 520),
    "hopper": (520, 150, 520),
    "reload": (776, 150, 776),
    "scatter": (2008, None, 2008),
    # wp74's witch hut farm: an 11x11 floor across chunk borders, a repeating command block killing
    # everything but players outside the box every tick; the drops inside pile up
    "hut": (1280, 150, 1280),
}
SCATTER_R = 120


def item(i, rng):
    """Drop number i of a pile that cannot merge."""
    k = i % 20
    if k < 14:
        return f'{{id:"minecraft:{FULL[i % len(FULL)]}",count:64}}'
    if k < 17:
        return f'{{id:"minecraft:{FULL[(i // 3) % len(FULL)]}",count:{rng.randint(33, 63)}}}'
    g = GEAR[i % len(GEAR)]
    if i % 2:
        return f'{{id:"minecraft:{g}",count:1,components:{{"minecraft:damage":{rng.randint(1, 30)}}}}}'
    ench = "minecraft:unbreaking" if g != "bow" else "minecraft:power"
    return f'{{id:"minecraft:{g}",count:1,components:{{"minecraft:enchantments":{{"{ench}":{rng.randint(1, 3)}}},"minecraft:damage":{rng.randint(0, 30)}}}}}'


def chamber(x, y, z, r=1):
    """A (2r+1)^2 glass-floored chamber with glass walls, floor top at y."""
    return [f"fill {x - r - 1} {y - 1} {z - r - 1} {x + r + 1} {y + 3} {z + r + 1} glass",
            f"fill {x - r} {y} {z - r} {x + r} {y + 3} {z + r} air"]


def pile_lines(x, y, z, n, seed, r=1):
    rng = random.Random(seed)
    out = []
    for i in range(n):
        px = x - r + rng.random() * (2 * r + 1)
        pz = z - r + rng.random() * (2 * r + 1)
        py = y + 0.2 + rng.random() * 1.5
        out.append(f"summon item {px:.3f} {py:.3f} {pz:.3f} {{Item:{item(i, rng)}}}")
    return out


def write_pack():
    pack = W / "pack" / "wp76"
    fn = pack / "data" / "wp76" / "function"
    fn.mkdir(parents=True, exist_ok=True)
    (pack / "pack.mcmeta").write_text(json.dumps({"pack": {"description": "wp76 bench", "min_format": 121, "max_format": 121}}))
    tags = pack / "data" / "minecraft" / "tags" / "function"
    tags.mkdir(parents=True, exist_ok=True)
    (tags / "tick.json").write_text(json.dumps({"values": ["wp76:tick"]}))
    (tags / "load.json").write_text(json.dumps({"values": ["wp76:load"]}))
    (fn / "load.mcfunction").write_text("scoreboard objectives add wp76 dummy\n")
    (fn / "tick.mcfunction").write_text(
        "execute if score farm wp76 matches 1.. run function wp76:farm_emit\n"
        "execute if score stream wp76 matches 1.. run function wp76:stream_emit\n"
        "execute if score killer wp76 matches 1.. run function wp76:killer\n")
    # chambers
    for scene in ("pile", "farm", "reload"):
        x, y, z = PLACES[scene]
        (fn / f"{scene}_build.mcfunction").write_text("\n".join(chamber(x, y, z)) + "\n")
    x, y, z = PLACES["pile"]
    for n in (1000, 5000, 10000):
        (fn / f"pile{n}.mcfunction").write_text("\n".join(pile_lines(x, y, z, n, 76)) + "\n")
    x, y, z = PLACES["farm"]
    (fn / "farm5000.mcfunction").write_text("\n".join(pile_lines(x, y, z, 5000, 77)) + "\n")
    # the farm's inflow: 4 singles a tick from above the chamber (mobs dying on the kill floor)
    rng = random.Random(78)
    emit = ["scoreboard players remove farm wp76 1"]
    for k in range(4):
        kind = ["rotten_flesh", "bone", "arrow", "string"][k]
        emit.append(f"summon item {x - 1 + rng.random() * 3:.3f} {y + 2.5:.3f} {z - 1 + rng.random() * 3:.3f} "
                    f"{{Item:{{id:\"minecraft:{kind}\",count:1}},Motion:[0.0,-0.1,0.0]}}")
    (fn / "farm_emit.mcfunction").write_text("\n".join(emit) + "\n")
    x, y, z = PLACES["reload"]
    (fn / "reload5000.mcfunction").write_text("\n".join(pile_lines(x, y, z, 5000, 79)) + "\n")
    # merge storm: 5000 singles of 5 kinds on a 6x6 floor (r=3 -> 7x7)
    x, y, z = PLACES["merge"]
    rng = random.Random(80)
    lines = chamber(x, y, z, 3)
    lines.append("say wp76 built")
    (fn / "merge_build.mcfunction").write_text("\n".join(lines) + "\n")
    lines = []
    for i in range(5000):
        kind = ["rotten_flesh", "bone", "arrow", "string", "gunpowder"][i % 5]
        lines.append(f"summon item {x - 3 + rng.random() * 7:.3f} {y + 0.2 + rng.random():.3f} {z - 3 + rng.random() * 7:.3f} "
                     f"{{Item:{{id:\"minecraft:{kind}\",count:1}}}}")
    (fn / "merge5000.mcfunction").write_text("\n".join(lines) + "\n")
    # TNT: 9x5x9 blocks 2..6 under the surface, lit from the top
    x, _, z = PLACES["tnt"]
    (fn / "tnt_build.mcfunction").write_text(
        f"execute positioned {x} 0 {z} positioned over motion_blocking_no_leaves run fill ~-4 ~-6 ~-4 ~4 ~-2 ~4 tnt\n"
        f"execute positioned {x} 0 {z} positioned over motion_blocking_no_leaves run setblock ~ ~-1 ~ redstone_block\n")
    # stream: 16 segments of 8 blocks along +x, each one lower, a source at each segment's start; a 5x5 bin at the end
    x, y, z = PLACES["stream"]
    lines = []
    for k in range(16):
        x0, fy = x + 8 * k, y - k
        lines.append(f"fill {x0} {fy} {z - 1} {x0 + 7} {fy + 2} {z + 1} glass")
        lines.append(f"fill {x0} {fy + 1} {z} {x0 + 7} {fy + 2} {z} air")
    bx, by = x + 128, y - 16
    lines.append(f"fill {bx} {by - 3} {z - 3} {bx + 6} {by + 2} {z + 3} glass")
    lines.append(f"fill {bx + 1} {by - 2} {z - 2} {bx + 5} {by + 2} {z + 2} air")
    lines.append(f"fill {bx} {by + 1} {z} {bx} {by + 2} {z} air")
    for k in range(16):
        lines.append(f"setblock {x + 8 * k} {y - k + 1} {z} water")
    (fn / "stream_build.mcfunction").write_text("\n".join(lines) + "\n")
    rng = random.Random(81)
    emit = ["scoreboard players remove stream wp76 1"]
    for k in range(4):
        it = (f'{{id:"minecraft:{FULL[k * 2]}",count:64}}' if k < 2 else f'{{id:"minecraft:{["cobblestone", "dirt"][k - 2]}",count:1}}')
        emit.append(f"summon item {x + 0.3 + rng.random() * 0.4:.3f} {y + 1.2:.3f} {z + 0.3 + rng.random() * 0.4:.3f} {{Item:{it}}}")
    (fn / "stream_emit.mcfunction").write_text("\n".join(emit) + "\n")
    # hoppers: 16x16 floor, each with 5 single items that nothing above matches; glass rim
    x, y, z = PLACES["hopper"]
    items = ",".join(f'{{Slot:{s}b,id:"minecraft:{n}",count:1}}' for s, n in enumerate(["diamond", "emerald", "gold_ingot", "lapis_lazuli", "quartz"]))
    (fn / "hopper_build.mcfunction").write_text(
        f"fill {x - 9} {y - 1} {z - 9} {x + 8} {y + 3} {z + 8} glass\n"
        f"fill {x - 8} {y} {z - 8} {x + 7} {y + 3} {z + 7} air\n"
        f"fill {x - 8} {y - 1} {z - 8} {x + 7} {y - 1} {z + 7} hopper[facing=down]{{Items:[{items}]}}\n"
        f"fill {x - 8} {y - 2} {z - 8} {x + 7} {y - 2} {z + 7} stone\n")
    rng = random.Random(82)
    lines = []
    for i in range(5000):
        kind = FULL[i % len(FULL)]
        c = 64 if i % 4 else rng.randint(33, 63)
        lines.append(f"summon item {x - 8 + rng.random() * 16:.3f} {y + 0.2 + rng.random():.3f} {z - 8 + rng.random() * 16:.3f} "
                     f"{{Item:{{id:\"minecraft:{kind}\",count:{c}}}}}")
    (fn / "hopper5000.mcfunction").write_text("\n".join(lines) + "\n")
    # scatter: a farm's drops spread over the simulated area (mobs killed where they stood), on the surface
    x, _, z = PLACES["scatter"]
    drops = ["rotten_flesh", "bone", "arrow", "string", "gunpowder", "spider_eye", "carrot", "feather", "leather", "ink_sac"]
    for n in (1000, 5000, 10000):
        rng = random.Random(83 + n)
        lines = []
        for i in range(n):
            px = x - SCATTER_R + rng.random() * 2 * SCATTER_R
            pz = z - SCATTER_R + rng.random() * 2 * SCATTER_R
            c = 1 if i % 3 else rng.randint(1, 3)
            lines.append(f"execute positioned {px:.3f} 0 {pz:.3f} positioned over motion_blocking run summon item ~ ~0.1 ~ "
                         f"{{Item:{{id:\"minecraft:{drops[i % len(drops)]}\",count:{c}}}}}")
        (fn / f"scatter{n}.mcfunction").write_text("\n".join(lines) + "\n")
    # the hut: floor top at y, box x/z -5..+5 around the centre, y..y+14; the command block 12 west
    x, y, z = PLACES["hut"]
    box = f"x={x - 5},y={y},z={z - 5},dx=10,dy=14,dz=10"
    (fn / "hut_build.mcfunction").write_text(
        f"fill {x - 6} {y - 1} {z - 6} {x + 6} {y + 4} {z + 6} glass\n"
        f"fill {x - 5} {y} {z - 5} {x + 5} {y + 4} {z + 5} air\n"
        "gamerule command_block_output false\n"
        f"setblock {x - 18} {y + 8} {z} repeating_command_block{{auto:1b,TrackOutput:0b,"
        f"Command:\"execute as @e[type=!player] unless entity @s[{box}] run kill @s\"}}\n")
    (fn / "hut_off.mcfunction").write_text(f"setblock {x - 18} {y + 8} {z} air\n")
    for n in (1000, 5000, 10000):
        rng = random.Random(90 + n)
        lines = []
        for i in range(n):
            px = x - 5 + rng.random() * 11
            pz = z - 5 + rng.random() * 11
            lines.append(f"summon item {px:.3f} {y + 0.2 + rng.random():.3f} {pz:.3f} {{Item:{item(i, rng)}}}")
        (fn / f"hut{n}.mcfunction").write_text("\n".join(lines) + "\n")
    # the farm's killer (wp74): every tick, everything but players and items
    (fn / "killer.mcfunction").write_text("kill @e[type=!player,type=!item]\n")
    print("pack written to", pack)


class Server:
    def __init__(self, which):
        self.which = which
        self.dir = W / which
        self.log = self.dir / "server.log"
        self.fifo = open(self.dir / "console", "w", buffering=1)
        self.pos = 0
        self.n = 0

    def lines(self):
        with open(self.log, encoding="utf-8", errors="replace") as f:
            f.seek(self.pos)
            text = f.read()
            self.pos = f.tell()
        return text.splitlines()

    def send(self, cmd):
        self.fifo.write(cmd + "\n")
        self.fifo.flush()

    def query(self, timeout=600.0):
        """`tick query` -> (avg, p50, p95, p99)."""
        self.send("tick query")
        end = time.time() + timeout
        avg = None
        while time.time() < end:
            for l in self.lines():
                m = AVG.search(l)
                if m:
                    avg = float(m.group(1))
                m = P99.search(l)
                if m:
                    return avg, float(m.group(1)), float(m.group(2)), float(m.group(3))
            time.sleep(0.1)
        raise TimeoutError("no tick query answer")

    def run(self, cmd, timeout=900.0):
        """Sends a command and waits until a marker after it ran (same tick); returns the lines between."""
        self.n += 1
        self.send(cmd)
        self.send(f"wp76marker{self.n}")
        end = time.time() + timeout
        got = []
        while time.time() < end:
            for l in self.lines():
                if f"wp76marker{self.n}" in l:
                    return got
                got.append(l)
            time.sleep(0.05)
        raise TimeoutError(cmd)

    def wait_for(self, text, timeout=900.0):
        end = time.time() + timeout
        while time.time() < end:
            for l in self.lines():
                if text in l:
                    return
            time.sleep(0.1)
        raise TimeoutError(text)

    def items(self):
        for l in self.run("execute if entity @e[type=item]"):
            m = COUNT.search(l)
            if m:
                return int(m.group(1))
        return 0


def measure(s, seconds, label):
    """`tick query` every 5 s over `seconds`: mean of the 100-tick averages, worst P99, item counts."""
    n0 = s.items()
    rows = []
    t_end = time.time() + seconds
    while time.time() < t_end:
        time.sleep(5)
        rows.append(s.query())
    n1 = s.items()
    avg = sum(r[0] for r in rows) / len(rows)
    p50 = sorted(r[1] for r in rows)[len(rows) // 2]
    p99 = max(r[3] for r in rows)
    print(f"[{s.which}] {label:24s} items {n0:6d}->{n1:6d}  mean {avg:7.2f} ms  p50 {p50:6.2f}  worst p99 {p99:7.1f}", flush=True)
    return {"scene": label, "items0": n0, "items1": n1, "mean": round(avg, 3), "p50": p50, "p99max": p99, "rows": rows}


def tp(s, scene, dy=12):
    x, y, z = PLACES[scene]
    s.run(f"tp Bench0 {x + 0.5} {(y or 100) + dy} {z + 0.5}")


def run(args):
    which = args.which
    subprocess.run([str(TOOLS / "servers76.sh"), which, "start"] + ([args.bin] if args.bin else []), check=True)
    s = Server(which)
    t0 = time.time()
    while True:
        try:
            s.query(timeout=5)
            break
        except (TimeoutError, FileNotFoundError):
            if time.time() - t0 > 600:
                sys.exit("server did not start")
    print(f"[{which}] up after {time.time() - t0:.0f}s", flush=True)
    for c in ["difficulty peaceful", "gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false",
              "gamerule max_command_sequence_length 1000000", "gamerule max_block_modifications 1000000"]:
        s.run(c)
    for scene, (x, y, z) in PLACES.items():
        if scene != "reload":
            s.run(f"forceload add {x - 24} {z - 24} {x + 24 + (140 if scene == 'stream' else 0)} {z + 24}")
    bot = subprocess.Popen([str(BOT), "--addr", f"127.0.0.1:{25590 if which == 'K' else 25597}",
                            "--count", "1", "--behavior", "idle", "--view-distance", "10", "--duration", "36000", "--name-prefix", "Bench", "--report-interval", "36000"],
                           stdout=open(W / f"bot_{which}.log", "w"), stderr=subprocess.STDOUT)
    results = []
    scenes = args.k.split(",")
    try:
        time.sleep(5)
        s.run("gamemode spectator Bench0")
        tp(s, "pile")
        settle = time.time()
        while time.time() - settle < 900:
            time.sleep(10)
            avg, p50, _, p99 = s.query()
            print(f"[{which}] settling: avg {avg} p99 {p99}", flush=True)
            if p99 < 15 and time.time() - settle > 30:
                break
        results.append(measure(s, 15, "idle"))
        for scene in scenes:
            if scene.startswith("scatter"):
                x, _, z = PLACES["scatter"]
                if not getattr(s, "scatter_ready", False):
                    s.run(f"tp Bench0 {x + 0.5} 140 {z + 0.5}")
                    settle = time.time()
                    while time.time() - settle < 300:
                        time.sleep(10)
                        avg, p50, _, p99 = s.query()
                        print(f"[{which}] scatter settling: avg {avg} p99 {p99}", flush=True)
                        if p99 < 15 and time.time() - settle > 40:
                            break
                    s.scatter_ready = True
                    results.append(measure(s, 15, "scatter idle"))
                s.run(f"function wp76:{scene}")
                time.sleep(10)
                results.append(measure(s, args.window, scene))
                s.run("scoreboard players set killer wp76 1")
                time.sleep(3)
                results.append(measure(s, args.window, scene + "+killer"))
                s.run("scoreboard players set killer wp76 0")
                s.run("kill @e[type=item]")
            elif scene.startswith("mobfarm"):
                # wp74's farm rounds: night, monsters spawning around a creative player, `tick sprint 1000`, then
                # every mob killed where it stands (`kill @e[type=!player,type=!item]`): the drops pile up over
                # the simulated area round after round (items live 6000 ticks)
                rounds = int(scene[7:] or 8)
                x, _, z = PLACES["scatter"]
                s.run(f"tp Bench0 {x + 0.5} 140 {z + 0.5}")
                time.sleep(20)
                for c in ["gamemode creative Bench0", f"execute positioned {x} 0 {z} positioned over motion_blocking run tp Bench0 ~ ~ ~",
                          "difficulty normal", "time set 18000", "gamerule spawn_mobs true"]:
                    s.run(c)
                for r in range(rounds):
                    s.send("tick sprint 1000")
                    s.wait_for("Sprint completed", 900)
                    time.sleep(1)
                    results.append(measure(s, 10, f"mobfarm round {r}"))
                    census = {}
                    for t in CENSUS:
                        n = 0
                        for l in s.run(f"execute if entity @e[type={t}]"):
                            m = COUNT.search(l)
                            if m:
                                n = int(m.group(1))
                        if n:
                            census[t] = n
                    print(f"[{which}]   census: {census}", flush=True)
                    results[-1]["census"] = census
                    killed = [l for l in s.run("kill @e[type=!player,type=!item]") if "Killed" in l or "killed" in l]
                    print(f"[{which}]   killed: {killed[-1][-60:] if killed else '-'}", flush=True)
                    results[-1]["killed"] = killed[-1][-60:] if killed else ""
                    time.sleep(1)
                s.run("gamerule spawn_mobs false")
                s.run("difficulty peaceful")
                s.run("kill @e[type=item]")
            elif scene.startswith("hut"):
                if not getattr(s, "hut_ready", False):
                    tp(s, "hut")
                    time.sleep(3)
                    s.run("function wp76:hut_build")
                    s.hut_ready = True
                    time.sleep(5)
                    results.append(measure(s, 15, "hut idle (killer on)"))
                s.run(f"function wp76:{scene}")
                time.sleep(10)
                results.append(measure(s, args.window, scene))
                if scene == scenes[-1] or not scenes[scenes.index(scene) + 1].startswith("hut"):
                    s.run("function wp76:hut_off")
                    s.hut_ready = False
                s.run("kill @e[type=item]")
            elif scene.startswith("pile"):
                n = int(scene[4:])
                tp(s, "pile")
                s.run("function wp76:pile_build")
                s.run(f"function wp76:{scene}")
                time.sleep(10)
                results.append(measure(s, args.window, scene))
                s.run("kill @e[type=item]")
            elif scene == "farm5000":
                tp(s, "farm")
                s.run("function wp76:farm_build")
                s.run("function wp76:farm5000")
                s.run("scoreboard players set farm wp76 100000")
                time.sleep(10)
                results.append(measure(s, args.window, scene))
                s.run("scoreboard players set farm wp76 0")
                s.run("kill @e[type=item]")
            elif scene == "merge5000":
                tp(s, "merge")
                s.run("function wp76:merge_build")
                s.run("function wp76:merge5000")
                results.append(measure(s, 10, "merge5000 first 10 s"))
                results.append(measure(s, 15, "merge5000 after"))
                s.run("kill @e[type=item]")
            elif scene == "tnt":
                tp(s, "tnt", 40)
                time.sleep(3)
                s.run("function wp76:tnt_build")
                time.sleep(10)
                results.append(measure(s, args.window, "tnt crater"))
                s.run("kill @e[type=item]")
            elif scene == "stream":
                x, y, z = PLACES["stream"]
                s.run(f"tp Bench0 {x + 64} {y + 10} {z + 0.5}")
                s.run("function wp76:stream_build")
                time.sleep(3)
                s.run("scoreboard players set stream wp76 1500")
                time.sleep(25)
                results.append(measure(s, 40, "stream (4/tick)"))
                s.run("scoreboard players set stream wp76 0")
                s.run("kill @e[type=item]")
            elif scene == "hopper5000":
                tp(s, "hopper")
                s.run("function wp76:hopper_build")
                s.run("function wp76:hopper5000")
                time.sleep(10)
                results.append(measure(s, args.window, scene))
                s.run("kill @e[type=item]")
            elif scene == "reload5000":
                x, y, z = PLACES["reload"]
                s.run(f"forceload add {x - 24} {z - 24} {x + 24} {z + 24}")
                tp(s, "reload")
                time.sleep(5)
                s.run("function wp76:reload_build")
                s.run("function wp76:reload5000")
                time.sleep(10)
                results.append(measure(s, 15, "reload5000 loaded"))
                # away: the bot to the pile, the chunks lose their tickets and unload with their items
                tp(s, "pile")
                s.run(f"forceload remove {x - 24} {z - 24} {x + 24} {z + 24}")
                results.append(measure(s, 30, "reload5000 unloading"))
                s.run(f"forceload add {x - 24} {z - 24} {x + 24} {z + 24}")
                tp(s, "reload")
                results.append(measure(s, 15, "reload5000 reloading"))
                results.append(measure(s, 15, "reload5000 back"))
                s.run("kill @e[type=item]")
            time.sleep(8)
        if args.out:
            Path(args.out).write_text(json.dumps(results, indent=1))
    finally:
        bot.terminate()
        subprocess.run([str(TOOLS / "servers76.sh"), which, "stop"])


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pack")
    r = sub.add_parser("run")
    r.add_argument("which", choices=["K", "V"])
    r.add_argument("-k", default="scatter1000,scatter5000,scatter10000,pile1000,pile5000,pile10000,farm5000,merge5000,tnt,stream,hopper5000,reload5000")
    r.add_argument("--out")
    r.add_argument("--bin")
    r.add_argument("--window", type=int, default=30)
    a = ap.parse_args()
    if a.cmd == "pack":
        write_pack()
    else:
        run(a)


if __name__ == "__main__":
    main()
