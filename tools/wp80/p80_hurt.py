"""wp80: `damage KilnTest 1 <type> by <cow 3 blocks east>` for every damage type, lock step: the player's Motion afterwards and which hurt packets his client got
(HurtAnimation, motion). Real survival players; the cow is NoAI."""
from lab80 import *
import collections
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 30, 200, 30
revive()
cl = clients()
for t in "KV":
    w(t, "gamemode survival KilnTest")
both_("kill @e[type=!player]")
both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
box(X - 6, Y - 1, Z - 6, X + 6, Y - 1, Z + 6, "minecraft:bedrock")
box(X - 6, Y, Z - 6, X + 6, Y + 5, Z + 6, "minecraft:air")
types = sys.argv[1].split(",") if len(sys.argv) > 1 else None
reg = open("/home/test/kiln/work/generated/data/minecraft/damage_type/") if False else None
import os
TD = "/home/test/kiln/work/generated/data/minecraft/damage_type"
names = sorted(f[:-5] for f in os.listdir(TD)) if os.path.isdir(TD) else []
if types:
    names = types
both_("tick freeze")
rows = []
for name in names:
    OB(f"kill @e[type=!player]")
    OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
    OB(f"summon minecraft:cow {X + 3}.5 {Y} {Z}.5 {{NoAI:1b,Tags:[\"src\"],Silent:1b}}")
    both_("effect give KilnTest minecraft:instant_health 1 30 true")
    step(14)
    for c in cl.values():
        c.cmd("spy on"); c.cmd("spylog clear")
    out = {}
    both_(f"execute in minecraft:overworld run damage KilnTest 1 minecraft:{name} by @e[tag=src,limit=1]")
    step(2)
    time.sleep(1.5)
    for k, c in cl.items():
        st = c.state(0)
        out[k] = [round(st["x"] - (X + 0.5), 3), round(st["y"] - Y, 3), round(st["z"] - (Z + 0.5), 3)]
    for k, c in cl.items():
        lines = json.loads(c.cmd("spylog clear"))
        cnt = collections.Counter(l.split(" ")[0] for l in lines)
        out[k + "pk"] = {a: cnt[a] for a in ("HurtAnimation", "DamageEvent", "SetHealth") if cnt[a]}
        out[k + "mot"] = sum(1 for l in lines if "motion" in l.lower())
        out[k + "cls"] = sorted(set(l.split(" ")[0] for l in lines))
    same = all(abs(a - b) < 0.01 for a, b in zip(out["K"], out["V"])) and out["Kpk"] == out["Vpk"]
    print(f"{name:32s} {'SAME' if same else 'DIFF'}  K disp {out['K']} {out['Kpk']} m{out['Kmot']}   V disp {out['V']} {out['Vpk']} m{out['Vmot']}", flush=True)
both_("tick unfreeze")
both_("kill @e[type=!player]")
