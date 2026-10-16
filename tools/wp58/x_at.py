from p68_ghast import *
kill_all_mobs()
sv("tp KilnTest 20.5 200 20.5", show=False); time.sleep(1.5)
rec_start(a)
sv("summon minecraft:happy_ghast 26.5 202 26.5 {NoAI:1b}", show=False)
time.sleep(1.5)
ls = lines()
F([l[:300] for l in ls if "attributes" in l or "entity_data" in l])
g = ents_of(a, "happy_ghast", 40)[0]
print(a.cmd(f"attrs {g['id']}")[:600])
