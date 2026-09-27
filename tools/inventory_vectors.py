"""Run tools/InventoryVectors.java against the server jar and its libraries.

usage: python tools/inventory_vectors.py clicks <out.jsonl> <sequences> <seed>
       python tools/inventory_vectors.py crafting <out.jsonl> <grids per recipe> <seed>
       python tools/inventory_vectors.py sync <out.json>
       python tools/inventory_vectors.py single <out.jsonl>   (cooking and brewing lookups)
       python tools/inventory_vectors.py            (all of them, into <work>/wp3-inventory)
"""

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


def run(args):
    cmd = ["java", "-Xss8m", "-cp", classpath(), str(ROOT / "tools" / "InventoryVectors.java")] + args
    return subprocess.call(cmd, cwd=WORK)


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        out = WORK / "wp3-inventory"
        out.mkdir(exist_ok=True)
        code = run(["clicks", str(out / "clicks.jsonl"), "5000", "1"])
        code = code or run(["crafting", str(out / "crafting.jsonl"), "8", "2"])
        code = code or run(["sync", str(out / "sync.json")])
        code = code or run(["single", str(out / "single.jsonl")])
        sys.exit(code)
    sys.exit(run(args))
