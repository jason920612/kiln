from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
scenario("eat84")
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv(f"effect give {ME} minecraft:saturation 5 5 true", show=False)
time.sleep(6)
sv(f"clear {ME}", show=False); sv(f"give {ME} minecraft:bread 2", show=False); sv(f"give {ME} minecraft:golden_apple 1", show=False)
for item in ("minecraft:bread", "minecraft:golden_apple"):
    a.select(item)
    st = a.state(0)
    n0 = a.inv().get(item, 0)
    a.mouse("right", "down"); time.sleep(2.6); a.mouse("right", "up"); time.sleep(0.8)
    F(item, "food", st["food"], "->", a.state(0)["food"], "count", n0, "->", a.inv().get(item, 0))
