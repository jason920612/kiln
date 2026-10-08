"""Runs every parity suite that replays vanilla-recorded data and prints one summary line each.

The recorded vectors live in the (gitignored) work directory: set KILN_WORK to it (default
<repo>/work). The datapack is KILN_DATAPACK (default <KILN_WORK>/generated). Logs go to
--out (default <KILN_WORK>/wp44/suites). Each suite is `cargo test --release` with the
environment variables its test reads; the pass count printed is cargo's own "N passed", plus
the line each parity test prints about its own comparison (rounds, scenarios, chunks, ...).

usage: python tools/parity_suites.py [--only NAME,...] [--out DIR] [--skip-workspace]
"""

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()


def vec(rel):
    return str(WORK / rel)


# name: (cargo args, env). Vector paths are relative to WORK.
SUITES = {
    "mob_parity": (["-p", "kiln-entity", "--test", "mob_parity"], {"KILN_MOB_VECTORS": vec("m6-mobs2/vectors.jsonl")}),
    "mob_spear": (["-p", "kiln-entity", "--test", "mob_parity"], {"KILN_MOB_VECTORS": vec("wp34/mob_spear.jsonl")}),
    "mob_wp36_kills": (["-p", "kiln-entity", "--test", "mob_parity"], {"KILN_MOB_VECTORS": vec("wp36/mob_kills.jsonl")}),
    "mob_wp36_ench": (["-p", "kiln-entity", "--test", "mob_parity"], {"KILN_MOB_VECTORS": vec("wp36/mob_ench.jsonl")}),
    "finalize_parity": (["-p", "kiln-entity", "--test", "finalize_parity"], {"KILN_FINALIZE_VECTORS": vec("wp33/finalize.jsonl"), "KILN_FINALIZE_HARD_VECTORS": vec("wp34/finalize_hard.jsonl")}),
    "entity_parity": (["-p", "kiln-entity", "--test", "parity"], {"KILN_ENTITY_VECTORS": vec("wp4-entities/vectors.jsonl")}),
    "fire": (["-p", "kiln-blocks", "--lib", "fire_parity"], {"KILN_FIRE_VECTORS": vec("wp-fire/vectors.jsonl")}),
    "combat": (["-p", "kiln-sim", "--lib", "parity"], {
        "KILN_COMBAT_VECTORS": vec("wp34/combat/vectors.jsonl"),
        "KILN_RIPTIDE_VECTORS": vec("wp34/combat/riptide.jsonl"),
        "KILN_MOUNT_VECTORS": vec("wp34/combat/mount.jsonl"),
        "KILN_ENCHANT_VECTORS": vec("wp34/combat/enchant_helpers.jsonl"),
        "KILN_SPEAR_VECTORS": vec("wp34/combat/spear.jsonl"),
    }),
    "effects": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_EFFECT_VECTORS": vec("wp9-effects/vectors.jsonl")}),
    "containers": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_CONTAINER_VECTORS": vec("wp15-containers/vectors.jsonl")}),
    "jukebox": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_CONTAINER_VECTORS": vec("wp36/containers/vectors.jsonl")}),
    "sculk": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_SCULK_VECTORS": vec("m6s3-warden/sculk_vectors.jsonl")}),
    "weather": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_WEATHER_VECTORS": vec("wx-weather/vectors.jsonl")}),
    "itemuse": (["-p", "kiln-sim", "--lib", "parity"], {"KILN_ITEMUSE_VECTORS": vec("wp-itemuse/vectors.jsonl")}),
    "inventory": (["-p", "kiln-inventory"], {"KILN_PARITY": "1"}),
    "item_corpus": (["-p", "kiln-item"], {}),
    "loot": (["-p", "kiln-loot"], {"KILN_PARITY": "1"}),
    "worldgen": (["-p", "kiln-worldgen"], {"KILN_PARITY": "1"}),
    "storage": (["-p", "kiln-storage"], {}),
    "proto": (["-p", "kiln-proto"], {}),
    "command": (["-p", "kiln-command"], {}),
    "region_stacks": (["-p", "kiln-sim", "--test", "region_stacks"], {}),
    "determinism": (["-p", "kiln-sim", "--test", "determinism"], {}),
}

SUMMARY = re.compile(r"(test result:|skipped|parity|match|scenarios|vectors|rounds|chunks|sequences|cases|agree|mismatch|diverg)", re.I)


def run(name, cargo_args, env_extra, out, with_data):
    env = dict(os.environ)
    env["KILN_WORK"] = str(WORK)
    if with_data:
        env.setdefault("KILN_DATAPACK", str(WORK / "generated"))
    env.update(env_extra)
    cmd = ["cargo", "test", "--release", *cargo_args, "--", "--nocapture", "--test-threads=4"]
    t = time.time()
    p = subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
    log = out / f"{name}.log"
    log.write_text(p.stdout + "\n--- stderr ---\n" + p.stderr, encoding="utf-8")
    keep = [l.strip() for l in (p.stdout + p.stderr).splitlines() if SUMMARY.search(l) and "Compiling" not in l and "warning" not in l]
    print(f"== {name}: exit {p.returncode} in {time.time() - t:.0f}s")
    for l in keep[-14:]:
        print("   ", l[:220])
    sys.stdout.flush()
    return p.returncode


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--only", help="comma-separated suite names")
    ap.add_argument("--out", default=str(WORK / "wp44" / "suites"))
    ap.add_argument("--skip-workspace", action="store_true")
    ap.add_argument("--no-data", action="store_true", help="do not set KILN_DATAPACK")
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    names = args.only.split(",") if args.only else list(SUITES)
    bad = 0
    for n in names:
        cargo_args, env = SUITES[n]
        bad += run(n, cargo_args, env, out, not args.no_data) != 0
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
