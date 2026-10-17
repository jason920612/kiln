from lock import *
both_("kill @e[type=!player]"); time.sleep(0.5)
KS = range(0, 10)
for axis in ("x", "z"):
    for k in KS:
        x = -1279.5 + (16 * k if axis == "x" else 0); z = 736.5 + (16 * k if axis == "z" else 0)
        both_(f'summon minecraft:item {x} 130 {z} {{Item:{{id:"minecraft:stone",count:1}},Tags:["t{axis}{k}"],NoGravity:0b}}')
time.sleep(2.5)
for axis in ("x", "z"):
    row = {"K": [], "V": []}
    for k in KS:
        for t in "KV":
            o = ask(t, f"data get entity @e[tag=t{axis}{k},limit=1] Pos[1]", wait=0.25)
            m = re.search(r"(\d+\.\d+)d", o[-80:])
            row[t].append(m.group(1)[:5] if m else "?")
    print(axis, "K", row["K"]); print(axis, "V", row["V"])
both_("kill @e[type=!player]")
