from p68_small import *
prep(); time.sleep(3)
reset()
for typ in ("cow", "pig", "horse", "sheep", "happy_ghast", "sulfur_cube", "villager"):
    reset()
    m = summon(typ, "{Age:-30}")
    if not m:
        F(typ, "no"); continue
    eid = m["id"]
    F(typ, "baby at start", m.get("baby"), m["bbh"])
    time.sleep(4.5)
    e = [x for x in a.state(40)["entities"] if x["id"] == eid]
    F(typ, "after 4.5 s: client baby", e and e[0].get("baby"), e and e[0]["bbh"], a.cmd(f"entdata {eid}")[:60], "server:", re.findall(r"Age: -?\d+", sv(f"data get entity @e[type={typ},limit=1]", show=False))[:2])
