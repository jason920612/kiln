from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
Y = 150; X = 20
OB(f"fill {X-3} {Y-3} {X-3} {X+3} {Y+3} {X+3} minecraft:air")
OB(f"setblock {X} {Y} {X} minecraft:crafter")
time.sleep(1.5)
for t in "KV":
    print(t, ask(t, f"execute in minecraft:overworld run data get block {X} {Y} {X}", wait=0.5).strip()[-300:])
OB(f"fill {X-3} {Y-3} {X-3} {X+3} {Y+3} {X+3} minecraft:air")
