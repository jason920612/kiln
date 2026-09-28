"""Run tools/FeatureVectors.java (vanilla FEATURES in Kiln's canonical order) for some seeds.

Each seed gets a throwaway vanilla server directory under <work>/wp4-features/vanilla/ (the
harness starts a dedicated server in-process for its generation context; it binds a free
port in 25591-25593 on 127.0.0.1). Dumps (Mojang-derived: never commit) go to
<work>/wp4-features/vectors unless --out is given.

usage: python tools/feature_vectors.py [--out DIR] [--structures] [--regions N] [--size S] [--heights N] [--near SET]
                                       [--check] [--bench N] [--dimension overworld|nether|end] [seed...]
       (default seeds 0 1 12345 -4172144997902289642 and one random seed)
"""

import argparse
import os
import random
import shutil
import socket
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


def free_port():
    for port in (25591, 25592, 25593):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
            try:
                s.bind(("127.0.0.1", port))
                return port
            except OSError:
                continue
    sys.exit("no free port in 25591-25593")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "wp4-features" / "vectors"))
    ap.add_argument("--structures", action="store_true")
    ap.add_argument("--regions", type=int)
    ap.add_argument("--size", type=int)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--bench", type=int)
    ap.add_argument("--heights", type=int)
    ap.add_argument("--dimension", default="overworld", choices=["overworld", "nether", "end"])
    ap.add_argument("--near", help="center regions on placement chunks of this structure set (random spread)")
    ap.add_argument("seeds", nargs="*")
    a = ap.parse_args()
    seeds = a.seeds or ["0", "1", "12345", "-4172144997902289642", "random"]
    seeds = [str(random.getrandbits(63) - (1 << 62)) if s == "random" else s for s in seeds]
    status = 0
    for seed in seeds:
        run = WORK / "wp4-features" / "vanilla" / (("" if a.dimension == "overworld" else a.dimension + "_") + seed + ("_s" if a.structures else "") + ("_" + a.near.split(":")[-1] if a.near else ""))
        if run.exists():
            shutil.rmtree(run)
        run.mkdir(parents=True)
        (run / "eula.txt").write_text("eula=true\n", encoding="utf-8")
        props = {
            "level-seed": seed,
            "server-ip": "127.0.0.1",
            "server-port": str(free_port()),
            "online-mode": "false",
            "enable-rcon": "false",
            "enable-query": "false",
            "generate-structures": "true" if a.structures else "false",
            "spawn-protection": "0",
            "max-tick-time": "-1",
            "sync-chunk-writes": "false",
            "view-distance": "3",
            "simulation-distance": "3",
        }
        (run / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()), encoding="utf-8")
        args = [str(Path(a.out).resolve()), seed]
        if a.regions is not None:
            args += ["--regions", str(a.regions)]
        if a.size is not None:
            args += ["--size", str(a.size)]
        if a.check:
            args.append("--check")
        if a.bench:
            args += ["--bench", str(a.bench)]
        if a.heights:
            args += ["--heights", str(a.heights)]
        if a.near:
            args += ["--near", a.near]
        if a.dimension != "overworld":
            args += ["--dimension", a.dimension]
        cmd = ["java", "-Xmx10g", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(),
               str(ROOT / "tools" / "FeatureVectors.java"), *args]
        print(f"seed {seed}: {run}", flush=True)
        status |= subprocess.call(cmd, cwd=run)
    sys.exit(status)


if __name__ == "__main__":
    main()
