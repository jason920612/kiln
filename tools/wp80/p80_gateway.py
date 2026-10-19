"""wp80 item 2: end gateways, lock step: an entity inside a gateway block is teleported to its exit portal (exact or searched), cooldowns, what is left behind.
Console only; uses the end with forceloaded squares."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))


def E(cmd):
    both_("execute in minecraft:the_end run " + cmd)


def pos_of(t, sel):
    o = ask(t, f"execute in minecraft:the_end run data get entity {sel} Pos", wait=0.4)
    m = re.findall(r"(-?\d+(?:\.\d+)?)d", o)
    return [round(float(v), 2) for v in m[:3]]


E("forceload add 0 0 48 48")
E("forceload add 90 90 120 120")
time.sleep(4)
for t in "KV":
    wait_loaded(t, 10, 100, 10, dim="the_end", secs=60)
    wait_loaded(t, 100, 100, 100, dim="the_end", secs=60)
E("fill 5 60 5 40 90 40 minecraft:air")
E("fill 95 60 95 110 90 110 minecraft:air")
E("fill 5 69 5 40 69 40 minecraft:end_stone")
E("fill 95 69 95 110 69 110 minecraft:end_stone")
cases = [
    ("exact", "{exit_portal:[I;100,75,100],ExactTeleport:1b}"),
    ("not exact", "{exit_portal:[I;100,75,100]}"),
    ("no exit", ""),
]
x = 10
for name, nbt_text in cases:
    E(f"setblock {x} 70 10 minecraft:end_gateway{nbt_text}")
    time.sleep(0.5)
    E(f"summon minecraft:pig {x}.5 70 10.5 {{NoAI:1b,Tags:[\"p{x}\"],PersistenceRequired:1b}}")
    time.sleep(2.5)
    r = {t: pos_of(t, f"@e[tag=p{x},limit=1]") for t in "KV"}
    cmpn(f"gateway {name:10s} pig after 2.5 s", r["K"], r["V"])
    for t in "KV":
        o = ask(t, f"execute in minecraft:the_end run data get block {x} 70 10", wait=0.4)
        print("   ", t, "gateway:", o.strip()[-200:], flush=True)
    x += 6
# an item and an arrow
E("setblock 30 70 10 minecraft:end_gateway{exit_portal:[I;100,75,100],ExactTeleport:1b}")
E("summon minecraft:item 30.5 70 10.5 {Item:{id:\"minecraft:stone\",count:3},Tags:[\"it\"],PickupDelay:32767s}")
E("summon minecraft:arrow 30.5 71.5 10.5 {Motion:[0.0d,-0.5d,0.0d],Tags:[\"ar\"]}")
time.sleep(3)
for sel in ("it", "ar"):
    r = {t: pos_of(t, f"@e[tag={sel},limit=1]") for t in "KV"}
    cmpn(f"gateway {sel} after 3 s", r["K"], r["V"])
E("kill @e[type=!player]")
E("fill 5 60 5 40 90 40 minecraft:air")
E("fill 95 60 95 110 90 110 minecraft:air")
E("forceload remove all")
