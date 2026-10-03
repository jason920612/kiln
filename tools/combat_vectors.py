"""Differential tests of Kiln's player combat against vanilla 26.3.

1. Runs tools/CombatVectors.java: a vanilla dedicated server started in-process (in a
   `server` directory next to the output, port 25594) where mock players attack each other;
   every attack's setup and outcome goes to <work>/m6-combat/vectors.jsonl, and
   EnchantmentHelper-level vectors (modifyDamage, getDamageProtection, processDurabilityChange,
   forEachModifier, getDestroySpeed...) to enchant_helpers.jsonl beside it.
2. Runs `cargo test -p kiln-sim _parity` with KILN_COMBAT_VECTORS and KILN_ENCHANT_VECTORS set:
   combat_parity replays each scenario through the simulation (an Attack packet) and compares
   health, absorption, exhaustion, fire ticks, item and armor durability, the knockback motion
   packet and death messages; enchant_parity checks the helper vectors; riptide_parity replays
   riptide.jsonl (the 1.2 lift on release, the spin's touch check) and mount_parity mount.jsonl
   (the screens of horses, donkeys and mules); spear_parity replays spear.jsonl (stabs and
   charges of spears, `--filter spear` writes only those).

usage: python tools/combat_vectors.py [--filter NAME] [--skip-java] [--out FILE]
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
    ap.add_argument("--out", default=str(WORK / "m6-combat" / "vectors.jsonl"))
    args = ap.parse_args()
    out = Path(args.out).resolve()
    if not args.skip_java:
        server_dir = out.parent / "server"
        server_dir.mkdir(parents=True, exist_ok=True)
        cmd = ["java", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "CombatVectors.java"), str(out)]
        if args.filter:
            cmd.append(args.filter)
        print("$ java ... CombatVectors.java", out, flush=True)
        p = subprocess.run(cmd, cwd=server_dir, capture_output=True, text=True, encoding="utf-8", errors="replace")
        lines = [l for l in (p.stdout + p.stderr).splitlines()
                 if "CombatVectors" in l or "Exception" in l or "Error" in l or "\tat " in l or "error:" in l]
        print("\n".join(lines[-60:]))
        if p.returncode != 0:
            sys.exit(f"CombatVectors failed ({p.returncode})")
    env = dict(os.environ, KILN_COMBAT_VECTORS=str(out),
               KILN_ENCHANT_VECTORS=str(out.with_name("enchant_helpers.jsonl")),
               KILN_RIPTIDE_VECTORS=str(out.with_name("riptide.jsonl")),
               KILN_MOUNT_VECTORS=str(out.with_name("mount.jsonl")),
               KILN_SPEAR_VECTORS=str(out.with_name("spear.jsonl")))
    if args.filter:
        env["KILN_PARITY_FILTER"] = args.filter
    sys.exit(subprocess.call(["cargo", "test", "-p", "kiln-sim", "--lib", "_parity", "--", "--nocapture",
                              "--test-threads=1"], cwd=ROOT, env=env))


if __name__ == "__main__":
    main()
