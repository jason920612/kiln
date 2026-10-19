from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false", "do_daylight_cycle false", "random_tick_speed 3"))
Y = 150; X0 = 6; Z0 = 6
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 12} {Y + 10} {Z0 + 12} minecraft:air")
OB(f"fill {X0} {Y - 1} {Z0} {X0 + 10} {Y - 1} {Z0 + 10} minecraft:sand")
OB(f"setblock {X0 + 2} {Y} {Z0 + 2} minecraft:sniffer_egg[hatch=0]")
OB(f"setblock {X0 + 6} {Y - 1} {Z0 + 2} minecraft:moss_block")
OB(f"setblock {X0 + 6} {Y} {Z0 + 2} minecraft:sniffer_egg[hatch=0]")
both_("tick freeze")
time.sleep(0.5)
for chunk in range(14):
    for t in "KV":
        sprint(t, 1000)
    r = {}
    for t in "KV":
        row = []
        for x in (2, 6):
            st = None
            for h in (0, 1, 2):
                o = ask(t, f"execute in minecraft:overworld run execute if block {X0 + x} {Y} {Z0 + 2} minecraft:sniffer_egg[hatch={h}]", wait=0.15)
                if "conditional.pass" in o or "Test passed" in o:
                    st = h
            row.append(st)
        r[t] = row
    print((chunk + 1) * 1000, "K", r["K"], "V", r["V"], flush=True)
both_("tick unfreeze")
OB(f"fill {X0 - 2} {Y - 2} {Z0 - 2} {X0 + 12} {Y + 10} {Z0 + 12} minecraft:air")
