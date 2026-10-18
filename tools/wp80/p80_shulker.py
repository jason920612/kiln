"""wp80 item 2: shulker bullets hitting an open shulker (duplication: teleport + a copy left behind unless 5+ shulkers within 8 blocks), the bullet's damage,
levitation on a player, health after; lock step with 25 rooms at once. Console only."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
Y = 150
ROUNDS = int(sys.argv[1]) if len(sys.argv) > 1 else 3
both_("kill @e[type=!player]")
def room(i):
    return 4 + 11 * (i % 5), 4 + 11 * (i // 5)
tot = {"K": [], "V": []}
for rnd in range(ROUNDS):
    both_("kill @e[type=!player]")
    both_("tick unfreeze")
    for i in range(25):
        x, z = room(i)
        OB(f"fill {x} {Y - 1} {z} {x + 8} {Y + 5} {z + 8} minecraft:stone")
        OB(f"fill {x + 1} {Y} {z + 1} {x + 7} {Y + 4} {z + 7} minecraft:air")
    both_("tick freeze")
    time.sleep(1)
    for i in range(25):
        x, z = room(i)
        ua, ub = [rnd * 1000 + i * 10 + 1, 7, 7, i], [rnd * 1000 + i * 10 + 2, 7, 7, i]
        OB(f"summon minecraft:shulker {x + 4}.5 {Y} {z + 4}.5 {{PersistenceRequired:1b,Peek:100b,AttachFace:0b,UUID:[I;{ua[0]},{ua[1]},{ua[2]},{ua[3]}],Tags:[\"A\"]}}")
        OB(f"summon minecraft:shulker {x + 7}.5 {Y} {z + 7}.5 {{PersistenceRequired:1b,Peek:0b,NoAI:1b,AttachFace:0b,UUID:[I;{ub[0]},{ub[1]},{ub[2]},{ub[3]}],Tags:[\"B\"]}}")
        OB(f"summon minecraft:shulker_bullet {x + 4}.5 {Y + 3} {z + 4}.5 {{Owner:[I;{ub[0]},{ub[1]},{ub[2]},{ub[3]}],Target:[I;{ua[0]},{ua[1]},{ua[2]},{ua[3]}],Steps:0,Tags:[\"bul\"]}}")
    step(2)
    # each room: number of shulkers (the original, a copy, the bystander)
    for t in "KV":
        n = []
        for i in range(25):
            x, z = room(i)
            n.append(count(t, f"@e[type=minecraft:shulker,x={x},y={Y},z={z},dx=8,dy=5,dz=8]"))
        tot[t].append(n)
    step(50)
    for t in "KV":
        n = []
        for i in range(25):
            x, z = room(i)
            n.append(count(t, f"@e[type=minecraft:shulker,x={x},y={Y},z={z},dx=8,dy=5,dz=8]"))
        tot[t].append(n)
    print(f"round {rnd}: shulkers per room after 2 ticks / 52 ticks", flush=True)
    for t in "KV":
        a, b = tot[t][-2], tot[t][-1]
        print(f"  {t}: rooms with 3 shulkers {sum(1 for v in b if v == 3)}/25, with 2 {sum(1 for v in b if v == 2)}, total {sum(b)}; early total {sum(a)}", flush=True)
both_("tick unfreeze")
both_("kill @e[type=!player]")
for i in range(25):
    x, z = room(i)
    OB(f"fill {x} {Y - 1} {z} {x + 8} {Y + 5} {z + 8} minecraft:air")
