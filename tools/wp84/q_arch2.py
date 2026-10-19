from lab84 import *
cl = clients()
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, "execute in minecraft:overworld run tp KilnTest -544 80 160")
time.sleep(25)
for name, box in (("sand", (-570, 30, 130, -520, 80, 190)), ("spawner", (-790, 40, 140, -745, 90, 180))):
    for t in "KV":
        sub = "suspicious_sand" if name == "sand" else "spawner"
        if name == "spawner":
            for tt in "KV":
                w(tt, "execute in minecraft:overworld run tp KilnTest -768 90 160")
            time.sleep(15)
        r = json.loads(cl[t].cmd("findbox %s %d %d %d %d %d %d 400" % (sub, *box)))
        print(name, t, len(r), sorted(map(tuple, r))[:60] if name == "spawner" else "")
        if name == "spawner":
            print([ (p, cl[t].block(*p)) for p in r])
    if name == "sand":
        k = set(map(tuple, json.loads(cl["K"].cmd("findbox suspicious_sand -570 30 130 -520 80 190 400"))))
        v = set(map(tuple, json.loads(cl["V"].cmd("findbox suspicious_sand -570 30 130 -520 80 190 400"))))
        print("onlyK", sorted(k - v))
        for p in sorted(k - v)[:6]:
            print(p, "K", cl["K"].block(*p)[:50], "V", cl["V"].block(*p)[:50])
