"""wp66: console-only driver for the two servers (no client): batch commands through the fifo, read the log back up to a marker.
TARGET=K (Kiln, ~/wp66) or V (vanilla, ~/wp66/vanilla)."""
import os, re, time
from pathlib import Path

HOME = Path.home()
T = os.environ.get("TARGET", "K")
DIR = HOME / "wp66" / ("vanilla" if T == "V" else "")
LOG = DIR / "server.log"
ANSI = re.compile(r"\x1b\[[0-9;]*m")
_n = [0]


def send(cmds):
    with open(DIR / "console", "w") as f:
        f.write("\n".join(cmds) + "\n")


def run(cmds, timeout=600, quiet_for=0.0):
    """Send the commands, then a marker; return the log lines (ANSI and timestamps stripped, no periodic reports) between."""
    _n[0] += 1
    mark = f"@@wp66m{_n[0]}x{int(time.time()*1000)%100000}"
    pos = LOG.stat().st_size
    # a long batch in one write can overflow the fifo while the server is busy: send in chunks
    for i in range(0, len(cmds), 200):
        send(cmds[i:i + 200])
    send([f"say {mark}"])
    t0 = time.time()
    while time.time() - t0 < timeout:
        with open(LOG, "rb") as f:
            f.seek(pos)
            txt = f.read().decode(errors="replace")
        if mark in txt:
            break
        time.sleep(0.05)
    else:
        raise RuntimeError("timeout waiting for the marker; last: " + txt[-300:])
    out = []
    for l in txt.splitlines():
        l = ANSI.sub("", l)
        if "chunk totals" in l or "mspt mean" in l or mark in l:
            continue
        l = re.sub(r"^\S+Z\s+\S+\s+\S+\s+", "", l) if T == "K" else re.sub(r"^\[[0-9:]+\] \[[^\]]*\]: ", "", l)
        out.append(l)
    return out


def counts(cmds, **kw):
    """Run fill-like commands; return the list of changed-block counts (0 for 'no blocks'); raise on unloaded/out-of-world errors."""
    res = []
    for l in run(cmds, **kw):
        m = re.search(r"Successfully filled (\d+)|commands\.fill\.success\[(\d+)\]|Successfully cloned (\d+)|commands\.clone\.success\[(\d+)\]", l)
        if m:
            res.append(int(next(g for g in m.groups() if g)))
        elif re.search(r"No blocks were|commands\.(fill|clone)\.failed", l):
            res.append(0)
        elif re.search(r"unloaded|outside|toobig|argument\.pos|Unknown|Incorrect|Could not|Expected", l):
            raise RuntimeError("command error: " + l)
    return res


def ready(x, z, tries=60):
    """Wait until the block at x,z can be set (the chunk is loaded)."""
    for _ in range(tries):
        out = run([f"setblock {x} 100 {z} minecraft:stone"])
        if not any(re.search(r"unloaded|argument\.pos", l) for l in out):
            run([f"setblock {x} 100 {z} minecraft:air"])
            return True
        time.sleep(0.5)
    return False


def gametime():
    for l in run(["time query gametime"]):
        m = re.search(r"time is (\d+)|commands\.time\.query\.gametime\[(\d+)\]", l)
        if m:
            return int(m.group(1) or m.group(2))
    raise RuntimeError("no gametime")


def step(n, timeout=3600):
    """`tick step n` on a frozen game and wait until the game time has advanced by n."""
    g0 = gametime()
    run([f"tick step {n}"])
    t0 = time.time()
    while time.time() - t0 < timeout:
        g = gametime()
        if g >= g0 + n:
            return g - g0
        time.sleep(0.05 if n < 200 else 0.5)
    raise RuntimeError("step timeout")


def advance(n, timeout=7200):
    """n ticks as fast as the server goes: `tick sprint`, then freeze again."""
    if n <= 40:
        return step(n)
    g0 = gametime()
    run([f"tick sprint {n}"])
    t0 = time.time()
    while time.time() - t0 < timeout:
        g = gametime()
        if g >= g0 + n:
            break
        time.sleep(0.5)
    else:
        raise RuntimeError("sprint timeout")
    time.sleep(0.3)
    out = run(["tick query"])
    if not any("frozen" in l for l in out):
        run(["tick freeze"])
    return gametime() - g0


def day_time():
    """The overworld day timeline's tick (`time query day`)."""
    for l in run(["time query day"]):
        m = re.search(r"at (\d+) tick|commands\.time\.query\.[a-z_.]*\[[^\]]*?(\d+)\]", l)
        if m:
            return int(m.group(1) or m.group(2))
    raise RuntimeError("no day time")
