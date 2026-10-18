from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
revive()
for t in "KV": w(t, "gamemode survival KilnTest")
X, Y, Z = 8, 150, 30
OB(f"fill {X - 12} {Y - 8} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
time.sleep(1.5)
for l in range(1, 5):
    OB(f"fill {X - l} {Y - l} {Z - l} {X + l} {Y - l} {Z + l} minecraft:iron_block")
OB(f"setblock {X} {Y} {Z} minecraft:beacon")
OB(f"tp KilnTest {X + 3}.5 {Y + 1} {Z}.5")
both_("effect clear KilnTest")
def show(tag):
    for t in "KV":
        b = ask(t, f"execute in minecraft:overworld run data get block {X} {Y} {Z}", wait=0.4)
        e = ask(t, "data get entity KilnTest active_effects", wait=0.4)
        print(tag, t, re.sub(r".*(query\[|following block data: )", "", b).strip()[:150], "|", re.findall(r'id: "minecraft:(\w+)"', e), re.findall(r"duration: (\d+)", e), flush=True)
time.sleep(10); show("after 10s")
OB(f"data merge block {X} {Y} {Z} {{primary_effect:\"minecraft:speed\"}}")
show("merged")
time.sleep(5); show("+5s")
time.sleep(5); show("+10s")
time.sleep(5); show("+15s")
OB(f"fill {X - 12} {Y - 8} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
