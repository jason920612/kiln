"""Vanilla acceptance test for statistics, advancements and the recipe book on disk.

1. Vanilla 26.3 on a copy of the reference world: a player joins, walks and jumps a little,
   gets advancements (whole ones and single criteria) and recipes, then leaves; vanilla writes
   players/stats/<uuid>.json, players/advancements/<uuid>.json and recipeBook in the player
   data.
2. Kiln on that world: the same player joins; what vanilla saved must be there (granting
   again fails with "already have it", giving the recipe again learns nothing, the Award Stats
   answer carries vanilla's play time and distances). The player gets more advancements and
   recipes and moves; Kiln saves on leave and `stop`.
3. Vanilla on the world Kiln saved: everything from both runs must be there, and vanilla must
   log no parse errors for the files Kiln wrote.

usage: python tools/advancement_check.py [--out DIR] [--kiln-port 25587] [--vanilla-port 25592]
                                         [--skip-build]
"""

import argparse
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
os.environ["KILN_WORK"] = str(WORK)
sys.path.insert(0, str(ROOT / "tools"))
sys.dont_write_bytecode = True
from persist_check import Client, Server, check, get, nbt_file, offline_uuid, pid, port_free, val, vanilla, RESULTS  # noqa: E402
from smoke_client import Buf, varint  # noqa: E402
import command_diff  # noqa: E402

NAME = "AdvBot"
PLAY_TIME = "minecraft:play_time"


def registry(name):
    reg = json.loads((WORK / "generated" / "reports" / "registries.json").read_text(encoding="utf-8"))
    entries = reg[name]["entries"]
    return {v["protocol_id"]: k for k, v in entries.items()}


STAT_TYPES = registry("minecraft:stat_type")
CUSTOM = registry("minecraft:custom_stat")


def request_stats(c, seconds=10):
    """Client Command REQUEST_STATS, then the Award Stats answer as {(type, name): value}."""
    c.send("play", "client_command", varint(1))
    want = pid("play", "clientbound", "award_stats")
    end = time.time() + seconds
    while time.time() < end:
        c.c.s.settimeout(1)
        try:
            i, b = c.c.recv()
        except Exception:  # noqa: BLE001
            continue
        if i == pid("play", "clientbound", "keep_alive"):
            c.send("play", "keep_alive", b.take(8))
        elif i == want:
            out = {}
            for _ in range(b.varint()):
                t, v, n = b.varint(), b.varint(), b.varint()
                kind = STAT_TYPES[t]
                name = CUSTOM.get(v, str(v)) if kind == "minecraft:custom" else str(v)
                out[(kind, name)] = n
            return out
    return None


def walk(c, steps=20):
    x, y, z, yaw, pitch = c.pos
    for k in range(steps):
        x += 0.2
        c.move(x, y, z, yaw, pitch)
        c.pump(0.06)
    c.pos = (x, y, z, yaw, pitch)


def feedback(server, command, timeout=15):
    return server.query(command, r"Granted|Couldn't|Revoked|Unlocked|No new recipes|Took|No recipes|Unknown|Incorrect", timeout) or ""


FIRST = [
    ("advancement grant {n} only minecraft:story/mine_stone", "Granted the advancement [Stone Age] to"),
    ("advancement grant {n} only minecraft:adventure/kill_a_mob minecraft:zombie", "Granted criterion 'minecraft:zombie'"),
    ("recipe give {n} minecraft:stick", "Unlocked 1 recipe(s)"),
]
AGAIN = [
    ("advancement grant {n} only minecraft:story/mine_stone", "as they already have it"),
    ("advancement grant {n} only minecraft:adventure/kill_a_mob minecraft:zombie", "as they already have it"),
    ("advancement grant {n} only minecraft:adventure/kill_a_mob minecraft:skeleton", "Granted criterion"),
    ("recipe give {n} minecraft:stick", "No new recipes were learned"),
]
SECOND = [
    ("advancement grant {n} only minecraft:story/upgrade_tools", "Granted the advancement [Getting an Upgrade]"),
    ("recipe give {n} minecraft:diamond_block", "Unlocked 1 recipe(s)"),
]
FINAL = AGAIN[:2] + [
    ("advancement grant {n} only minecraft:adventure/kill_a_mob minecraft:skeleton", "as they already have it"),
    ("recipe give {n} minecraft:stick", "No new recipes were learned"),
    ("advancement grant {n} only minecraft:story/upgrade_tools", "as they already have it"),
    ("recipe give {n} minecraft:diamond_block", "No new recipes were learned"),
    # Unlocked on the first tick by `recipes/decorations/crafting_table` (a `tick` criterion).
    ("recipe give {n} minecraft:crafting_table", "No new recipes were learned"),
]


def run_steps(server, steps, label):
    for cmd, want in steps:
        line = feedback(server, cmd.format(n=NAME))
        check(f"{label}: {cmd.format(n=NAME)}", want in line, line)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "advancement-check"))
    ap.add_argument("--kiln-port", type=int, default=25587)
    ap.add_argument("--vanilla-port", type=int, default=25592)
    ap.add_argument("--skip-build", action="store_true")
    a = ap.parse_args()
    out = Path(a.out).resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    for port in (a.kiln_port, a.vanilla_port):
        if not port_free(port):
            sys.exit(f"port {port} is in use")
    if not a.skip_build and subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "kiln-server"], cwd=ROOT).returncode:
        sys.exit("build failed")
    uid = str(offline_uuid(NAME))
    lang = command_diff.english_lang(out)

    # ---- vanilla writes the files ----
    print("== vanilla", flush=True)
    world = out / "world-parent"
    shutil.copytree(WORK / "vanilla-world", world, ignore=shutil.ignore_patterns("session.lock", "logs"))
    shutil.rmtree(world / "world" / "players", ignore_errors=True)
    s = vanilla(world, a.vanilla_port, out / "vanilla-1.log")
    try:
        c = Client("127.0.0.1", a.vanilla_port, NAME)
        c.pump(5, lambda: c.pos is not None)
        c.loaded()
        c.pump(3)
        walk(c)
        run_steps(s, FIRST, "vanilla")
        c.pump(2)
        vstats = request_stats(c)
        check("vanilla answers the stats request", vstats is not None)
        c.close()
        time.sleep(2)
        s.query("save-all flush", r"Saved the game", 120)
    finally:
        s.stop()
    stats_file = world / "world" / "players" / "stats" / f"{uid}.json"
    adv_file = world / "world" / "players" / "advancements" / f"{uid}.json"
    check("vanilla wrote players/stats", stats_file.exists())
    check("vanilla wrote players/advancements", adv_file.exists())
    vanilla_stats = json.loads(stats_file.read_text(encoding="utf-8"))["stats"]
    vanilla_play_time = vanilla_stats["minecraft:custom"][PLAY_TIME]

    # ---- Kiln reads and writes them ----
    print("== Kiln", flush=True)
    exe = out / ("kiln-adv" + (".exe" if os.name == "nt" else ""))
    shutil.copy2(ROOT / "target" / "release" / ("kiln.exe" if os.name == "nt" else "kiln"), exe)
    env = dict(os.environ, KILN_PORT=str(a.kiln_port), KILN_WORLD=str(world / "world"), KILN_LANG=str(lang), RUST_LOG="info")
    k = Server([str(exe)], ROOT, env=env, log=out / "kiln.log")
    if not k.wait_for("listening on", 60):
        k.stop()
        sys.exit("Kiln did not start")
    try:
        c = Client("127.0.0.1", a.kiln_port, NAME)
        c.pump(10, lambda: c.pos is not None)
        c.loaded()
        c.pump(2)
        kstats = request_stats(c) or {}
        pt = kstats.get(("minecraft:custom", PLAY_TIME), 0)
        check("Kiln continues vanilla's play time", pt >= vanilla_play_time, f"vanilla {vanilla_play_time}, Kiln {pt}")
        walked = vanilla_stats["minecraft:custom"].get("minecraft:walk_one_cm", 0) + vanilla_stats["minecraft:custom"].get("minecraft:fly_one_cm", 0)
        kw = kstats.get(("minecraft:custom", "minecraft:walk_one_cm"), 0) + kstats.get(("minecraft:custom", "minecraft:fly_one_cm"), 0)
        check("Kiln keeps vanilla's distance", kw >= walked, f"vanilla {walked}, Kiln {kw}")
        run_steps(k, AGAIN, "Kiln on vanilla's files")
        walk(c)
        run_steps(k, SECOND, "Kiln")
        c.pump(2)
        c.close()
        time.sleep(2)
    finally:
        k.stop()
    kiln_stats = json.loads(stats_file.read_text(encoding="utf-8"))
    kiln_adv = json.loads(adv_file.read_text(encoding="utf-8"))
    check("Kiln's stats file has the data version", isinstance(kiln_stats.get("DataVersion"), int))
    check("Kiln's advancement file marks done", kiln_adv.get("minecraft:story/upgrade_tools", {}).get("done") is True)
    kiln_play_time = kiln_stats["stats"]["minecraft:custom"][PLAY_TIME]

    # ---- vanilla reads Kiln's files ----
    print("== vanilla on Kiln's files", flush=True)
    s = vanilla(world, a.vanilla_port, out / "vanilla-2.log")
    try:
        c = Client("127.0.0.1", a.vanilla_port, NAME)
        c.pump(5, lambda: c.pos is not None)
        c.loaded()
        c.pump(2)
        run_steps(s, FINAL, "vanilla on Kiln's files")
        vstats = request_stats(c) or {}
        pt = vstats.get(("minecraft:custom", PLAY_TIME), 0)
        check("vanilla continues Kiln's play time", pt >= kiln_play_time, f"Kiln {kiln_play_time}, vanilla {pt}")
        # Every recipe vanilla's book holds, to compare with the recipe files.
        s.query(f"recipe give {NAME} *", r"Unlocked|No new")
        c.close()
        time.sleep(2)
        s.query("save-all flush", r"Saved the game", 120)
    finally:
        s.stop()
    data = nbt_file(world / "world" / "players" / "data" / f"{uid}.dat")
    book = {val(t) for t in val(get(data, "recipeBook", "recipes"))}
    files = {}
    root = WORK / "generated" / "data" / "minecraft" / "recipe"
    for f in root.rglob("*.json"):
        files["minecraft:" + f.relative_to(root).as_posix()[:-5]] = json.loads(f.read_text(encoding="utf-8"))["type"]
    special = {k for k, t in files.items() if "special" in t or t in ("minecraft:brewing", "minecraft:crafting_decorated_pot")}
    extra = sorted(book & special)
    missing = sorted(set(files) - special - book)
    print(f"vanilla's book: {len(book)} recipes; special by type but in the book: {extra}; not special but missing: {missing}")
    bad = [l for l in s.lines if re.search(r"Couldn't (parse|access|read)|Failed to parse|Tried to load unrecognized recipe", l)]
    check("vanilla logs no errors reading Kiln's files", not bad, "; ".join(bad[:3]))

    passed = sum(ok for _, ok, _ in RESULTS)
    print(f"\n{passed}/{len(RESULTS)} checks passed")
    return 0 if passed == len(RESULTS) else 1


if __name__ == "__main__":
    sys.exit(main())
