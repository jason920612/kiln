from p68_small import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
for typ in ("camel_husk", "camel", "nautilus", "zombie_nautilus", "happy_ghast", "sulfur_cube", "copper_golem", "parched", "bogged", "breeze", "armadillo", "creaking", "sniffer", "mannequin", "wolf", "horse", "donkey", "llama", "strider", "pig"):
    clear_area()
    sv(f"summon minecraft:{typ} 24.5 200 24.5 {{PersistenceRequired:1b,NoAI:1b}}", show=False)
    time.sleep(1.2)
    es = ents_of(a, typ, 40)
    if not es:
        F(typ, "none"); continue
    at = json.loads(a.cmd(f"attrs {es[0]['id']}"))
    keep = {k.split(":")[1]: round(v["v"], 4) for k, v in at.items() if v["v"] != 0 and v["v"] != 1.0 or k.endswith(("movement_speed", "jump_strength", "flying_speed", "max_health", "step_height"))}
    F(typ, dict(sorted(keep.items())))
