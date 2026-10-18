"""wp78: thrown trident with Impaling against an aquatic mob (guardian) and a land mob (cow), K against V in lock step; the tag that carries the stack is tried as `weapon` and as `item`."""
from lock import *
X, Y, Z = 800, 200, 800
setup(frozen=False, rules=("mob_griefing false", "command_block_output false"))
both_(f"execute in minecraft:overworld run forceload add {X - 16} {Z - 16} {X + 16} {Z + 16}")
for t in "KV":
    wait_loaded(t, X + 12, Y + 8, Z + 12, secs=120)
both_("tick freeze")
def hp(t, tag):
    o = ask(t, f"execute in minecraft:overworld run data get entity @e[tag={tag},limit=1] Health", wait=0.5)
    m = re.findall(r"(\d+(?:\.\d+)?)f", o)
    return float(m[-1]) if m else None
for key in ("weapon", "item"):
    for mob, health in (("guardian", 30), ("cow", 10), ("drowned", 20)):
        for lvl in (0, 5):
            both_("kill @e[type=!player]")
            both_(f"execute in minecraft:overworld run fill {X - 8} {Y - 1} {Z - 8} {X + 12} {Y + 4} {Z + 8} minecraft:air")
            both_(f"execute in minecraft:overworld run fill {X - 8} {Y - 1} {Z - 8} {X + 12} {Y - 1} {Z + 8} minecraft:bedrock")
            time.sleep(0.6)
            both_(f"execute in minecraft:overworld run summon minecraft:{mob} {X + 0.5} {Y} {Z + 0.5} {{NoAI:1b,Health:500f,attributes:[{{id:\"minecraft:max_health\",base:500d}}],Tags:[\"m\"]}}")
            ench = f"\"minecraft:enchantments\":{{\"minecraft:impaling\":{lvl}}}" if lvl else ""
            stack = f"{{id:\"minecraft:trident\",count:1,components:{{{ench}}}}}"
            both_(f"execute in minecraft:overworld run summon minecraft:trident {X - 5.5} {Y + 0.6} {Z + 0.5} {{Motion:[2.5d,0.0d,0.0d],{key}:{stack}}}")
            step(8)
            r = {t: hp(t, "m") for t in "KV"}
            d = {t: round(500 - r[t], 2) if r[t] else None for t in "KV"}
            print(f"{key:7s} {mob:9s} impaling {lvl}: damage K {d['K']} V {d['V']} {'SAME' if d['K'] == d['V'] else 'DIFF'}", flush=True)
both_("tick unfreeze")
