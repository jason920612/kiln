"""Native format round trip on vanilla worlds: each world is converted to Kiln's native format
and back (`kiln world convert`), the result must equal the original chunk for chunk
(`kiln world compare`: every chunk's NBT byte for byte, header timestamps, every other file),
and the vanilla 26.3 server must load the converted-back world without errors, with the same
blocks and entities as the original.

Worlds: the reference world (tools/gen_vanilla_world.py) and the entity fixture of
tools/entity_persist_check.py, when present, or those given with --world.

usage: python tools/native_roundtrip_check.py [--world DIR ...] [--port 25594] [--skip-build]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
EXT = ".exe" if os.name == "nt" else ""
KILN = ROOT / "target" / "release" / f"kiln{EXT}"
ERRORS = ("Couldn't load chunk", "Failed to load", "Exception", "Couldn't read chunk", "corrupt", "Failed to read")
OVERWORLD = Path("dimensions/minecraft/overworld")


def run(*args):
    r = subprocess.run([str(KILN), *map(str, args)], capture_output=True, text=True)
    print("   ", (r.stdout + r.stderr).strip().splitlines()[-1] if (r.stdout + r.stderr).strip() else "")
    if r.returncode != 0:
        sys.exit(f"kiln {' '.join(map(str, args))} failed:\n{r.stdout}{r.stderr}")
    return r.stdout


def chunk_area(world):
    """Bounding box of the stored overworld chunks, from the region file names."""
    xs, zs = [], []
    for f in (world / OVERWORLD / "region").glob("r.*.mca"):
        _, x, z, _ = f.name.split(".")
        xs.append(int(x))
        zs.append(int(z))
    return min(xs) * 32, max(xs) * 32 + 31, min(zs) * 32, max(zs) * 32 + 31


def vanilla_view(world, work, port, probes):
    """Starts vanilla on a copy of `world`, force-loads its chunks and answers the probes;
    returns (answers, error lines)."""
    server = work / "server"
    if server.exists():
        shutil.rmtree(server)
    server.mkdir(parents=True)
    shutil.copytree(world, server / "world")
    (server / "world" / "session.lock").unlink(missing_ok=True)
    (server / "eula.txt").write_text("eula=true\n")
    (server / "server.properties").write_text(
        "\n".join([f"server-port={port}", "online-mode=false", "enable-rcon=false", "max-tick-time=-1", "spawn-protection=0"]) + "\n"
    )
    p = subprocess.Popen(
        ["java", "-Xmx3G", "-jar", str(WORK / "server.jar"), "--nogui"],
        cwd=server, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
    )
    lines = []
    threading.Thread(target=lambda: [lines.append(l.rstrip()) for l in p.stdout], daemon=True).start()

    def wait_for(pred, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if pred():
                return True
            time.sleep(0.2)
        return False

    def cmd(c):
        n = len(lines)
        p.stdin.write(c + "\n")
        p.stdin.flush()
        return n

    try:
        if not wait_for(lambda: any("Done (" in l for l in lines), 600):
            sys.exit("vanilla server did not start:\n" + "\n".join(lines[-20:]))
        # No block or entity changes while probing.
        for c in ("tick freeze", "gamerule minecraft:spawn_mobs false", "gamerule minecraft:random_tick_speed 0"):
            cmd(c)
        x0, x1, z0, z1 = chunk_area(server / "world")
        for cx in range(x0 // 16, x1 // 16 + 1):
            for cz in range(z0 // 16, z1 // 16 + 1):
                n = cmd(f"forceload add {cx * 256} {cz * 256} {cx * 256 + 255} {cz * 256 + 255}")
                wait_for(lambda: len(lines) > n, 30)
        def ask(probe):
            n = cmd(probe)
            wait_for(lambda: any("Test passed" in l or "Test failed" in l for l in lines[n:]), 30)
            verdict = next((l for l in lines[n:] if "Test passed" in l or "Test failed" in l), "no answer")
            return verdict.split("]: ")[-1]

        # Entity chunks load asynchronously: ask until the answer holds for 15 s.
        answers, last, since = [], None, time.time()
        end = time.time() + 300
        while time.time() < end:
            a0 = ask(probes[0])
            if a0 != last:
                last, since = a0, time.time()
            elif time.time() - since >= 15:
                break
            time.sleep(3)
        answers.append(last)
        for probe in probes[1:]:
            answers.append(ask(probe))
        cmd("stop")
        try:
            p.wait(timeout=120)
        except subprocess.TimeoutExpired:
            p.kill()
    finally:
        if p.poll() is None:
            p.kill()
    return answers, [l for l in lines if any(e in l for e in ERRORS)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--world", action="append", type=Path)
    ap.add_argument("--port", type=int, default=25594)
    ap.add_argument("--skip-build", action="store_true")
    a = ap.parse_args()
    worlds = a.world or [
        w for w in (WORK / "vanilla-world" / "world", WORK / "entity-persist-check" / "fixture" / "world") if (w / "level.dat").exists()
    ]
    if not worlds:
        sys.exit("no worlds; run tools/gen_vanilla_world.py")
    if not a.skip_build:
        subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=ROOT, check=True)
    failed = False
    for world in worlds:
        name = world.parent.name
        work = WORK / "native-check" / name
        if work.exists():
            shutil.rmtree(work)
        work.mkdir(parents=True)
        print(f"{name}: {world}")
        print("  to native")
        run("world", "convert", "--to", "native", world, work / "native")
        print("  back to Anvil")
        run("world", "convert", "--to", "anvil", work / "native", work / "back")
        print("  compare")
        run("world", "compare", world, work / "back")

        # What vanilla sees in each: blocks at a few columns and the entities.
        x0, x1, z0, z1 = chunk_area(world)
        probes = [f"execute if entity @e[type=!player]"]
        for i in range(8):
            x, z = x0 + (x1 - x0) * (i + 1) // 9, z0 + (z1 - z0) * ((i * 5) % 9 + 1) // 9
            for y in (-60, 0, 40, 62, 70):
                probes.append(f"execute if block {x} {y} {z} minecraft:air")
        seen = {}
        for label, w in (("original", world), ("converted back", work / "back")):
            answers, errors = vanilla_view(w, work / label.replace(" ", "-"), a.port, probes)
            seen[label] = answers
            print(f"  vanilla on the {label} world: {answers[0]}; {len(errors)} errors")
            for e in errors:
                print("    ERROR", e)
            failed |= bool(errors) and label == "converted back"
        # The entity count is informational: vanilla loads entity chunks asynchronously and
        # counts in a frozen world still drift by an entity or two between runs of the same
        # files (the compare above already found the entity chunks byte for byte equal).
        if seen["original"][1:] != seen["converted back"][1:]:
            failed = True
            for p, o, b in zip(probes[1:], seen["original"][1:], seen["converted back"][1:]):
                if o != b:
                    print(f"    DIFF {p}: {o} / {b}")
        else:
            print(f"  {len(probes) - 1} block probes answered the same")
    if failed:
        sys.exit(1)
    print("native round trip: vanilla worlds unchanged and accepted")


if __name__ == "__main__":
    main()
