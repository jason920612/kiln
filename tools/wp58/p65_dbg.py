from p65 import *
for pos in ["274 57 218", "267 57 220", "275 64 198", "282 72 190", "232 50 210"]:
    sv("execute in minecraft:the_nether run forceload add 232 190 290 230", show=False)
    out = sv(f"execute in minecraft:the_nether run data get block {pos}", wait=1.0, show=False)
    print(pos, "=>", out.strip()[:300])
