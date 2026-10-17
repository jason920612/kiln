from lock import *
back_to_overworld()
for t in "KV":
    w(t, "gamerule command_block_output false"); w(t, "gamerule log_admin_commands false")
    w(t, "summon minecraft:pig 5 200 5 {NoAI:1b}"); w(t, "summon minecraft:pig 6 200 5 {NoAI:1b}")
time.sleep(0.5)
for t in "KV":
    print(t, "gamerules off:", repr(ask(t, "kill @e[type=pig]", wait=0.5)[-200:]), "count", kill_count(t, "@e[type=pig]"))
    w(t, "gamerule command_block_output true"); w(t, "gamerule log_admin_commands true")
    w(t, "summon minecraft:pig 5 200 5 {NoAI:1b}"); w(t, "summon minecraft:pig 6 200 5 {NoAI:1b}")
time.sleep(0.5)
for t in "KV":
    print(t, "gamerules on:", repr(ask(t, "kill @e[type=pig]", wait=0.5)[-200:]))
