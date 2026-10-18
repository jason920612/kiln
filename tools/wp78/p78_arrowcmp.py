"""wp78: arrows fired from enchanted weapons, Kiln against vanilla in lock step (console only): a NoAI cow with huge health, an arrow summoned 6 blocks away with
`weapon` Power 0..5 / Punch 0..3 (crit off): health lost and the cow's Motion x after the hit. Exact numbers, not statistics."""
from lock import *
X, Y, Z = 800, 200, 800
setup(frozen=False, rules=("mob_griefing false", "command_block_output false"))
both_(f"execute in minecraft:overworld run forceload add {X - 16} {Z - 16} {X + 16} {Z + 16}")
for t in "KV":
    wait_loaded(t, X + 12, Y + 8, Z + 12, secs=120)
both_("tick freeze")
def num(o, pat):
    m = re.findall(pat, o)
    return float(m[-1]) if m else None
rows = []
cases = [("none", "")] + [(f"power {l}", f"minecraft:power\":{l}") for l in range(1, 6)] + [(f"punch {l}", f"minecraft:punch\":{l}") for l in range(1, 4)] + [("power 5 + punch 2", "minecraft:power\":5,\"minecraft:punch\":2")]
for name, ench in cases:
    both_("kill @e[type=!player]")
    both_(f"execute in minecraft:overworld run fill {X - 8} {Y - 1} {Z - 8} {X + 12} {Y + 4} {Z + 8} minecraft:air")
    both_(f"execute in minecraft:overworld run fill {X - 8} {Y - 1} {Z - 8} {X + 12} {Y - 1} {Z + 8} minecraft:bedrock")
    time.sleep(0.8)
    both_(f"execute in minecraft:overworld run summon minecraft:cow {X + 0.5} {Y} {Z + 0.5} {{NoAI:1b,Health:5000f,attributes:[{{id:\"minecraft:max_health\",base:5000d}}],Tags:[\"c\"]}}")
    weapon = f",weapon:{{id:\"minecraft:bow\",count:1,components:{{\"minecraft:enchantments\":{{\"{ench}}}}}}}" if ench else ""
    both_(f"execute in minecraft:overworld run summon minecraft:arrow {X - 5.5} {Y + 0.6} {Z + 0.5} {{Motion:[3.0d,0.0d,0.0d],crit:0b,damage:2.0d{weapon}}}")
    step(8)
    out = {}
    for t in "KV":
        o = ask(t, "execute in minecraft:overworld run data get entity @e[tag=c,limit=1] Health", wait=0.5)
        m = ask(t, "execute in minecraft:overworld run data get entity @e[tag=c,limit=1] Motion", wait=0.5)
        out[t] = (num(o, r"(\d+(?:\.\d+)?)f"), re.findall(r"(-?\d+(?:\.\d+)?)d", m)[:1])
    d = {t: round(5000 - out[t][0], 3) if out[t][0] else None for t in "KV"}
    print(f"{name:20s} damage K {d['K']} V {d['V']}  motion x K {out['K'][1]} V {out['V'][1]}  {'SAME' if d['K'] == d['V'] and out['K'][1] == out['V'][1] else 'DIFF'}", flush=True)
both_("tick unfreeze")
