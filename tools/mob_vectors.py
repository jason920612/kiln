"""Differential tests of Kiln's mobs against vanilla 26.3.

1. Runs tools/MobVectors.java: a vanilla dedicated server started in-process (in
   <out dir>/server, port $KILN_MOB_PORT or 25597) that ticks mob scenarios by hand with pinned seeds and
   records every mob's state after every tick into <work>/m6-mobs2/vectors.jsonl.
2. Runs `cargo test -p kiln-entity --test mob_parity` with KILN_MOB_VECTORS set, which replays
   each scenario in Rust and compares position, velocity, rotations, health, target, running
   goals and the ticks the player was hit.

usage: python tools/mob_vectors.py [--filter NAME] [--skip-java] [--out FILE]
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


# Scenarios whose vanilla result depends on the JVM's JIT state: the guardians' wobble calls
# `Math.sin`, which the interpreter and compiled code may round differently in the last bit, and the
# scenarios' path decisions amplify that. After a full recording's warm-up the outcome varies from run
# to run, so these are recorded again by a fresh JVM (deterministic) and replace the first recording.
COLD = ["guardian"]


def record(out, filter_):
    """Runs the vanilla harness: all scenarios (or those whose name contains `filter_`) into `out`."""
    server_dir = out.parent / "server"
    server_dir.mkdir(parents=True, exist_ok=True)
    cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "--add-opens", "java.base/java.util=ALL-UNNAMED", "-cp", classpath(),
           str(ROOT / "tools" / "MobVectors.java"), str(out)]
    if filter_:
        cmd.append(filter_)
    print("$ java ... MobVectors.java", out, filter_ or "", flush=True)
    p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
    lines = [l for l in (p.stdout + p.stderr).splitlines()
             if "MobVectors" in l or "DBG" in l or "Exception" in l or "Error" in l or "	at " in l or "error:" in l]
    print(chr(10).join(lines[-60:]))
    if p.returncode != 0:
        sys.exit(f"MobVectors failed ({p.returncode})")


def splice(out, cold):
    """Replaces the scenarios of `out` that `cold` has with the ones of `cold`."""
    import json

    fresh = {}
    for line in cold.read_text(encoding="utf-8").splitlines():
        fresh[json.loads(line)["name"]] = line
    lines = []
    for line in out.read_text(encoding="utf-8").splitlines():
        lines.append(fresh.get(json.loads(line)["name"], line))
    out.write_text(chr(10).join(lines) + chr(10), encoding="utf-8")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", help="only scenarios whose name contains this")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "m6-mobs2" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        record(out, args.filter)
        if not args.filter:
            # (see COLD) the JIT-dependent scenarios, from a JVM that has compiled nothing yet
            for name in COLD:
                cold = out.with_name(out.stem + ".cold.jsonl")
                record(cold, name)
                splice(out, cold)
                cold.unlink(missing_ok=True)
    env = dict(os.environ, KILN_MOB_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-entity", "--test", "mob_parity", "--", "--nocapture"],
                             cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
