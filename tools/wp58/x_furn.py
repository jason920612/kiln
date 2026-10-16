from p68_small import *
def sprint(n):
    sv(f"tick sprint {n}", show=False, wait=0.5)
    for _ in range(60):
        if "sprinting" not in sv("tick query", show=False, wait=0.3).lower():
            break
        time.sleep(0.5)
def gt():
    return int(re.findall(r"(\d+)", sv("time query gametime", show=False, wait=0.4))[-1])
for item in ("beef", "resin_clump"):
    sv("setblock 18 200 20 minecraft:air", show=False)
    sv("setblock 18 200 20 minecraft:furnace[facing=south]", show=False)
    sv(f"item replace block 18 200 20 container.0 with minecraft:{item} 4", show=False)
    sv("item replace block 18 200 20 container.1 with minecraft:coal 1", show=False)
    g0 = gt()
    for step in range(5):
        sprint(150)
        d = sv("data get block 18 200 20", show=False, wait=0.5)
        print(item, "t+", gt() - g0, re.findall(r"cooking_time_spent: \d+", d), re.findall(r'count: \d+, Slot: \d+b, id: "minecraft:\w+"', sv("data get block 18 200 20 Items", show=False, wait=0.5)))
