"""wp70 command benchmark: the same block-editing and bulk commands on Kiln and on vanilla 26.3 (same seed,
same world), each command's worst tick read with `/tick query` (P99 of the last 100 ticks is the slowest of
them; the median is the idle tick).

On the VM:  python3 bench70.py pack                    write the datapack to ~/wp70/pack/wp70
            python3 bench70.py run K|V [-k TEXT] [--repeat N] [--out FILE] [--bin PATH]
Servers come from wp70_servers.sh (Kiln 25590, vanilla 25597); an idle kiln-bot (spectator, above the test
area) gets the chunks, so packet building and sending is part of the cost on both servers.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

HOME = Path.home()
W = HOME / "wp70"
TOOLS = Path(__file__).resolve().parent
P99 = re.compile(r"P50: ([\d.]+) ?ms P95: ([\d.]+) ?ms P99: ([\d.]+) ?ms")
AVG = re.compile(r"Average time per tick: ([\d.]+)ms")

# name, setup commands (not measured), measured command, blocks it touches (per-block cost)
X0, Z0 = 0, 0


def cube(x, y, z, n):
    return f"{x} {y} {z} {x + n - 1} {y + n - 1} {z + n - 1}"


CASES = [
    # air above the terrain
    ("fill4096 air->stone", [], f"fill {cube(0, 150, 0, 16)} stone", 4096),
    ("fill4096 stone->air", [], f"fill {cube(0, 150, 0, 16)} air", 4096),
    ("fill32768 air->stone", [], f"fill {cube(0, 150, 0, 32)} stone", 32768),
    ("fill32768 replace filter stone->glass", [], f"fill {cube(0, 150, 0, 32)} glass replace stone", 32768),
    ("fill32768 hollow", [], f"fill {cube(0, 150, 0, 32)} oak_planks hollow", 32768),
    ("fill32768 outline", [], f"fill {cube(0, 150, 0, 32)} stone outline", 5768),
    ("fill32768 strict ->air", [], f"fill {cube(0, 150, 0, 32)} air strict", 32768),
    ("fill32768 keep", [], f"fill {cube(0, 150, 0, 32)} glass keep", 32768),
    ("fill32768 destroy glass", [], f"fill {cube(0, 150, 0, 32)} air destroy", 32768),
    ("fill32768 water", [], f"fill {cube(0, 150, 0, 32)} water", 32768),
    ("fill32768 water->air", [], f"fill {cube(0, 150, 0, 32)} air", 32768),
    # terrain (light, fluids, neighbours)
    ("fill32768 terrain->air", [], f"fill {cube(200, 40, 200, 32)} air", 32768),
    ("fill32768 terrain air->stone", [], f"fill {cube(200, 40, 200, 32)} stone", 32768),
    # setblock
    ("setblock 1", [], "setblock 0 150 0 stone", 1),
    ("setblock destroy", [], "setblock 0 150 0 air destroy", 1),
    # clone
    ("clone32768 setup", [], f"fill {cube(0, 150, 0, 32)} stone", 32768),
    ("clone32768", [], "clone 0 150 0 31 181 31 64 150 0", 32768),
    ("clone32768 masked", [f"fill {cube(64, 150, 0, 32)} air"], "clone 0 150 0 31 181 31 64 150 0 masked", 32768),
    ("clone32768 move", [f"fill {cube(64, 150, 0, 32)} air"], "clone 0 150 0 31 181 31 64 150 0 replace move", 32768),
    ("chests4096 fill", [], f"fill {cube(0, 150, 0, 16)} chest", 4096),
    ("clone4096 chests", [], "clone 0 150 0 15 165 15 64 150 64", 4096),
    ("clone4096 chests force onto chests", [], "clone 0 150 0 15 165 15 64 150 64 replace force", 4096),
    ("fill4096 chests->air", [f"fill {cube(64, 150, 64, 16)} air"], f"fill {cube(0, 150, 0, 16)} air", 4096),
    # biomes
    ("fillbiome32768", [], f"fillbiome {cube(0, 150, 0, 32)} minecraft:desert", 32768),
    ("fillbiome32768 back", [], f"fillbiome {cube(0, 150, 0, 32)} minecraft:plains replace minecraft:desert", 32768),
    # place
    ("place feature oak", ["setblock 300 150 300 dirt"], "place feature minecraft:fancy_oak 300 151 300", 1),
    ("place template", [], "place template minecraft:end_city/fat_tower_base 330 150 330", 1),
    ("place structure igloo", [], "place structure minecraft:igloo 400 150 400", 1),
    # bulk functions
    ("function setblock4096 stone", [], "function wp70:set_stone", 4096),
    ("function setblock4096 air", [], "function wp70:set_air", 4096),
    ("function summon1000", [], "function wp70:summon", 1000),
    ("execute as @e tp x1000", [], "execute as @e[tag=b] at @s run tp @s ~ ~ ~0.5", 1000),
    ("execute as @e data merge x1000", [], "execute as @e[tag=b] run data merge entity @s {CustomName:\"x\"}", 1000),
    ("execute as @e setblock x1000", [], "execute as @e[tag=b] at @s run setblock ~ ~-1 ~ stone", 1000),
    ("function tp10000", [], "function wp70:tp_each", 10000),
    ("kill x1000", [], "kill @e[tag=b]", 1000),
]


def write_pack():
    pack = W / "pack" / "wp70"
    fn = pack / "data" / "wp70" / "function"
    fn.mkdir(parents=True, exist_ok=True)
    (pack / "pack.mcmeta").write_text(json.dumps({"pack": {"description": "wp70 bench", "min_format": 121, "max_format": 121}}))
    cubes = [(x, y, z) for z in range(16) for y in range(16) for x in range(16)]
    for name, block in (("set_stone", "stone"), ("set_air", "air")):
        (fn / f"{name}.mcfunction").write_text("".join(f"setblock {x + 100} {y + 150} {z} {block}\n" for x, y, z in cubes))
    # 1000 armor stands on a 40x25 grid above a platform at y 199
    (fn / "summon.mcfunction").write_text(
        "".join(f"summon minecraft:armor_stand {100 + (i % 40) + 0.5} 201 {100 + i // 40 + 0.5} {{Tags:[\"b\"],NoGravity:1b}}\n" for i in range(1000)))
    # 10 tp per stand, one command per line
    (fn / "tp_each.mcfunction").write_text("execute as @e[tag=b] at @s run tp @s ~ ~ ~0.01\n" * 10)
    print("pack written to", pack)


class Server:
    def __init__(self, which):
        self.which = which
        self.dir = W / which
        self.log = self.dir / "server.log"
        self.fifo = open(self.dir / "console", "w", buffering=1)
        self.pos = 0

    def lines(self):
        with open(self.log, encoding="utf-8", errors="replace") as f:
            f.seek(self.pos)
            text = f.read()
            self.pos = f.tell()
        return text.splitlines()

    def send(self, cmd):
        self.fifo.write(cmd + "\n")
        self.fifo.flush()

    def query(self, timeout=600.0, before=None):
        """Sends `tick query`; returns (p50, p99, avg, lines that came before the answer)."""
        self.send("tick query")
        end = time.time() + timeout
        got = []
        avg = None
        while time.time() < end:
            for l in self.lines():
                m = AVG.search(l)
                if m:
                    avg = float(m.group(1))
                m = P99.search(l)
                if m:
                    return float(m.group(1)), float(m.group(3)), avg, got
                got.append(l)
            time.sleep(0.1)
        raise TimeoutError("no tick query answer")

    def wait_for(self, text, timeout=900.0):
        """Lines up to the first one containing `text`."""
        end = time.time() + timeout
        got = []
        while time.time() < end:
            for l in self.lines():
                if text in l:
                    return got
                got.append(l)
            time.sleep(0.05)
        raise TimeoutError(text)

    def rss_mb(self):
        pat = "kiln-run70" if self.which == "K" else "wp70vanilla-marker"
        try:
            pid = subprocess.check_output(["pgrep", "-f" if self.which == "V" else "-x", pat], text=True).split()[0]
            for l in open(f"/proc/{pid}/status"):
                if l.startswith("VmRSS"):
                    return int(l.split()[1]) // 1024
        except Exception:
            return -1
        return -1


FEEDBACK = re.compile(r"(?:\]: |kiln_sim::commands: )(.*)$")


def feedback(lines):
    out = []
    for l in lines:
        m = FEEDBACK.search(l)
        if m and "tick" not in m.group(1).lower() and "Unknown or incomplete" not in m.group(1):
            out.append(m.group(1))
    return out


def run(args):
    which = args.which
    subprocess.run([str(TOOLS / "wp70_servers.sh"), which, "start"] + ([args.bin] if args.bin else []), check=True)
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
              "forceload add 0 0 111 95", "forceload add 96 96 143 127", "forceload add 192 192 239 239",
              "forceload add 288 288 447 447", "gamerule max_block_modifications 32768"]:
        s.send(c)
    bot = subprocess.Popen([str(HOME / "wt/wp70-command-perf/target/release/kiln-bot"), "--addr", f"127.0.0.1:{25590 if which == 'K' else 25597}",
                            "--count", "1", "--behavior", "idle", "--duration", "36000", "--name-prefix", "Bench", "--report-interval", "36000"],
                           stdout=open(W / f"bot_{which}.log", "w"), stderr=subprocess.STDOUT)
    try:
        time.sleep(5)
        for c in ["gamemode spectator Bench0", "tp Bench0 16 200 16", "fill 100 199 100 140 199 125 barrier"]:
            s.send(c)
        # wait for the forceloaded chunks: ticks get quiet
        settle = time.time()
        while time.time() - settle < 900:
            time.sleep(10)
            p50, p99, _, _ = s.query()
            print(f"[{which}] settling: p50 {p50} p99 {p99}", flush=True)
            if p99 < 15 and time.time() - settle > 30:
                break
        results = []
        cases = [c for c in CASES if not args.k or any(k in c[0] for k in args.k.split(","))]
        for rep in range(args.repeat):
            for name, setup, cmd, blocks in cases:
                for c in setup:
                    s.send(c)
                time.sleep(5.5)  # the previous heavy tick leaves the 100-tick window
                s.query()
                rss0 = s.rss_mb()
                t = time.time()
                s.send(cmd)
                # an unknown command after it marks its end (both run in the same tick: wait for it, then a
                # later tick asks for the times)
                s.send(f"wp70marker{rep}")
                lines = s.wait_for(f"wp70marker{rep}")
                wall = time.time() - t
                time.sleep(1.0)
                p50, p99, avg, _ = s.query()
                rss1 = s.rss_mb()
                fb = feedback(lines)
                r = {"case": name, "rep": rep, "cmd": cmd, "blocks": blocks, "p50": p50, "p99": p99, "avg": avg, "wall_s": round(wall, 2),
                     "rss0": rss0, "rss1": rss1, "feedback": fb[-3:]}
                results.append(r)
                print(f"[{which}] {name:40s} worst {p99:8.1f} ms  idle {p50:5.1f}  per block {(p99 - p50) * 1000 / max(blocks, 1):8.2f} us  "
                      f"rss {rss0}->{rss1} MB  {' | '.join(fb[-2:])[:140]}", flush=True)
        if args.out:
            Path(args.out).write_text(json.dumps(results, indent=1))
    finally:
        bot.terminate()
        subprocess.run([str(TOOLS / "wp70_servers.sh"), which, "stop"])


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("pack")
    r = sub.add_parser("run")
    r.add_argument("which", choices=["K", "V"])
    r.add_argument("-k", default="")
    r.add_argument("--repeat", type=int, default=1)
    r.add_argument("--out")
    r.add_argument("--bin")
    a = ap.parse_args()
    if a.cmd == "pack":
        write_pack()
    else:
        run(a)


if __name__ == "__main__":
    main()
