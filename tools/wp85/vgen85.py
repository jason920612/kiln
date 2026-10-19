"""wp85 (on the VM): generate areas with the vanilla server and its packs, then stop it.

usage: python3 tools/wp85/vgen85.py <name> <seed> "<packs>" <area>...
  area = <dimension>@<x1>,<z1>,<x2>,<z2>   block coordinates, e.g. minecraft:overworld@-128,-128,127,127
The world is ~/wp85/<name>/world (a fresh one; the packs of tools/wp85/packs named in <packs> are in its datapacks), the server
logs to ~/wp85/<name>/server.log. Every area is force-loaded in windows of 16x16 chunks (the command's limit), so the chunks are
generated to FULL and saved. Environment: PORT is fixed by sv85.sh (25596).
"""
import os
import subprocess
import sys
import time

HOME = os.path.expanduser("~")
SV = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sv85.sh")


def main():
    name, seed, packs, *areas = sys.argv[1:]
    d = os.path.join(HOME, "wp85", name)
    env = dict(os.environ, NAME=name, SEED=seed, PACKS=packs)
    subprocess.run(["bash", SV, "V", "fresh"], env=env, check=True)
    log = os.path.join(d, "server.log")

    def wait_for(text, timeout=900, since=0):
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

    at = wait_for('Done (')
    for area in areas:
        dim, rect = area.split("@")
        x1, z1, x2, z2 = (int(v) for v in rect.split(","))
        cx1, cz1, cx2, cz2 = x1 >> 4, z1 >> 4, x2 >> 4, z2 >> 4
        for wx in range(cx1, cx2 + 1, 16):
            for wz in range(cz1, cz2 + 1, 16):
                ex, ez = min(wx + 15, cx2), min(wz + 15, cz2)
                before = os.path.getsize(log)
                send(f"execute in {dim} run forceload add {wx * 16} {wz * 16} {ex * 16 + 15} {ez * 16 + 15}")
                data_end = wait_for("be force loaded", since=max(0, before - 1))
    before = os.path.getsize(log)
    send("save-all flush")
    wait_for("Saved the game", since=max(0, before - 1))
    subprocess.run(["bash", SV, "V", "stop"], env=env, check=True)
    print("done", d)


if __name__ == "__main__":
    main()
