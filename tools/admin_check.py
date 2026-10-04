"""Administration state across the two servers: the vanilla 26.3 server and Kiln load each
other's saves with the same seed, game rules, command storage, operators and difficulty.

1. Vanilla creates a world (a negative seed, hard difficulty), a game rule of each type is
   changed, command storage is written (two namespaces), a player is opped and the difficulty
   changed; every query is answered and the server stopped.
2. Kiln loads that save (KILN_GENERATOR=noise too, which must take the save's seed): the same
   queries must answer the same; then Kiln changes rules, storage, operators and difficulty and
   stops (saving).
3. Vanilla loads what Kiln saved: the queries must show Kiln's changes, and ops.json lists
   Kiln's operator.

Needs KILN_WORK (default <repo>/work) with server.jar, versions/26.3 and generated/, and the
release build of kiln-server (cargo build --release -p kiln-server).

usage: python tools/admin_check.py [--kiln-exe PATH] [--out DIR] [--vanilla-port 25591] [--kiln-port 25592]
"""

import argparse
import json
import os
import re
import shutil
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
os.environ["KILN_WORK"] = str(WORK)
sys.path.insert(0, str(ROOT / "tools"))
sys.dont_write_bytecode = True
import command_diff as cd  # noqa: E402

SEED = -777000111222
# Queries both servers answer, in the order asked. Output must be identical.
QUERIES = [
    "seed",
    "gamerule keep_inventory",
    "gamerule random_tick_speed",
    "gamerule respawn_radius",
    "gamerule max_entity_cramming",
    "gamerule pvp",
    "data get storage kiln:t n",
    "data get storage minecraft:plain list",
    "data get storage minecraft:plain list[1]",
]

RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  | {detail}" if detail else ""), flush=True)
    return ok


def run(server, command, seq=[0]):
    seq[0] += 1
    return server.run(command, f"zqsync{seq[0]}")


def vanilla_props(base: Path, port: int, seed):
    props = [
        f"server-port={port}", "online-mode=false", "white-list=false", "enforce-secure-profile=false",
        "generate-structures=false", "difficulty=hard", "spawn-protection=0", "view-distance=4",
        "simulation-distance=4", "max-tick-time=-1", "sync-chunk-writes=false", "enable-rcon=false",
        "enable-query=false",
    ]
    if seed is not None:
        props.append(f"level-seed={seed}")
    (base / "server.properties").write_text("\n".join(props) + "\n", encoding="utf-8")
    (base / "eula.txt").write_text("eula=true\n", encoding="utf-8")


def start_vanilla(base: Path, port: int, log: Path):
    argv = ["java", "-Xmx2G", "-Dstdout.encoding=UTF-8", "-Dfile.encoding=UTF-8", "-jar", str(WORK / "server.jar"), "--nogui"]
    (base / "world" / "session.lock").unlink(missing_ok=True)
    s = cd.Server("vanilla", argv, base, os.environ.copy(), cd.VANILLA_LINE, log)
    if not s.wait_for("Done (", 300):
        s.stop()
        sys.exit("vanilla did not start: " + log.read_text(encoding="utf-8", errors="replace")[-2000:])
    return s


def start_kiln(base: Path, port: int, exe: Path, lang: Path, log: Path, noise: bool):
    env = os.environ.copy()
    env.update({"KILN_PORT": str(port), "KILN_LANG": str(lang), "RUST_LOG": "info", "NO_COLOR": "1",
                "KILN_WORLD": str(base / "world"), "KILN_DATAPACK": str(WORK / "generated")})
    for k in ("KILN_OPS", "KILN_SEED", "KILN_DIFFICULTY", "KILN_GENERATOR"):
        env.pop(k, None)
    if noise:
        env["KILN_GENERATOR"] = "noise"
    copy = base / "kiln-admin.exe"
    shutil.copy2(exe, copy)
    s = cd.Server("kiln", [str(copy)], base, env, cd.KILN_LINE, log)
    if not s.wait_for("listening on", 120):
        s.stop()
        sys.exit("kiln did not start: " + log.read_text(encoding="utf-8", errors="replace")[-2000:])
    return s


def answers(server):
    return {q: run(server, q) for q in QUERIES}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--kiln-exe", default=str(ROOT / "target" / "release" / "kiln.exe"))
    ap.add_argument("--out", default=str(WORK / "wp44" / "wp44-admin" / "admin-check"))
    ap.add_argument("--vanilla-port", type=int, default=25591)
    ap.add_argument("--kiln-port", type=int, default=25592)
    a = ap.parse_args()
    for port in (a.vanilla_port, a.kiln_port):
        if not cd.port_free(port):
            sys.exit(f"port {port} is in use")
    out = Path(a.out)
    if out.exists():
        shutil.rmtree(out)
    base = out / "server"
    base.mkdir(parents=True)
    lang = cd.english_lang(out)
    exe = Path(a.kiln_exe)

    # 1. vanilla writes a world.
    vanilla_props(base, a.vanilla_port, SEED)
    v = start_vanilla(base, a.vanilla_port, out / "vanilla-1.log")
    for c in ["gamerule keep_inventory true", "gamerule random_tick_speed 7", "gamerule respawn_radius 3",
              "difficulty normal", "data modify storage kiln:t n set value 5",
              "data modify storage minecraft:plain list set value [1,2,3]", "op Alice"]:
        run(v, c)
    first = answers(v)
    vanilla_difficulty = run(v, "difficulty")
    v.stop()
    time.sleep(1)
    check("vanilla saved the seed", f"{SEED}" in "".join(first["seed"]), str(first["seed"]))
    gen = base / "world" / "data" / "minecraft" / "world_gen_settings.dat"
    check("world_gen_settings.dat exists", gen.exists())
    kiln_dir = out / "kiln-world-base"
    shutil.copytree(base, kiln_dir, ignore=shutil.ignore_patterns("session.lock", "logs", "*.exe"))

    # 2. Kiln loads it.
    k = start_kiln(kiln_dir, a.kiln_port, exe, lang, out / "kiln-1.log", noise=True)
    gen_line = next((l for l in k.all if "generation: seed" in l), "")
    check("KILN_GENERATOR=noise takes the save's seed", f"seed {SEED}" in gen_line, gen_line.strip()[-90:])
    second = answers(k)
    for q in QUERIES:
        check(f"kiln loads the vanilla save: {q}", second[q] == first[q], f"vanilla={first[q]} kiln={second[q]}")
    kiln_difficulty = run(k, "difficulty")
    check("kiln keeps the saved difficulty (normal)", kiln_difficulty == vanilla_difficulty, f"vanilla={vanilla_difficulty} kiln={kiln_difficulty}")
    ops = json.loads((kiln_dir / "ops.json").read_text(encoding="utf-8"))
    names = {o["name"].lower() for o in ops}
    check("ops.json from vanilla lists alice", "alice" in names, str(ops))
    # Kiln changes what the operator can change.
    for c in ["gamerule keep_inventory false", "gamerule max_entity_cramming 9", "gamerule pvp false",
              "difficulty peaceful", "data modify storage kiln:t n set value 6",
              "data modify storage minecraft:plain list append value 4", "op Bob", "deop Alice"]:
        run(k, c)
    third = answers(k)
    k.stop()
    time.sleep(1)
    ops = json.loads((kiln_dir / "ops.json").read_text(encoding="utf-8"))
    names = {o["name"].lower(): o for o in ops}
    check("ops.json after /op and /deop", "bob" in names and "alice" not in names and names["bob"]["level"] == 4, str(ops))
    check("ops.json entries have vanilla's fields", all(set(o) == {"uuid", "name", "level", "bypassesPlayerLimit"} for o in ops), str(ops))

    # 3. vanilla loads what Kiln saved.
    vanilla_props(kiln_dir, a.vanilla_port, None)
    v = start_vanilla(kiln_dir, a.vanilla_port, out / "vanilla-2.log")
    fourth = answers(v)
    for q in QUERIES:
        check(f"vanilla loads the Kiln save: {q}", fourth[q] == third[q], f"kiln={third[q]} vanilla={fourth[q]}")
    already = run(v, "op Bob")
    fresh = run(v, "op Carol")
    check("vanilla reads Kiln's ops.json", already and "Nothing changed" in already[0] and fresh and "Made" in fresh[0], f"{already} {fresh}")
    bad = [l for l in v.all if "ERROR" in l or "Failed to" in l or "Exception" in l]
    check("vanilla logs no errors loading it", not bad, "; ".join(bad[:3]))
    v.stop()

    failed = [n for n, ok, _ in RESULTS if not ok]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
