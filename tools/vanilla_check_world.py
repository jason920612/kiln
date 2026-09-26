"""Check that the vanilla 26.3 server accepts region files written by Kiln.

Copies the reference world, replaces its overworld region files with the given ones, starts
the vanilla server, force-loads the chunks of each expectation and asks vanilla
`execute if block X Y Z <block>`. Fails on any failed test or chunk loading error.

usage: python tools/vanilla_check_world.py <region-dir> X,Y,Z,block [X,Y,Z,block ...]
"""

import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"
ERRORS = ("Couldn't load chunk", "Failed to load", "Exception", "Couldn't read chunk", "corrupt")


def main():
    regions = Path(sys.argv[1])
    checks = [a.split(",") for a in sys.argv[2:]]
    base = WORK / "vanilla-check"
    if base.exists():
        shutil.rmtree(base)
    shutil.copytree(WORK / "vanilla-world", base)
    for f in (base / "session.lock", base / "world" / "session.lock"):
        f.unlink(missing_ok=True)
    rdir = base / "world" / "dimensions" / "minecraft" / "overworld" / "region"
    shutil.rmtree(rdir)
    shutil.copytree(regions, rdir)
    props = (base / "server.properties").read_text().replace("server-port=25591", "server-port=25592")
    (base / "server.properties").write_text(props)

    p = subprocess.Popen(
        ["java", "-Xmx2G", "-jar", str(WORK / "server.jar"), "--nogui"],
        cwd=base, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
    )
    lines = []
    threading.Thread(target=lambda: [lines.append(l.rstrip()) for l in p.stdout], daemon=True).start()

    def wait_for(pred, timeout):
        end = time.time() + timeout
        while time.time() < end:
            if pred():
                return True
            time.sleep(0.2)
        return False

    def cmd(c):
        p.stdin.write(c + "\n")
        p.stdin.flush()

    if not wait_for(lambda: any("Done (" in l for l in lines), 300):
        p.kill()
        sys.exit("vanilla server did not start:\n" + "\n".join(lines[-20:]))
    for x, y, z, _ in checks:
        cmd(f"forceload add {x} {z}")
    time.sleep(3)
    results = []
    for x, y, z, block in checks:
        n = len(lines)
        cmd(f"execute if block {x} {y} {z} {block}")
        wait_for(lambda: any("Test passed" in l or "Test failed" in l for l in lines[n:]), 20)
        verdict = next((l for l in lines[n:] if "Test passed" in l or "Test failed" in l), "no answer")
        results.append((f"{x},{y},{z} {block}", "Test passed" in verdict, verdict))
    cmd("stop")
    try:
        p.wait(timeout=60)
    except subprocess.TimeoutExpired:
        p.kill()
    errors = [l for l in lines if any(e in l for e in ERRORS)]
    for name, ok, verdict in results:
        print(("PASS " if ok else "FAIL ") + name + "  | " + verdict.split("]: ")[-1])
    for e in errors:
        print("ERROR", e)
    if errors or not all(ok for _, ok, _ in results):
        sys.exit(1)
    print("vanilla accepted the world")


if __name__ == "__main__":
    main()
