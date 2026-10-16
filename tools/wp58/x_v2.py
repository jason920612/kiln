from p68 import *
a = C(25999); b = C(26000)
ME="KilnTest"; OBS="Obs"
setup_two()
V = (20, 200, 18)
key = "minecraft:trial_key"
sv("fill 12 200 12 28 204 28 minecraft:air", show=False)
sv(f"setblock 20 200 18 minecraft:vault[ominous=false,facing=south]{{config:{{key_item:{{id:\"{key}\",count:1}},loot_table:\"minecraft:chests/trial_chambers/reward\"}}}}", show=False)
for nm in (ME, OBS):
    sv(f"clear {nm}", show=False); give(nm, key, 4)
sv(f"tp {ME} 19.5 200 21.5", show=False)
sv(f"tp {OBS} 21.5 200 21.5", show=False)
time.sleep(2)
b.select(key)
b.lookat(20.5, 200.5, 18.5)
time.sleep(0.8)
print("B hit", b.state(0).get("hit"), "B pos", pos(b), "held", b.state(0).get("held"))
print("data before", re.findall(r"connected_players: \[[^\]]*\]|state: \"\w+\"", sv("data get block 20 200 18", show=False, wait=0.6)))
b.mouse("right", "tap")
time.sleep(3)
print("B keys", b.inv().get(key), blk(a, *V))
print("data after", re.findall(r"connected_players: \[[^\]]*\]|state: \"\w+\"|rewarded_players: \[[^\]]*\]", sv("data get block 20 200 18", show=False, wait=0.6)))
print(sv("data get block 20 200 18", show=False, wait=0.6)[-600:])
