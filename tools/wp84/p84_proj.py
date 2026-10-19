"""wp84: real projectiles / hurting entities against the survival player: hurt packets the client gets (HurtAnimation, DamageEvent, motion), lock step."""
from lab84 import *
import collections
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false", "mob_griefing false"))
both_("difficulty normal")
X, Y, Z = 30, 200, 30
revive()
cl = clients()
for t in "KV":
    w(t, "gamemode survival KilnTest")
both_("kill @e[type=!player]")
both_("effect clear KilnTest")
both_("attribute KilnTest minecraft:max_health base set 1000")
box(X - 8, Y - 1, Z - 8, X + 8, Y - 1, Z + 8, "minecraft:bedrock")
box(X - 8, Y, Z - 8, X + 8, Y + 6, Z + 8, "minecraft:air")
CASES = {
 "arrow": "summon minecraft:arrow {x} {y} {z} {{Motion:[-3.0d,0.0d,0.0d],damage:2.0d}}",
 "small_fireball": "summon minecraft:small_fireball {x} {y} {z} {{Motion:[-0.5d,0.0d,0.0d],power:[-0.5d,0.0d,0.0d]}}",
 "llama_spit": "summon minecraft:llama_spit {x} {y} {z} {{Motion:[-1.0d,0.0d,0.0d]}}",
 "trident": "summon minecraft:trident {x} {y} {z} {{Motion:[-3.0d,0.0d,0.0d]}}",
 "snowball": "summon minecraft:snowball {x} {y} {z} {{Motion:[-1.0d,0.0d,0.0d]}}",
 "wind_charge": "summon minecraft:wind_charge {x} {y} {z} {{Motion:[-0.5d,0.0d,0.0d],acceleration_power:0.1d}}",
 "evoker_fangs": "summon minecraft:evoker_fangs {px} {py} {pz} {{Warmup:0}}",
 "zombie": "summon minecraft:zombie {x} {py} {pz} {{NoAI:0b,PersistenceRequired:1b}}",
 "falling_anvil": "summon minecraft:falling_block {px} {pyy} {pz} {{BlockState:{{Name:\"minecraft:anvil\"}},Time:1,HurtEntities:1b,FallHurtAmount:2.0f,FallHurtMax:40}}",
 "tnt": "summon minecraft:tnt {x} {py} {pz} {{fuse:1}}",
}
names = sys.argv[1].split(",") if len(sys.argv) > 1 else list(CASES)
both_("tick freeze")
for name in names:
    OB("kill @e[type=!player]")
    OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
    both_("effect give KilnTest minecraft:instant_health 1 10 true")
    step(14)
    for c in cl.values():
        c.cmd("spy on"); c.cmd("spylog clear")
    cmd = CASES[name].format(x=f"{X + 4}.5", y=f"{Y + 1.2}", z=f"{Z}.5", px=f"{X}.5", py=f"{Y}", pz=f"{Z}.5", pyy=f"{Y + 3}")
    OB(cmd)
    step(8)
    time.sleep(0.5)
    res = {}
    for k, c in cl.items():
        lines = json.loads(c.cmd("spylog clear"))
        cnt = collections.Counter(l.split(" ")[0] for l in lines)
        res[k] = {a: cnt[a] for a in ("HurtAnimation", "DamageEvent", "SetHealth") if cnt[a]}
        res[k + "cls"] = sorted(c for c in cnt if c not in ("entity_data", "Pos", "PosRot", "Rot", "entity_event", "sound"))
        res[k + "hp"] = c.state(0)["health"]
    print(f"{name:16s} {'SAME' if res['K'] == res['V'] else 'DIFF'}  K {res['K']} hp {res['Khp']} {res['Kcls']}  V {res['V']} hp {res['Vhp']} {res['Vcls']}", flush=True)
both_("tick unfreeze")
both_("kill @e[type=!player]")
