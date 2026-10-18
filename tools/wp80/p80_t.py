from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
revive()
both_("tick unfreeze")
for t in "KV": w(t, "gamemode survival KilnTest")
X, Y, Z = 8, 150, 30
OB(f"fill {X - 12} {Y - 8} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
time.sleep(1.5)
for l in range(1, 5):
    OB(f"fill {X - l} {Y - l} {Z - l} {X + l} {Y - l} {Z + l} minecraft:iron_block")
OB(f"setblock {X} {Y} {Z} minecraft:beacon")
time.sleep(5)
for prim, sec, dist in (("speed", "speed", 41), ("speed", "regeneration", 52), ("strength", "regeneration", 49), ("haste", "regeneration", 51), ("jump_boost", "", 3)):
    OB(f"data merge block {X} {Y} {Z} {{primary_effect:\"minecraft:{prim}\"" + (f",secondary_effect:\"minecraft:{sec}\"" if sec else "") + "}")
    OB(f"tp KilnTest {X + dist}.5 {Y + 1} {Z}.5")
    both_("effect clear KilnTest")
    time.sleep(7)
    out = {}
    for t in "KV":
        e = ask(t, "data get entity KilnTest active_effects", wait=0.4)
        out[t] = sorted(zip(re.findall(r'id: "minecraft:(\w+)"', e), re.findall(r"amplifier: (\d+)b", e) or ["0"] * 9))
    cmpn(f"{prim}+{sec or '-'} at {dist}", out["K"], out["V"])
OB(f"fill {X - 12} {Y - 8} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
