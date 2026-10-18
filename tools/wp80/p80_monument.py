"""wp80 item 1: sponges (water absorption, wet sponge drying in the nether) and conduits (prismarine frame levels, activation, range, the attack on a hostile mob in water),
lock step on both servers (console only)."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 20, 150, 20
both_("kill @e[type=!player]")
# ---- sponge: a 9x4x9 pool with the sponge in the middle on the floor, and a lone sponge in a 3x3 puddle
OB(f"fill {X - 12} {Y - 2} {Z - 12} {X + 12} {Y + 8} {Z + 12} minecraft:air")
OB(f"fill {X - 5} {Y - 1} {Z - 5} {X + 5} {Y - 1} {Z + 5} minecraft:stone")
OB(f"fill {X - 5} {Y} {Z - 5} {X + 5} {Y + 3} {Z + 5} minecraft:water")
time.sleep(0.5)
kn = {t: count_blocks(t, X - 6, Y, Z - 6, X + 6, Y + 3, Z + 6, "minecraft:water") for t in "KV"}
cmpn("pool water before", kn["K"], kn["V"])
OB(f"setblock {X} {Y} {Z} minecraft:sponge")
time.sleep(1.0)
w = {t: count_blocks(t, X - 6, Y, Z - 6, X + 6, Y + 3, Z + 6, "minecraft:water") for t in "KV"}
cmpn("pool water after sponge", w["K"], w["V"])
for t in "KV":
    pass
r = {}
for t in "KV":
    r[t] = [count_blocks(t, X - 6, Y - 1, Z - 6, X + 6, Y + 4, Z + 6, b) for b in ("minecraft:wet_sponge", "minecraft:sponge", "minecraft:water")]
cmpn("wet_sponge/sponge/water count", r["K"], r["V"])
# the wet sponge keeps absorbing nothing; place a second sponge next to it, then a third sponge in dry air (no change)
OB(f"setblock {X + 3} {Y} {Z + 3} minecraft:sponge")
OB(f"setblock {X + 8} {Y} {Z + 8} minecraft:sponge")
time.sleep(1.0)
r = {t: [count_blocks(t, X - 9, Y - 1, Z - 9, X + 9, Y + 4, Z + 9, b) for b in ("minecraft:wet_sponge", "minecraft:sponge", "minecraft:water")] for t in "KV"}
cmpn("after 2 more sponges", r["K"], r["V"])
# waterlogged blocks and kelp: a sponge also soaks waterlogged slabs / seagrass and removes kelp
OB(f"fill {X - 12} {Y} {Z - 12} {X + 12} {Y + 8} {Z + 12} minecraft:air")
OB(f"fill {X - 5} {Y - 1} {Z - 5} {X + 5} {Y - 1} {Z + 5} minecraft:stone")
OB(f"fill {X - 5} {Y} {Z - 5} {X + 5} {Y + 3} {Z + 5} minecraft:water")
OB(f"setblock {X + 1} {Y} {Z + 1} minecraft:kelp[age=3]")
OB(f"setblock {X + 2} {Y} {Z + 1} minecraft:seagrass")
OB(f"setblock {X - 1} {Y} {Z - 1} minecraft:oak_slab[waterlogged=true]")
OB(f"setblock {X - 2} {Y} {Z - 1} minecraft:tall_seagrass[half=lower]")
OB(f"setblock {X - 2} {Y + 1} {Z - 1} minecraft:tall_seagrass[half=upper]")
OB(f"setblock {X + 1} {Y + 1} {Z + 1} minecraft:kelp_plant")
time.sleep(0.5)
OB(f"setblock {X} {Y} {Z} minecraft:sponge")
time.sleep(1.0)
r = {t: [count_blocks(t, X - 6, Y, Z - 6, X + 6, Y + 3, Z + 6, b) for b in ("minecraft:water", "minecraft:kelp", "minecraft:seagrass", "minecraft:tall_seagrass", "minecraft:kelp_plant", "minecraft:oak_slab[waterlogged=true]", "minecraft:oak_slab[waterlogged=false]", "minecraft:wet_sponge")] for t in "KV"}
cmpn("plants/slab case", r["K"], r["V"])
# ---- wet sponge in the nether dries into a sponge (and in the overworld it stays)
both_("execute in minecraft:the_nether run forceload add 0 0 16 16")
time.sleep(3)
for t in "KV":
    wait_loaded(t, 8, 70, 8, dim="the_nether", secs=60)
both_("execute in minecraft:the_nether run setblock 8 70 8 minecraft:air")
both_("execute in minecraft:the_nether run setblock 8 70 8 minecraft:wet_sponge")
time.sleep(1.2)
for t in "KV":
    o = ask(t, "execute in minecraft:the_nether if block 8 70 8 minecraft:sponge", wait=0.5)
    print(t, "nether wet_sponge -> sponge:", ("pass" in o.lower() or "Test passed" in o), o.strip()[-90:])
both_("execute in minecraft:the_nether run setblock 8 70 8 minecraft:netherrack")
both_("execute in minecraft:the_nether run forceload remove 0 0 16 16")
OB(f"fill {X - 12} {Y - 2} {Z - 12} {X + 12} {Y + 8} {Z + 12} minecraft:air")
