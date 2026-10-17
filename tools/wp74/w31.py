from lock import *
for t in "KV":
    res = {ty: count(t, f"@e[type=minecraft:{ty}]") for ty in ("item","experience_orb","zombified_piglin","piglin","ghast","magma_cube","enderman","strider","hoglin","skeleton","wither_skeleton","blaze","cow","pig","zombie","sheep","bat","slime","creeper")}
    print(t, {k:v for k,v in res.items() if v})
