"""wp85 (on the VM): generate the areas of a case of tools/wp85/cases.json with the vanilla server and its packs, then stop it.

usage: python3 tools/wp85/vgen85.py <case> [suffix]
  The world is ~/wp85/<case><suffix>/world (a fresh one; the case's packs of tools/wp85/packs are in its datapacks), the server logs
  to ~/wp85/<case><suffix>/server.log. A second run with suffix `_b` gives the baseline: vanilla against itself (features that span
  chunks are not deterministic in vanilla). Every area is force-loaded in windows of 16x16 chunks (the command's limit), so the
  chunks are generated to FULL and saved. The vanilla server listens on 25596 (sv85.sh).
"""
import json
import os
import subprocess
import sys
import time

HOME = os.path.expanduser("~")
HERE = os.path.dirname(os.path.abspath(__file__))
SV = os.path.join(HERE, "sv85.sh")


def main():
    case_name = sys.argv[1]
    suffix = sys.argv[2] if len(sys.argv) > 2 else ""
    case = next(c for c in json.load(open(os.path.join(HERE, "cases.json"))) if c["name"] == case_name)
    name = case_name + suffix
    d = os.path.join(HOME, "wp85", name)
    env = dict(os.environ, NAME=name, SEED=str(case["seed"]), PACKS=" ".join(case["packs"]))
    subprocess.run(["bash", SV, "V", "fresh"], env=env, check=True)
    log = os.path.join(d, "server.log")

    def wait_for(text, since=0, timeout=1800):
        end = time.time() + timeout
        while time.time() < end:
            data = open(log, encoding="utf-8", errors="replace").read()
            at = data.find(text, since)
            if at >= 0:
                return at + len(text)
            time.sleep(0.5)
        raise SystemExit(f"timeout waiting for {text!r}; see {log}")

    def send(cmd):
        with open(os.path.join(d, "console"), "w") as f:
            f.write(cmd + "\n")

    def mark():
        return len(open(log, encoding="utf-8", errors="replace").read())

    wait_for("Done (")
    for dim, x1, z1, x2, z2 in case["areas"]:
        cx1, cz1, cx2, cz2 = x1 >> 4, z1 >> 4, x2 >> 4, z2 >> 4
        for wx in range(cx1, cx2 + 1, 16):
            for wz in range(cz1, cz2 + 1, 16):
                ex, ez = min(wx + 15, cx2), min(wz + 15, cz2)
                at = mark()
                send(f"execute in {dim} run forceload add {wx * 16} {wz * 16} {ex * 16 + 15} {ez * 16 + 15}")
                wait_for("be force loaded", since=at)
    at = mark()
    send("save-all flush")
    wait_for("Saved the game", since=at)
    subprocess.run(["bash", SV, "V", "stop"], env=env, check=True)
    print("done", d)


if __name__ == "__main__":
    main()
