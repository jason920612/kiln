"""wp80 item 1: guardian beam against a survival player in a water tank (both real players, lock step with freeze/step). Health every 10 ticks for N ticks, R reps."""
from lab80 import *
N = int(sys.argv[1]) if len(sys.argv) > 1 else 400
R = int(sys.argv[2]) if len(sys.argv) > 2 else 3
KIND = sys.argv[3] if len(sys.argv) > 3 else "guardian"
setup(frozen=False, rules=("natural_regeneration false", "command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
X, Y, Z = 30, 200, 30
revive()
for t in "KV":
    w(t, "gamemode survival KilnTest")
for rep in range(R):
    revive()
    both_("kill @e[type=!player]")
    both_("effect clear KilnTest")
    both_("effect give KilnTest minecraft:water_breathing 999 0 true")
    both_("attribute KilnTest minecraft:max_health base set 1000")
    both_("effect give KilnTest minecraft:instant_health 1 10 true")
    tank(X, Y, Z, 7, 4)
    OB(f"tp KilnTest {X}.5 {Y} {Z}.5")
    both_("tick freeze")
    time.sleep(1)
    both_("effect give KilnTest minecraft:instant_health 1 20 true")
    step(2)
    OB(f"summon minecraft:{KIND} {X + 5}.5 {Y + 1} {Z + 5}.5 {{PersistenceRequired:1b}}")
    h0 = {t: health(t) for t in "KV"}
    series = {t: [] for t in "KV"}
    for i in range(0, N, 5):
        step(5)
        for t in "KV":
            series[t].append(health(t))
    for t in "KV":
        hs = series[t]
        hits = []
        prev = h0[t]
        for i, h in enumerate(hs):
            if h is not None and prev is not None and h < prev - 1e-6:
                hits.append((5 * (i + 1), round(prev - h, 3)))
            prev = h if h is not None else prev
        print(f"rep {rep} {t}: start {h0[t]} hits(tick,dmg) {hits}", flush=True)
    both_("tick unfreeze")
both_("kill @e[type=!player]")
