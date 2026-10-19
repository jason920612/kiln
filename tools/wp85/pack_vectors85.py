"""wp85 (on the VM): vanilla's chunk generation with the packs of a case of tools/wp85/cases.json, dumped for
crates/kiln-worldgen/tests/packs.rs (tools/wp85/PackVectors.java, built from tools/FeatureVectors.java by make_pack_vectors.py).

usage: python3 tools/wp85/pack_vectors85.py <case|vector name> [...]   (a case runs all its vectors; output <work>/wp85/vectors)
The vanilla server of the harness binds 127.0.0.1:25597.
"""
import json
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


def run(case, vector):
    out = WORK / "wp85" / "vectors"
    out.mkdir(parents=True, exist_ok=True)
    srv = WORK / "wp85" / "pack-servers" / vector["name"]
    if srv.exists():
        shutil.rmtree(srv)
    (srv / "world" / "datapacks").mkdir(parents=True)
    for pack in case["packs"]:
        shutil.copytree(HERE / "packs" / pack, srv / "world" / "datapacks" / pack)
    (srv / "eula.txt").write_text("eula=true\n")
    props = {
        "level-seed": str(case["seed"]), "server-ip": "127.0.0.1", "server-port": "25597", "online-mode": "false",
        "enable-rcon": "false", "enable-query": "false", "generate-structures": "true", "spawn-protection": "0",
        "max-tick-time": "-1", "sync-chunk-writes": "false", "view-distance": "3", "simulation-distance": "3",
    }
    (srv / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()))
    args = [str(out), str(case["seed"]), "--dimension", vector["dimension"], "--name", vector["name"]]
    for at in vector["at"]:
        args += ["--at", at]
    cmd = ["java", "-Xmx3g", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp", classpath(), str(HERE / "PackVectors.java"), *args]
    print(f"{vector['name']}: {srv}", flush=True)
    return subprocess.call(cmd, cwd=srv)


def main():
    cases = json.load(open(HERE / "cases.json"))
    status = 0
    for want in sys.argv[1:]:
        for case in cases:
            for vector in case.get("vectors", []):
                if want in (case["name"], vector["name"]):
                    status |= run(case, vector)
    sys.exit(status)


if __name__ == "__main__":
    main()
