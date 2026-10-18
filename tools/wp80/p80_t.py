from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
X, Z = 8, 30
def lv(t, Y):
    b = ask(t, f"execute in minecraft:overworld run data get block {X} {Y} {Z}", wait=0.4)
    m = re.search(r"Levels: (\d+)", b)
    return m.group(1) if m else b[-60:]
for Y in (150, 100, 66):
    OB(f"fill {X - 12} {Y - 8} {Z - 12} {X + 12} {Y + 12} {Z + 12} minecraft:air")
    time.sleep(1.5)
    for l in range(1, 5):
        OB(f"fill {X - l} {Y - l} {Z - l} {X + l} {Y - l} {Z + l} minecraft:iron_block")
    OB(f"setblock {X} {Y} {Z} minecraft:beacon")
    for i in range(3):
        time.sleep(5)
        print(Y, "K", lv("K", Y), "V", lv("V", Y), flush=True)
    for t in "K":
        print(count_blocks(t, X, Y + 1, Z, X, 319, Z, "minecraft:air"), "air above of", 319 - Y)
OB(f"fill {X - 12} 55 {Z - 12} {X + 12} 170 {Z + 12} minecraft:air")
