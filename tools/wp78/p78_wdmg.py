"""wp78: damage to a NoAI Wither (armour 4, the half-health arrow shield) in lock step: /damage with several types, arrows by entity."""
from lock import *
CX, CY, CZ = 500, 200, 500

def hp(t):
    o = ask(t, "data get entity @e[type=wither,limit=1] Health", wait=0.2)
    m = re.findall(r"(-?\d+(?:\.\d+)?)f", o)
    return float(m[-1]) if m else None

def both_hp():
    return {t: hp(t) for t in "KV"}

setup(frozen=False, rules=("mob_griefing false",))
for t in "KV":
    w(t, "gamemode creative KilnTest"); w(t, f"tp KilnTest {CX+8} {CY+4} {CZ+8}"); w(t, "effect give KilnTest minecraft:slow_falling 99999 0 true")
respawn_all()
time.sleep(3)
rows = []
def fresh(health):
    both_("kill @e[type=!player]")
    time.sleep(0.3)
    both_(f"summon minecraft:wither {CX}.5 {CY} {CZ}.5 {{NoAI:1b,Invul:0,Health:{health}f,Tags:[\"w\"]}}")
    time.sleep(0.5)
tests = [
 ("generic 10", "damage @e[type=wither,limit=1] 10 minecraft:generic"),
 ("generic 1", "damage @e[type=wither,limit=1] 1 minecraft:generic"),
 ("magic 10", "damage @e[type=wither,limit=1] 10 minecraft:magic"),
 ("arrow 6 (projectile)", "damage @e[type=wither,limit=1] 6 minecraft:arrow"),
 ("mob_attack 8", "damage @e[type=wither,limit=1] 8 minecraft:mob_attack"),
 ("explosion 10", "damage @e[type=wither,limit=1] 10 minecraft:explosion"),
 ("wither 1", "damage @e[type=wither,limit=1] 1 minecraft:wither"),
 ("fire 3", "damage @e[type=wither,limit=1] 3 minecraft:on_fire"),
 ("trident 8", "damage @e[type=wither,limit=1] 8 minecraft:trident"),
 ("thrown 4", "damage @e[type=wither,limit=1] 4 minecraft:thrown"),
 ("wither_skull 4", "damage @e[type=wither,limit=1] 4 minecraft:wither_skull"),
 ("fireball 4", "damage @e[type=wither,limit=1] 4 minecraft:fireball"),
 ("mace/player 20", "damage @e[type=wither,limit=1] 20 minecraft:player_attack"),
]
for health in (300, 140):
    for name, cmd in tests:
        fresh(health)
        b = both_hp()
        both_(cmd)
        time.sleep(0.4)
        a = both_hp()
        d = {t: (round(b[t] - a[t], 3) if a[t] is not None and b[t] is not None else None) for t in "KV"}
        print(f"hp {health:3d} {name:22s} damage K {d['K']} V {d['V']}  {'SAME' if d['K'] == d['V'] else 'DIFF'}", flush=True)
# real arrows flying into it
for health in (300, 140):
    for power in (0, 1):
        fresh(health)
        b = both_hp()
        both_(f"summon minecraft:arrow {CX-6}.5 {CY+1.2} {CZ}.5 {{Motion:[3.0d,0.0d,0.0d],crit:0b,damage:2.0d}}")
        both_("tick unfreeze") if False else None
        time.sleep(1.5)
        a = both_hp()
        d = {t: (round(b[t] - a[t], 3) if a[t] is not None and b[t] is not None else None) for t in "KV"}
        print(f"hp {health:3d} real arrow Motion 3.0 damage K {d['K']} V {d['V']}  {'SAME' if d['K'] == d['V'] else 'DIFF'}", flush=True)
both_("kill @e[type=!player]")
