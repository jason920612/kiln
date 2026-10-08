"""Parity audit: which vanilla block classes with behaviour does Kiln mention at all?

1. Runs tools/AuditBlockBehaviour.java (reflection over the vanilla 26.3 block registry: for
   every block the hooks its class chain overrides, random ticking, block entity).
2. Tokenises the hand-written Rust sources (no generated tables, no worldgen/protocol) and
   counts how often each Java class name, and each block's id (as CONSTANT or word), occurs.
3. Prints one line per vanilla block class, behaviour-bearing ones first. A class whose own
   name and ancestors never occur in the sources, and none of whose blocks' ids occur, has no
   Kiln behaviour: "NONE". It is a first filter (a class can be mentioned and still be partial),
   the audit in docs/parity-coverage.md judges the rest by hand.

usage: python tools/parity_audit.py [--skip-java] [--out FILE]
"""

import argparse
import collections
import os
import re
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402

# Hooks that make a block do something a player can observe on the server side.
SERVER_HOOKS = {
    "randomTick", "tick", "neighborChanged", "useWithoutItem", "useItemOn", "entityInside", "stepOn", "fallOn",
    "triggerEvent", "getAnalogOutputSignal", "getSignal", "getDirectSignal", "attack", "onProjectileHit",
    "onExplosionHit", "handlePrecipitation", "performBonemeal", "playerDestroy", "playerWillDestroy",
    "affectNeighborsAfterRemoval", "onPlace", "setPlacedBy", "updateEntityMovementAfterFallOn", "getTicker",
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-java", action="store_true")
    ap.add_argument("--out", default=str(WORK / "wp44" / "blocks_audit.tsv"))
    args = ap.parse_args()
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    if not args.skip_java:
        cmd = ["java", "-cp", classpath(), str(ROOT / "tools" / "AuditBlockBehaviour.java"), str(out)]
        p = subprocess.run(cmd, cwd=WORK, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if p.returncode != 0:
            sys.exit(p.stdout + p.stderr)

    tokens = collections.Counter()
    for p in (ROOT / "crates").rglob("*.rs"):
        sp = p.as_posix()
        if "/gen/" in sp or "/target/" in sp or "kiln-worldgen" in sp or "kiln-proto" in sp or "kiln-net" in sp:
            continue
        tokens.update(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", p.read_text(encoding="utf-8", errors="replace")))

    classes = collections.OrderedDict()
    for line in out.read_text(encoding="utf-8").splitlines():
        id, chain, hooks, flags = (line.split("\t") + ["", "", "", ""])[:4]
        ch = chain.split(">")
        c = classes.setdefault(ch[0], {"blocks": [], "chain": ch, "hooks": set(), "flags": set()})
        c["blocks"].append(id.replace("minecraft:", ""))
        c["hooks"].update(h for h in hooks.split(",") if h)
        c["flags"].update(flags.split())

    rows = []
    for own, c in classes.items():
        hooks = c["hooks"] & SERVER_HOOKS
        mentions = {k: tokens[k] for k in c["chain"] if tokens[k]}
        id_hits = [b for b in c["blocks"] if tokens[b.upper()] or tokens[b]]
        status = "mentioned" if (mentions or id_hits) else "NONE"
        rows.append((status, own, len(c["blocks"]), sorted(hooks), sorted(c["flags"]), mentions, len(id_hits), c["blocks"][0]))
    rows.sort(key=lambda r: (r[0] != "NONE", -len(r[3]), r[1]))
    for status, own, n, hooks, flags, mentions, idh, first in rows:
        print(f"{status}\t{own}\t{n} blocks (e.g. {first})\t{' '.join(flags)}\t{','.join(hooks)}\tclass mentions: {mentions}\tids mentioned: {idh}")


if __name__ == "__main__":
    main()
