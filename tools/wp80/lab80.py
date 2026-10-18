"""wp80 lab helpers on top of lock.py: a platform at a far corner of the forceloaded square, player health / entity data parsing for both servers."""
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
