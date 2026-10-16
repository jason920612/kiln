from p68_small import *
prep(); time.sleep(3)
reset()
m = summon("copper_golem", "{}")
F("client:", {k: m[k] for k in ("pose","bbh","bbw","hp")}, a.cmd(f"entdata {m['id']}")[:200])
out = sv("data get entity @e[type=copper_golem,limit=1]", show=False)
F(out[-1500:])
