"""wp87 (on the VM): vanilla's chunk generation for the superflat dimensions of tools/wp87/packs/kiln87_flat, dumped for crates/kiln-worldgen/tests/wp87_flat.rs
(tools/wp87/FlatVectors.java, built by make_flat_vectors.py).

usage: python3 tools/wp87/flat87.py [case ...]   (cases: feat, village, lakes, all; output <work>/wp87/vectors/pack_flat_<case>.bin)
The vanilla server of the harness binds 127.0.0.1:25597.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
WORK = Path(os.environ.get("KILN_WORK") or HERE.parent.parent / "work").resolve()
SEED = 87
CASES = {
    "feat": ("kiln87:flatfeat", ["-4,-4,8,8", "40,-30,8,8"]),
    "village": ("kiln87:flatvillage", ["-20,-20,16,16", "60,60,16,16"]),
    "lakes": ("kiln87:flatlakes", ["-4,-4,10,10", "-60,20,10,10"]),
    "all": ("kiln87:flatall", ["-30,-30,14,14", "30,-70,14,14"]),
}


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


def run(name):
    dim, windows = CASES[name]
    out = WORK / "wp87" / "vectors"
    out.mkdir(parents=True, exist_ok=True)
    srv = WORK / "wp87" / "flat-servers" / name
    if srv.exists():
        shutil.rmtree(srv)
    (srv / "world" / "datapacks").mkdir(parents=True)
    shutil.copytree(HERE / "packs" / "kiln87_flat", srv / "world" / "datapacks" / "kiln87_flat")
    (srv / "eula.txt").write_text("eula=true\n")
    props = {
        "level-seed": str(SEED), "server-ip": "127.0.0.1", "server-port": "25597", "online-mode": "false",
        "enable-rcon": "false", "enable-query": "false", "generate-structures": "true", "spawn-protection": "0",
        "max-tick-time": "-1", "sync-chunk-writes": "false", "view-distance": "3", "simulation-distance": "3",
    }
    (srv / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()))
    args = [str(out), str(SEED), "--dimension", dim, "--name", f"flat_{name}"]
    for w in windows:
        args += ["--at", w]
    cmd = ["java", "-Xmx3g", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(), str(HERE / "FlatVectors.java"), *args]
    print(f"{name}: {srv}", flush=True)
    return subprocess.call(cmd, cwd=srv)


if __name__ == "__main__":
    status = 0
    for n in (sys.argv[1:] or list(CASES)):
        status |= run(n)
    sys.exit(status)
