"""wp84 lab helpers on top of lock.py: a platform at a far corner of the forceloaded square, player health / entity data parsing for both servers."""
from lock import *

def O(t, cmd, wait=0.3):
    return ask(t, "execute in minecraft:overworld run " + cmd, wait=wait)

def OB(cmd):
    both_("execute in minecraft:overworld run " + cmd)

def fnum(o, pat=r"(-?\d+(?:\.\d+)?)[fdb]?$"):
    m = re.findall(r"(-?\d+(?:\.\d+)?)[fd]\b", o)
    return float(m[-1]) if m else None

def health(t, who="KilnTest"):
    return fnum(ask(t, f"data get entity {who} Health", wait=0.25))

def eget(t, sel, path, wait=0.25):
    """raw answer of data get entity <sel> <path> (overworld)"""
    return ask(t, f"execute in minecraft:overworld run data get entity {sel} {path}", wait=wait)

def box(x0, y0, z0, x1, y1, z1, blk):
    OB(f"fill {x0} {y0} {z0} {x1} {y1} {z1} {blk}")

def tank(cx, cy, cz, r=3, h=3, water=True, wall="minecraft:glass"):
    """a (2r+1)^2 x h water tank with a bedrock floor, glass walls and roof, centre bottom cx,cy,cz (floor at cy-1)"""
    box(cx - r - 1, cy - 1, cz - r - 1, cx + r + 1, cy + h + 1, cz + r + 1, wall)
    box(cx - r - 1, cy - 1, cz - r - 1, cx + r + 1, cy - 1, cz + r + 1, "minecraft:bedrock")
    box(cx - r, cy, cz - r, cx + r, cy + h - 1, cz + r, "minecraft:water" if water else "minecraft:air")

def revive():
    respawn_all()


def nbt(t, what, sel, path="", wait=0.35):
    """normalised answer of `data get <what> <sel> [path]` in the overworld: the text after the prefix (Kiln: translation key, vanilla: English)"""
    o = ask(t, f"execute in minecraft:overworld run data get {what} {sel} {path}".rstrip(), wait=wait)
    o = o.strip().splitlines()[-1] if o.strip() else ""
    m = re.search(r"commands\.data\.\w+\.query\[(?:[^,\[\]]*, )?(.*)\]$", o)
    if m and "commands.data" in o:
        txt = m.group(1)
        # entity: the first argument is the entity name
        txt = re.sub(r"^entity\.minecraft\.\w+, ", "", txt)
        return txt
    m = re.search(r"(?:entity data|block data|storage|following [a-z ]*data): (.*)$", o)
    if m:
        return m.group(1)
    return o[-200:]


def cmpn(name, k, v, show=True):
    same = k == v
    if show:
        print(f"{name:34s} {'SAME' if same else 'DIFF'}  K {str(k)[:150]}  V {str(v)[:150]}", flush=True)
    return same


def probe(name, fn):
    """fn(t) for each server, compare the results"""
    r = {t: fn(t) for t in "KV"}
    return cmpn(name, r["K"], r["V"])


def block_at(t, x, y, z, dim="overworld"):
    """block id (with properties) at x y z via `execute if block` with a probe: use data get block for entities; this uses setblock-test with the client-free `fill ... replace`: returns the answer of `execute if block`"""
    return None


def count_blocks(t, x0, y0, z0, x1, y1, z1, blk, dim="overworld"):
    """number of blocks `blk` in the box: `execute store result score` is not readable on the console, so replace-fill with a marker block and read the count, then fill back"""
    o = ask(t, f"execute in minecraft:{dim} run fill {x0} {y0} {z0} {x1} {y1} {z1} minecraft:structure_void replace {blk}", wait=0.4)
    m = re.search(r"success\[(\d+)\]|filled (\d+) block|\[(\d+)\]", o)
    n = int(next(g for g in m.groups() if g)) if m else 0
    if n:
        ask(t, f"execute in minecraft:{dim} run fill {x0} {y0} {z0} {x1} {y1} {z1} {blk} replace minecraft:structure_void", wait=0.3)
    return n
