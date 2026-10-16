from p68_small import *
reset()
for typ in ("camel", "camel_husk", "horse", "donkey", "zombie_horse", "skeleton_horse", "llama", "mule"):
    clear_area()
    sv(f"summon minecraft:{typ} 24.5 200 24.5 {{PersistenceRequired:1b,NoAI:1b}}", show=False)
    time.sleep(1.2)
    es = ents_of(a, typ, 40)
    F(typ, a.cmd(f"entdata {es[0]['id']}")[:100] if es else None)
