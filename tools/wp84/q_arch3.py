from lab84 import *
cl = clients()
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, "execute in minecraft:overworld run tp KilnTest -768 65 160")
time.sleep(35)
for t in "KV":
    r = json.loads(cl[t].cmd("findbox spawner -808 -60 120 -728 120 200 400"))
    print(t, [(p, cl[t].block(*p)[:40]) for p in r])
    sp = cl[t].cmd("findbox sandstone -808 55 120 -728 90 200 5")
    print(t, "sandstone sample", sp[:60])
for t in "KV":
    print(t, nbt(t, "block", "%s %s %s" % tuple(json.loads(cl[t].cmd("findbox spawner -808 -60 120 -728 120 200 400"))[0]), "SpawnData")[:200])
