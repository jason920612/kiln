"""wp80 item 8: copper bulbs in redstone (toggling, lit / powered, light, comparator output), lightning on a lightning rod and on weathered copper (cleaning, waxed stays),
lock step (console only)."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_weather_cycle false"))
both_("difficulty normal")
Y = 150
X0, Z0 = 8, 8
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 30} {Y + 10} {Z0 + 20} minecraft:air")
OB(f"fill {X0 - 2} {Y - 1} {Z0 - 2} {X0 + 30} {Y - 1} {Z0 + 20} minecraft:stone")
both_("tick freeze")
time.sleep(0.5)


def state_of(t, x, y, z):
    o = ask(t, f"execute in minecraft:overworld run data get block {x} {y} {z}", wait=0.25)
    return o
# ---- copper bulbs: a bulb with a redstone block beside, toggled on and off
kinds = ["copper_bulb", "exposed_copper_bulb", "weathered_copper_bulb", "oxidized_copper_bulb", "waxed_copper_bulb"]
for i, kind in enumerate(kinds):
    x = X0 + 2 * i
    OB(f"setblock {x} {Y} {Z0} minecraft:{kind}")
    OB(f"setblock {x} {Y} {Z0 + 1} minecraft:comparator[facing=south]")
step(2)
def read(t, x):
    b = ask(t, f"execute in minecraft:overworld run execute if block {x} {Y} {Z0} minecraft:{kinds[(x - X0) // 2]}[lit=true]", wait=0.2)
    p = ask(t, f"execute in minecraft:overworld run execute if block {x} {Y} {Z0} minecraft:{kinds[(x - X0) // 2]}[powered=true]", wait=0.2)
    return ("conditional.pass" in b or "Test passed" in b, "conditional.pass" in p or "Test passed" in p)
rows = {}
for pulse in range(4):
    for i, kind in enumerate(kinds):
        OB(f"setblock {X0 + 2 * i - 1} {Y} {Z0} minecraft:redstone_block")
    step(3)
    on = {t: [read(t, X0 + 2 * i) for i in range(5)] for t in "KV"}
    cmpn(f"pulse {pulse} powered: (lit, powered) per bulb", on["K"], on["V"])
    for i, kind in enumerate(kinds):
        OB(f"setblock {X0 + 2 * i - 1} {Y} {Z0} minecraft:air")
    step(3)
    off = {t: [read(t, X0 + 2 * i) for i in range(5)] for t in "KV"}
    cmpn(f"pulse {pulse} released:", off["K"], off["V"])
# the light level of a lit bulb: the block entity free; the comparator beside reads nothing, use the light via a daylight-free fill test
# ---- lightning: weathered copper blocks around a rod, a rod, a waxed block
OB(f"fill {X0} {Y} {Z0 + 6} {X0 + 10} {Y} {Z0 + 6} minecraft:oxidized_copper")
OB(f"setblock {X0 + 2} {Y} {Z0 + 6} minecraft:weathered_copper")
OB(f"setblock {X0 + 4} {Y} {Z0 + 6} minecraft:waxed_oxidized_copper")
OB(f"setblock {X0 + 6} {Y} {Z0 + 6} minecraft:exposed_copper_stairs[facing=east]")
OB(f"setblock {X0 + 8} {Y} {Z0 + 6} minecraft:oxidized_cut_copper")
OB(f"setblock {X0 + 6} {Y + 1} {Z0 + 6} minecraft:oxidized_lightning_rod[facing=up]")
step(2)
OB(f"summon minecraft:lightning_bolt {X0 + 2}.5 {Y + 1} {Z0 + 6}.5")
step(2)
def chain(t):
    out = []
    for dx in range(0, 11):
        o = ask(t, f"execute in minecraft:overworld run data get block {X0 + dx} {Y} {Z0 + 6}", wait=0.0)
        out.append(None)
    return out
names = lambda t: [kiln_name(t, X0 + dx, Y, Z0 + 6) for dx in range(0, 11)]
def kiln_name(t, x, y, z):
    for cand in ("copper", "exposed_copper", "weathered_copper", "oxidized_copper", "waxed_oxidized_copper", "exposed_copper_stairs", "oxidized_cut_copper", "weathered_cut_copper", "exposed_cut_copper", "cut_copper", "waxed_copper", "air"):
        o = ask(t, f"execute in minecraft:overworld run execute if block {x} {y} {z} minecraft:{cand}", wait=0.12)
        if "conditional.pass" in o or "Test passed" in o:
            return cand
    return "?"
r = {t: [kiln_name(t, X0 + dx, Y, Z0 + 6) for dx in (0, 1, 2, 3, 4, 5, 6, 8)] for t in "KV"}
cmpn("after a bolt at x+2 (a row of oxidized copper, weathered at 2, waxed at 4)", r["K"], r["V"])
rod = {t: kiln_name(t, X0 + 6, Y + 1, Z0 + 6) for t in "KV"}
o = {t: ask(t, f"execute in minecraft:overworld run execute if block {X0 + 6} {Y + 1} {Z0 + 6} minecraft:lightning_rod", wait=0.15) for t in "KV"}
cmpn("the rod above the stairs", {t: "pass" in o[t] for t in "KV"}["K"], {t: "pass" in o[t] for t in "KV"}["V"])
OB(f"summon minecraft:lightning_bolt {X0 + 6}.5 {Y + 1} {Z0 + 6}.5")
step(2)
r = {t: [kiln_name(t, X0 + dx, Y, Z0 + 6) for dx in (5, 6, 7, 8)] for t in "KV"}
cmpn("after a bolt on the rod over the stairs", r["K"], r["V"])
both_("tick unfreeze")
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 30} {Y + 10} {Z0 + 20} minecraft:air")
