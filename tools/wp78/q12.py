from lock import *
import re
X0, Y0, Z0, W, H, D = 10, 200, 10, 9, 4, 9
both_("tick freeze")
for rnd in range(3):
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 + H + 1} {Z0 + D} minecraft:air")
    both_(f"fill {X0 - 1} {Y0 - 1} {Z0 - 1} {X0 + W} {Y0 - 1} {Z0 + D} minecraft:stone")
    both_("kill @e[type=!player]")
    both_("tick step 30"); time.sleep(2.2)
    for c in ["setblock 15 200 15 minecraft:slime_block", "setblock 15 201 15 minecraft:slime_block", "setblock 15 201 14 minecraft:sticky_piston[facing=south]", "setblock 14 201 14 minecraft:stone"]:
        both_(c)
    time.sleep(0.5)
    for n in (1, 3, 8, 20):
        both_(f"tick step {n}"); time.sleep(0.4 + 0.06 * n)
        row = {}
        for t in "KV":
            out = ask(t, f"execute as @e[type=item,x={X0 - 3},y={Y0 - 3},z={Z0 - 3},dx={W + 6},dy={H + 80},dz={D + 6}] run data get entity @s Item", wait=1.0)
            row[t] = re.findall(r"id: \"minecraft:([a-z_]+)\"", out)
        print(rnd, "after", n, "ticks: items K", row["K"], "V", row["V"], flush=True)
    blocks = {t: ask(t, "execute if block 15 201 14 minecraft:sticky_piston", wait=0.5).strip()[-40:] for t in "KV"}
    print("   piston present", blocks)
