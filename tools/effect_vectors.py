"""Differential tests of Kiln's mob effects, fire and air against vanilla 26.3.

1. Runs tools/EffectVectors.java: a vanilla dedicated server started in-process (in a `server`
   directory next to the output, port 25595) where a mock player is set up and ticked through
   each scenario; the player's state after every tick goes to <work>/wp9-effects/vectors.jsonl
   (the first line holds the mob effect and potion registries).
2. Runs `cargo test -p kiln-sim effect_parity` with KILN_EFFECT_VECTORS set: effect_parity
   replays each scenario through the simulation tick by tick and compares health, absorption,
   food, fire ticks, air, the active effects, attribute values, the destroy speed and the
   effect packets bit for bit.

usage: python tools/effect_vectors.py [--filter NAME] [--skip-java] [--out FILE]
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--filter", help="only scenarios whose name contains this")
    ap.add_argument("--skip-java", action="store_true", help="reuse the existing vectors")
    ap.add_argument("--out", default=str(WORK / "wp9-effects" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "EffectVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... EffectVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "EffectVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"EffectVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_EFFECT_VECTORS=str(out))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "effect_parity", "--", "--nocapture",
                              "--test-threads=1"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
