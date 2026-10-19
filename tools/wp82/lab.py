"""wp82 lab helpers on top of lk.py (console only, no clients): Kiln (K, ~/wp82/K) and vanilla (V, ~/wp82/V) driven from one process in lock step
(`tick freeze` + `tick step`). Answers are parsed from both logs (Kiln logs translation keys, vanilla English)."""
import re, sys, time
from lk import *

F = lambda *a: print(*a, flush=True)


def both_(cmd):
    w("K", cmd)
    w("V", cmd)


def OB(cmd):
    both_("execute in minecraft:overworld run " + cmd)


def O(t, cmd, wait=0.3):
    return ask(t, "execute in minecraft:overworld run " + cmd, wait=wait)


def setup(frozen=True, rules=(), size=60):
    w("V", "op W0")
    for g in ("random_tick_speed 0", "spawn_mobs false", "mob_griefing false", "advance_weather false", "advance_time false", "command_block_output false",
              "log_admin_commands false") + tuple(rules):
        both_("gamerule " + g)
    both_(f"execute in minecraft:overworld run forceload add 0 0 {size} {size}")
    time.sleep(3)
    if frozen:
        both_("tick freeze")
        time.sleep(0.5)


def nbt(t, what, sel, path="", wait=0.35):
    """normalised answer of `data get <what> <sel> [path]` in the overworld: the text after the prefix (Kiln: translation key, vanilla: English)"""
    o = ask(t, f"execute in minecraft:overworld run data get {what} {sel} {path}".rstrip(), wait=wait)
    o = o.strip().splitlines()[-1] if o.strip() else ""
    m = re.search(r"commands\.data\.\w+\.query\[(?:[^,\[\]]*, )?(.*)\]$", o)
    if m and "commands.data" in o:
        txt = m.group(1)
        return re.sub(r"^entity\.minecraft\.\w+, ", "", txt)
    m = re.search(r"(?:entity data|block data|storage|following [a-z ]*data): (.*)$", o)
    if m:
        return m.group(1)
    return o[-300:]


def cmpn(name, k, v, show=True):
    same = k == v
    if show:
        F(f"{name:34s} {'SAME' if same else 'DIFF'}  K {str(k)[:160]}  V {str(v)[:160]}")
    return same


def count_blocks(t, x0, y0, z0, x1, y1, z1, blk, dim="overworld"):
    """number of blocks `blk` in the box: replace-fill with a marker block, read the count, fill back"""
    o = ask(t, f"execute in minecraft:{dim} run fill {x0} {y0} {z0} {x1} {y1} {z1} minecraft:structure_void replace {blk}", wait=0.4)
    m = re.search(r"success\[(\d+)\]|filled (\d+) block|\[(\d+)\]", o)
    n = int(next(g for g in m.groups() if g)) if m else 0
    if n:
        ask(t, f"execute in minecraft:{dim} run fill {x0} {y0} {z0} {x1} {y1} {z1} {blk} replace minecraft:structure_void", wait=0.3)
    return n
