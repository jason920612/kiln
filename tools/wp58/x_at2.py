from p68_ghast import *
import json as _j
kill_all_mobs()
sv("tp KilnTest 20.5 200 20.5", show=False); time.sleep(1.5)
sv("summon minecraft:happy_ghast 26.5 202 26.5 {NoAI:1b}", show=False)
time.sleep(1.5)
g = ents_of(a, "happy_ghast", 40)[0]
at = _j.loads(a.cmd(f"attrs {g['id']}"))
print({k.split(":")[1]: v["v"] for k, v in at.items() if "speed" in k or "flying" in k or "camera" in k})
print(sv("data get entity @e[type=happy_ghast,limit=1] attributes", show=False)[-700:])
