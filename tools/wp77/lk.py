"""wp77: drive Kiln (K) and the vanilla reference server (V) from one process (consoles are fifos in ~/wp77/<K|V>, logs server.log).
Kiln steps at the tick rate (a step of n ticks takes n * 50 ms), vanilla at full speed: wait for the slower one."""
import os, re, time
HOME = os.path.expanduser("~")
D = {"K": HOME + "/wp77/K", "V": HOME + "/wp77/V"}
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def w(t, cmd):
    with open(D[t] + "/console", "w") as f:
        f.write(cmd + "\n")


def both(cmd):
    w("K", cmd)
    w("V", cmd)


def logpos(t):
    return os.stat(D[t] + "/server.log").st_size


def since(t, n):
    with open(D[t] + "/server.log", "rb") as f:
        f.seek(n)
        return ANSI.sub("", f.read().decode(errors="replace"))


def ask(t, cmd, wait=0.3):
    n = logpos(t)
    w(t, cmd)
    time.sleep(wait)
    return since(t, n)


def ask_both(cmd, wait=0.3):
    n = {t: logpos(t) for t in "KV"}
    both(cmd)
    time.sleep(wait)
    return {t: since(t, n[t]) for t in "KV"}


def gametime(t):
    out = ask(t, "time query gametime")
    m = re.search(r"gametime\[(\d+)\]|game time is (\d+)", out)
    return int(m.group(1) or m.group(2))


def step(n=1):
    both(f"tick step {n}")
    time.sleep(0.3 + 0.055 * n)


def sprint(t, n):
    pos = logpos(t)
    w(t, f"tick sprint {n}")
    t0 = time.time()
    while time.time() - t0 < 3000:
        time.sleep(1)
        txt = since(t, pos)
        if "sprint.report" in txt or "Sprint completed" in txt or "sprint" in txt and "ticks" in txt:
            return
    raise RuntimeError("sprint did not finish on " + t)


def wait_log(t, text, timeout=120, start=0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if text in since(t, start):
            return True
        time.sleep(0.5)
    return False
