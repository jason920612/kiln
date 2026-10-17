from lock import *
both_("setblock -1293 66 735 air"); time.sleep(0.5)
def short(o): return [l[-90:] for l in o.splitlines() if l.strip()][:2]
for c in ["setblock -1293 66 735 minecraft:stone", "setblock -1293 66 735 minecraft:stone",
          "setblock -1293 66 735 minecraft:chest{Items:[{Slot:0b,id:\"minecraft:stone\",count:1}]}", "setblock -1293 66 735 minecraft:chest{Items:[{Slot:0b,id:\"minecraft:stone\",count:2}]}",
          "setblock -1293 66 735 minecraft:chest", "setblock -1293 66 735 minecraft:chest replace", "setblock -1293 66 735 minecraft:chest keep",
          "setblock -1293 66 735 minecraft:repeating_command_block{Command:\"say hi\"}", "setblock -1293 66 735 minecraft:repeating_command_block{Command:\"say hi\"}",
          "setblock -1293 66 735 minecraft:repeating_command_block"]:
    r = ask_both(c)
    print(c[:70], "\n  K", short(r["K"]), "\n  V", short(r["V"]))
both_("setblock -1293 66 735 air")
