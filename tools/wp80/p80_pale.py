"""wp80 item 7: eyeblossoms open and close with the time of day (random ticks), pale moss with bone meal (dispensers). Lock step, console only."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false"))
both_("difficulty normal")
Y = 150
X0, Z0 = 4, 4
both_("kill @e[type=!player]")
OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 40} {Y + 4} {Z0 + 40} minecraft:air")
OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 40} {Y - 1} {Z0 + 40} minecraft:grass_block")
# 20x20 blossoms: left half closed, right half open
for z in range(0, 40, 2):
    for x in range(0, 40, 2):
        pass
OB(f"fill {X0} {Y} {Z0} {X0 + 19} {Y} {Z0 + 39} minecraft:closed_eyeblossom")
OB(f"fill {X0 + 20} {Y} {Z0} {X0 + 39} {Y} {Z0 + 39} minecraft:open_eyeblossom")
def counts(t):
    return [count_blocks(t, X0, Y, Z0, X0 + 39, Y, Z0 + 39, b) for b in ("minecraft:closed_eyeblossom", "minecraft:open_eyeblossom")]
for label, tm in (("day", 1000), ("night", 14000), ("dusk", 12500), ("dawn", 23500), ("day", 6000)):
    both_(f"time set {tm}")
    both_("gamerule random_tick_speed 4096")
    time.sleep(5)
    both_("gamerule random_tick_speed 0")
    r = {t: counts(t) for t in "KV"}
    cmpn(f"time {label:5s} {tm:5d} [closed, open]", r["K"], r["V"])
    both_("gamerule random_tick_speed 4096")
    time.sleep(10)
    both_("gamerule random_tick_speed 0")
    r = {t: counts(t) for t in "KV"}
    cmpn(f"  +10s            [closed, open]", r["K"], r["V"])
both_("time set 1000")
OB(f"fill {X0 - 1} {Y - 1} {Z0 - 1} {X0 + 40} {Y + 4} {Z0 + 40} minecraft:air")
