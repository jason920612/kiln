"""`/place structure|jigsaw|feature` against the vanilla 26.3 server: the same commands on the same
generated world (same seed), then the blocks each of them changed, read from the saved region
files, are compared.

Vanilla places with `level.getRandom()` (unseeded), so what a structure does with random
numbers (decay, loot seeds, which house variant) differs from run to run even between two
vanilla runs. So the check is statistical: the vanilla runs are compared with each other (the
noise floor) and Kiln with vanilla, over the blocks that changed. Reported per case:
`changed` positions, `same` (equal in both), and the share of mismatches.

usage: python tools/place_diff.py [--seed N] [--kiln-exe PATH] [-k TEXT] [-v]
Ports: the first free of 25581-25583 (one server at a time). Needs KILN_WORK (default
<repo>/work) with the vanilla server.jar and KILN_DATAPACK (default $KILN_WORK/generated).
"""

import argparse
import os
import queue
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK", ROOT / "work"))
SCRATCH = Path(os.environ.get("KILN_PLACE_SCRATCH", WORK / "wp44" / "wp44-end" / "place"))

# (name, forceload area as chunk coordinates min/max, commands run after the first snapshot)
CASES = [
    ("structure desert_pyramid", (4, 4, 12, 12), ["place structure minecraft:desert_pyramid 100 90 100"]),
    ("structure igloo", (4, 4, 12, 12), ["place structure minecraft:igloo 100 90 100"]),
    ("structure swamp_hut", (4, 4, 12, 12), ["place structure minecraft:swamp_hut 100 90 100"]),
    ("structure jungle_pyramid", (4, 4, 12, 12), ["place structure minecraft:jungle_pyramid 100 90 100"]),
    ("structure shipwreck", (4, 4, 12, 12), ["place structure minecraft:shipwreck 100 70 100"]),
    ("structure ruined_portal", (4, 4, 12, 12), ["place structure minecraft:ruined_portal 100 90 100"]),
    ("structure village_plains", (0, 0, 16, 16), ["place structure minecraft:village_plains 130 90 130"]),
    ("structure mineshaft", (4, 4, 12, 12), ["place structure minecraft:mineshaft 100 40 100"]),
    ("structure stronghold", (4, 4, 12, 12), ["place structure minecraft:stronghold 100 40 100"]),
    ("jigsaw village", (2, 2, 14, 14), ["place jigsaw minecraft:village/plains/town_centers minecraft:bottom 6 100 90 100"]),
    ("feature oak", (4, 4, 12, 12), ["place feature minecraft:oak 100 90 100"]),
    ("feature ore_diamond", (4, 4, 12, 12), ["place feature minecraft:ore_diamond_small 100 20 100"]),
    ("feature lake", (4, 4, 12, 12), ["place feature minecraft:lake_lava 100 60 100"]),
]


def port_free(port):
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", port)) != 0


def first_free_port(wait=1800):
    end = time.time() + wait
    while True:
        for p in (25581, 25582, 25583):
            if port_free(p):
                return p
        if time.time() > end:
            sys.exit("no free harness port")
        time.sleep(5)


# ---- a small Anvil reader -------------------------------------------------------------

def read_nbt(data):
    pos = 0

    def u(fmt, n):
        nonlocal pos
        v = struct.unpack_from(fmt, data, pos)[0]
        pos += n
        return v

    def string():
        nonlocal pos
        n = u(">H", 2)
        s = data[pos:pos + n].decode("utf-8", "replace")
        pos += n
        return s

    def payload(t):
        nonlocal pos
        if t == 1:
            return u(">b", 1)
        if t == 2:
            return u(">h", 2)
        if t == 3:
            return u(">i", 4)
        if t == 4:
            return u(">q", 8)
        if t == 5:
            return u(">f", 4)
        if t == 6:
            return u(">d", 8)
        if t == 7:
            n = u(">i", 4)
            v = data[pos:pos + n]
            pos += n
            return v
        if t == 8:
            return string()
        if t == 9:
            et = u(">b", 1)
            n = u(">i", 4)
            return [payload(et) for _ in range(n)]
        if t == 10:
            out = {}
            while True:
                et = u(">b", 1)
                if et == 0:
                    return out
                name = string()
                out[name] = payload(et)
        if t == 11:
            n = u(">i", 4)
            v = struct.unpack_from(f">{n}i", data, pos)
            pos += 4 * n
            return list(v)
        if t == 12:
            n = u(">i", 4)
            v = struct.unpack_from(f">{n}q", data, pos)
            pos += 8 * n
            return list(v)
        raise ValueError(f"nbt tag {t}")

    t = u(">b", 1)
    string()
    return payload(t)


def read_chunk(region_path, x, z):
    with open(region_path, "rb") as f:
        head = f.read(8192)
        entry = struct.unpack_from(">I", head, 4 * ((x & 31) + (z & 31) * 32))[0]
        if entry == 0:
            return None
        offset, count = entry >> 8, entry & 255
        f.seek(offset * 4096)
        length = struct.unpack(">I", f.read(4))[0]
        comp = f.read(1)[0]
        raw = f.read(length - 1)
    if comp == 2:
        raw = zlib.decompress(raw)
    elif comp == 1:
        import gzip
        raw = gzip.decompress(raw)
    elif comp != 3:
        raise ValueError(f"compression {comp}")
    return read_nbt(raw)


def state_string(entry):
    props = entry.get("Properties")
    name = entry["Name"]
    if props:
        name += "[" + ",".join(f"{k}={props[k]}" for k in sorted(props)) + "]"
    return name


def chunk_blocks(chunk):
    """{(x, y, z) local: state} of the non-air blocks of a chunk NBT."""
    out = {}
    for sec in chunk.get("sections", []):
        bs = sec.get("block_states")
        if not bs:
            continue
        palette = [state_string(e) for e in bs["palette"]]
        y0 = sec["Y"] * 16
        if len(palette) == 1:
            if palette[0] != "minecraft:air":
                for i in range(4096):
                    out[(i & 15, y0 + (i >> 8), (i >> 4) & 15)] = palette[0]
            continue
        bits = max(4, (len(palette) - 1).bit_length())
        per = 64 // bits
        mask = (1 << bits) - 1
        longs = bs["data"]
        for i in range(4096):
            v = (longs[i // per] >> ((i % per) * bits)) & mask
            s = palette[v]
            if s != "minecraft:air" and s != "minecraft:cave_air" and s != "minecraft:void_air":
                out[(i & 15, y0 + (i >> 8), (i >> 4) & 15)] = s
    return out


def snapshot(world, area):
    """{(x, y, z): state} of the chunks (cx0, cz0, cx1, cz1) in the overworld of `world`."""
    out = {}
    cx0, cz0, cx1, cz1 = area
    for cz in range(cz0, cz1 + 1):
        for cx in range(cx0, cx1 + 1):
            region = world / "region" / f"r.{cx >> 5}.{cz >> 5}.mca"
            if not region.exists():
                continue
            chunk = read_chunk(region, cx, cz)
            if chunk is None:
                continue
            for (lx, y, lz), s in chunk_blocks(chunk).items():
                out[(cx * 16 + lx, y, cz * 16 + lz)] = s
    return out


# ---- servers ------------------------------------------------------------------------------

class Server:
    def __init__(self, argv, cwd, env, log):
        self.lines = []
        self.q = queue.Queue()
        self.log = open(log, "w", encoding="utf-8")
        self.p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                  text=True, encoding="utf-8", errors="replace", bufsize=1)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.p.stdout:
            line = re.sub(r"\x1b\[[0-9;]*m", "", raw.rstrip("\r\n"))
            self.log.write(line + "\n")
            self.log.flush()
            self.lines.append(line)

    def seen(self, text, since=0):
        return any(text in l for l in self.lines[since:])

    def wait_for(self, text, timeout, since=0):
        end = time.time() + timeout
        while time.time() < end:
            if self.seen(text, since):
                return True
            if self.p.poll() is not None:
                return False
            time.sleep(0.2)
        return False

    def send(self, line):
        self.p.stdin.write(line + "\n")
        self.p.stdin.flush()

    def command(self, line, wait=None, timeout=120):
        n = len(self.lines)
        self.send(line)
        if wait:
            if not self.wait_for(wait, timeout, n):
                print(f"  (no '{wait}' after {line})")
        return self.lines[n:]

    def stop(self):
        if self.p.poll() is None:
            try:
                self.send("stop")
                self.p.wait(timeout=90)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()
        self.log.close()


def start(kind, seed, port, base, kiln_exe):
    shutil.rmtree(base, ignore_errors=True)
    base.mkdir(parents=True)
    if kind == "vanilla":
        (base / "eula.txt").write_text("eula=true\n")
        (base / "server.properties").write_text("\n".join([
            f"server-port={port}", "online-mode=false", f"level-seed={seed}", "level-type=minecraft\\:normal", "generate-structures=true",
            "spawn-protection=0", "max-tick-time=-1", "sync-chunk-writes=false", "enable-rcon=false", "enable-query=false",
            "view-distance=4", "simulation-distance=4", "spawn-monsters=false", "spawn-animals=false", "difficulty=peaceful", "",
        ]))
        argv = ["java", "-Xmx2G", "-jar", str(WORK / "server.jar"), "--nogui"]
        return Server(argv, base, os.environ.copy(), base / "server.log"), base / "world"
    world = base / "world"
    world.mkdir()
    env = os.environ.copy()
    env.update({"KILN_PORT": str(port), "KILN_WORLD": str(world), "KILN_GENERATOR": "noise", "KILN_SEED": str(seed), "RUST_LOG": "info", "NO_COLOR": "1"})
    env.setdefault("KILN_DATAPACK", str(WORK / "generated"))
    exe = base / "kiln-place.exe"
    shutil.copy2(kiln_exe, exe)
    return Server([str(exe)], base, env, base / "server.log"), world


def run_case(kind, seed, area, commands, kiln_exe):
    port = first_free_port()
    base = SCRATCH / kind
    server, world = start(kind, seed, port, base, kiln_exe)
    try:
        ready = "Done (" if kind == "vanilla" else "listening on"
        if not server.wait_for(ready, 600):
            sys.exit(f"{kind} did not start")
        cx0, cz0, cx1, cz1 = area
        server.command(f"forceload add {(cx0 - 1) * 16} {(cz0 - 1) * 16} {(cx1 + 1) * 16 + 15} {(cz1 + 1) * 16 + 15}", "orceload", 60)
        # Wait until the area is loaded.
        for _ in range(240):
            out = server.command(f"execute if loaded {cx1 * 16 + 8} 64 {cz1 * 16 + 8}")
            time.sleep(1.0)
            if any("Test passed" in l for l in server.lines[-5:]):
                break
        time.sleep(3)
        server.command("save-all flush", "Saved the game", 120)
        time.sleep(1)
        before = snapshot(world, area)
        feedback = []
        for c in commands:
            feedback += [l for l in server.command(c, None) if "place" in l.lower() or "Placed" in l or "Could not" in l or "failed" in l.lower()]
            time.sleep(2)
        server.command("save-all flush", "Saved the game", 120)
        time.sleep(1)
        after = snapshot(world, area)
        changed = {p: after.get(p, "minecraft:air") for p in set(before) | set(after) if before.get(p, "minecraft:air") != after.get(p, "minecraft:air")}
        feed = " | ".join(l.split("]: ")[-1] for l in server.lines[-8:] if "Placed" in l or "Could not" in l or "failed" in l.lower())
        return changed, feed
    finally:
        server.stop()


def compare(a, b):
    keys = set(a) | set(b)
    same = sum(1 for k in keys if a.get(k) == b.get(k))
    return len(keys), same


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--kiln-exe", default=str(ROOT / "target" / "release" / "kiln.exe"))
    ap.add_argument("-k", dest="filter")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--no-noise-floor", action="store_true", help="skip the second vanilla run")
    a = ap.parse_args()
    SCRATCH.mkdir(parents=True, exist_ok=True)
    rows = []
    for name, area, commands in CASES:
        if a.filter and a.filter not in name:
            continue
        print(f"== {name}", flush=True)
        v1, f1 = run_case("vanilla", a.seed, area, commands, a.kiln_exe)
        k, fk = run_case("kiln", a.seed, area, commands, a.kiln_exe)
        v2 = None
        if not a.no_noise_floor:
            v2, _ = run_case("vanilla", a.seed, area, commands, a.kiln_exe)
        union, same = compare(v1, k)
        floor = compare(v1, v2) if v2 is not None else None
        print(f"  vanilla changed {len(v1)}, kiln changed {len(k)}; kiln vs vanilla: {same}/{union} positions equal ({100.0 * same / max(union, 1):.1f}%)"
              + (f"; vanilla vs vanilla {floor[1]}/{floor[0]} ({100.0 * floor[1] / max(floor[0], 1):.1f}%)" if floor else ""))
        print(f"  vanilla: {f1}\n  kiln:    {fk}")
        rows.append((name, len(v1), len(k), same, union, floor))
        if a.verbose:
            diff = [p for p in set(v1) | set(k) if v1.get(p) != k.get(p)][:12]
            for p in diff:
                print(f"    {p}: vanilla {v1.get(p)} kiln {k.get(p)}")
    print("\nsummary")
    for name, nv, nk, same, union, floor in rows:
        print(f"  {name:28} vanilla {nv:6} kiln {nk:6} equal {100.0 * same / max(union, 1):5.1f}%" + (f" (vanilla vs vanilla {100.0 * floor[1] / max(floor[0], 1):5.1f}%)" if floor else ""))


if __name__ == "__main__":
    main()
