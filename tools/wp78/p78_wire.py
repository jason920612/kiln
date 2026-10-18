"""wp78: the jungle temple's tripwire trap, console only (lock step helpers): an entity put on the tripwire (-302,66,-1226) must power the hooks, the redstone and fire the dispensers."""
from lock import *
def ok(t, c): 
    o = ask(t, c, wait=0.5)
    return ("pass" in o.lower() and "fail" not in o.lower()) or "Test passed" in o
both_("execute in minecraft:overworld run forceload add -312 -1240 -288 -1212")
for t in "KV":
    wait_loaded(t, -310, 110, -1240, secs=90)
time.sleep(2)
both_("execute in minecraft:overworld run setblock -302 66 -1226 minecraft:tripwire[attached=true,east=true,west=true]")
time.sleep(1)
both_("gamerule spawn_mobs false")
for what in ("minecraft:cow", "minecraft:item", "minecraft:arrow"):
    both_("kill @e[type=!player]")
    time.sleep(0.5)
    print("before", {t: (ok(t, "execute in minecraft:overworld if block -302 66 -1226 minecraft:tripwire[powered=true]"), ok(t, "execute in minecraft:overworld if block -295 67 -1221 minecraft:dispenser[triggered=true]")) for t in "KV"})
    extra = "{Item:{id:'minecraft:stone',count:1}}" if what.endswith("item") else ("{NoAI:1b}" if what.endswith("cow") else "{Motion:[0.0d,0.0d,0.0d]}")
    both_(f"execute in minecraft:overworld run summon {what} -301.5 66 -1225.5 {extra}")
    for i in range(6):
        time.sleep(0.5)
        r = {t: (ok(t, "execute in minecraft:overworld if block -302 66 -1226 minecraft:tripwire[powered=true]"), ok(t, "execute in minecraft:overworld if block -303 66 -1226 minecraft:tripwire_hook[powered=true]"), ok(t, "execute in minecraft:overworld if block -295 67 -1221 minecraft:dispenser[triggered=true]"), ok(t, "execute in minecraft:overworld if block -301 67 -1219 minecraft:dispenser[triggered=true]")) for t in "KV"}
        print(what, i, "wire/hook/disp1/disp2:", r)
