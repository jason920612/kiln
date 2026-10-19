"""wp87 (on the VM): vanilla's chunk generation for a seed with a chosen decoration order, dumped like tools/wp85/pack_vectors85.py does.

usage: python3 tools/wp87/order87.py NAME SEED RANK X,Z,W,H [X,Z,W,H ...] [--dimension minecraft:overworld] [--out DIR]
  RANK  the nine class ranks of the decoration order, e.g. 0,1,2,7,6,3,8,5,4 (Kiln's canonical one) or 8,7,6,5,4,3,2,1,0
  X,Z,W,H  a window of target chunks (chunk coordinates of the corner, size)
The dump is <work>/wp87/vectors/NAME.bin (the format crates/kiln-worldgen/tests/packs.rs reads). The harness' server binds 127.0.0.1:25598.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


def main():
    args = sys.argv[1:]
    dimension = "minecraft:overworld"
    out = WORK / "wp87" / "vectors"
    if "--dimension" in args:
        i = args.index("--dimension")
        dimension = args[i + 1]
        del args[i:i + 2]
    if "--out" in args:
        i = args.index("--out")
        out = Path(args[i + 1])
        del args[i:i + 2]
    name, seed, rank, windows = args[0], args[1], args[2], args[3:]
    out.mkdir(parents=True, exist_ok=True)
    srv = WORK / "wp87" / "servers" / name
    if srv.exists():
        shutil.rmtree(srv)
    srv.mkdir(parents=True)
    (srv / "eula.txt").write_text("eula=true\n")
    props = {
        "level-seed": seed, "server-ip": "127.0.0.1", "server-port": "25598", "online-mode": "false",
        "enable-rcon": "false", "enable-query": "false", "generate-structures": "true", "spawn-protection": "0",
        "max-tick-time": "-1", "sync-chunk-writes": "false", "view-distance": "3", "simulation-distance": "3",
    }
    (srv / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()))
    cmd = ["java", "-Xmx3g", f"-Dwp87.rank={rank}", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(), str(HERE / "OrderVectors.java"),
           str(out), seed, "--dimension", dimension, "--name", name]
    for w in windows:
        cmd += ["--at", w]
    print(" ".join(cmd[:6]), "...", flush=True)
    return subprocess.call(cmd, cwd=srv)


if __name__ == "__main__":
    sys.exit(main())
