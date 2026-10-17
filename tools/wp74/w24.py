from lock import *
import re
X0, Y0, Z0 = 100, 200, 100
for t in "KV":
    w(t, "kill @e[type=villager]"); w(t, "time set 2000")
    x, z = X0 + 3, Z0
    w(t, f"fill {x - 1} {Y0} {z - 1} {x + 1} {Y0 + 2} {z + 3} minecraft:stone")
    w(t, f"fill {x} {Y0} {z} {x} {Y0 + 1} {z + 1} minecraft:air")
    w(t, f"fill {x} {Y0 + 2} {z} {x} {Y0 + 2} {z + 1} minecraft:glass")
    w(t, f"setblock {x} {Y0} {z + 1} minecraft:lectern")
    w(t, f'summon minecraft:villager {x}.5 {Y0} {z}.5 {{PersistenceRequired:1b,Tags:["v1"],VillagerData:{{profession:"minecraft:none",level:1,type:"minecraft:plains"}}}}')
time.sleep(1)
for t in "KV": sprint(t, 100)
for t in "KV":
    w(t, 'execute as @e[tag=v1] run data merge entity @s {Offers:{Recipes:[{buy:{id:"minecraft:emerald",count:1},sell:{id:"minecraft:bread",count:1},uses:99,maxUses:12,xp:1,priceMultiplier:0.05f,demand:0,specialPrice:0,rewardExp:1b}]}}')
for step in range(10):
    for t in "KV": sprint(t, 600)
    for t in "KV": w(t, "tick freeze")
    time.sleep(0.4)
    for t in "KV":
        o = ask(t, 'data get entity @e[tag=v1,limit=1]', wait=0.5)
        f = lambda k: (re.findall(k + r": (-?\d+)", o) or ["-"])[0]
        up = re.findall(r"uses: (\d+)", o)
        mem = re.findall(r"minecraft:(job_site|potential_job_site|meeting_point|home|last_worked_at_poi|last_slept)", o)
        print((step+1)*600, t, "LastRestock", f("LastRestock"), "RestocksToday", f("RestocksToday"), "uses", up, "mem", sorted(set(mem)), flush=True)
    for t in "KV": w(t, "tick unfreeze")
