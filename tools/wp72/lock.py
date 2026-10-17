"""wp72 helpers for scripts that drive Kiln and vanilla from ONE process in lock step (`tick freeze` + `tick step`): console writes to both
servers, answers parsed from both logs (Kiln logs translation keys, vanilla English), game time alignment and the two real clients.
Kiln steps at the tick rate (a step of n ticks takes n * 50 ms), vanilla at full speed: wait for the slower one."""
import json, re, time, os
import p61
from p61 import *
from ctl import C, PLAY

p61.VDIR = HOME / "wp72" / "vanilla"
LOGS = {"K": PLAY / "server.log", "V": p61.VDIR / "server.log"}
CONS = {"K": PLAY / "console", "V": p61.VDIR / "console"}
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def clients():
    return {"K": C(25999), "V": C(26000)}


def w(t, cmd):
    with open(CONS[t], "w") as f:
        f.write(cmd + "\n")


def both_(cmd):
    w("K", cmd)
    w("V", cmd)


def logpos(t):
    return LOGS[t].stat().st_size


def since(t, n):
    with open(LOGS[t], "rb") as f:
        f.seek(n)
        return ANSI_RE.sub("", f.read().decode(errors="replace"))


def ask(t, cmd, wait=0.3):
    n = logpos(t)
    w(t, cmd)
    time.sleep(wait)
    return since(t, n)


def ask_both(cmd, wait=0.3):
    n = {t: logpos(t) for t in "KV"}
    both_(cmd)
    time.sleep(wait)
    return {t: since(t, n[t]) for t in "KV"}


def gametime(t):
    out = ask(t, "time query gametime")
    m = re.search(r"gametime\[(\d+)\]|game time is (\d+)", out)
    return int(m.group(1) or m.group(2))


def step(n=1):
    both_(f"tick step {n}")
    time.sleep(0.3 + 0.055 * n)


def align(mod, value):
    """Step both servers so that the NEXT tick has game time == value (mod `mod`)."""
    for t in "KV":
        need = (value - 1 - gametime(t)) % mod
        if need:
            w(t, f"tick step {need}")
    time.sleep(0.4 + 0.055 * mod)
    return {t: gametime(t) for t in "KV"}


def setup(frozen=True, rules=()):
    w("V", "op KilnTest")
    for g in ("random_tick_speed 0", "spawn_mobs false", "mob_griefing false", "do_weather_cycle false", "do_daylight_cycle false") + tuple(rules):
        both_("gamerule " + g)
    both_("forceload add 0 0 60 60")
    time.sleep(2)
    if frozen:
        both_("tick freeze")


NUM = re.compile(r"-?\d+(?:\.\d+)?")


def data_get(t, what, path=""):
    """`data get <what> <path>`: the raw answer text of server t."""
    return ask(t, f"data get {what} {path}".strip())


def respawn_all():
    """Click the respawn button on any client that shows the death screen (the actor's window is 960x540, the second client's 480x270)."""
    for c, pos in ((C(25999), "480 298"), (C(26000), "240 149")):
        for _ in range(4):
            if c.screen() != "DeathScreen":
                break
            c.cmd("mousepos " + pos)
            time.sleep(1.2)
            c.mouse("left", "tap")
            time.sleep(2.5)
