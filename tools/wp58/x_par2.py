from p68_mounts import *
reset()
platform(0, 0, 40, 40, y=199, clear=12)
for typ in ("parched", "skeleton", "bogged", "stray"):
    clear_area()
    sv(f"summon minecraft:{typ} 20.5 200 28.5 {{PersistenceRequired:1b,NoAI:1b}}", show=False)
    time.sleep(1.0)
    out = sv(f"data get entity @e[type={typ},limit=1] attributes", show=False)
    out2 = sv(f"data get entity @e[type={typ},limit=1] equipment", show=False)
    F(typ, re.findall(r'\{id: "minecraft:(?:follow_range|movement_speed|attack_damage|max_health)"[^}]*\}', out), "equip:", out2[-160:])
