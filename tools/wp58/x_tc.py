from p68_small import *
sv(f"gamemode creative {ME}", show=False)
sv(f"tp {ME} 230.5 20 230.5", show=False)
sv("forceload add 200 200 260 260", show=False)
time.sleep(12)
out = sv("place structure minecraft:trial_chambers 230 -30 230", show=False, wait=6.0)
F("place:", out[-200:])
time.sleep(4)
sv(f"tp {ME} 230.5 -24 230.5", show=False)
time.sleep(6)
for name in ("trial_spawner", "vault", "chest", "copper_bulb", "waxed", "decorated_pot", "dispenser", "heavy_core", "crafter"):
    hits = a.find(name, 60, 400)
    F(name, len(hits), hits[:3])
F("player", pos(a), a.state(0)["health"])
