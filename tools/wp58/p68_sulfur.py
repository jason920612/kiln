"""wp68 #1: sulfur cube (spawn egg, swallow items, TNT/flint, magma, shears, bucket, push), Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p68 import *
import sys

a = C(25999)
scenario("sulfur")
ME = "KilnTest"

def prep():
    if T == "V":
        sv("op KilnTest", show=False)
    alive(a)
    sv("gamerule spawn_mobs false", show=False)
    sv("gamerule mob_griefing true", show=False)
    sv("difficulty normal", show=False)
    sv("time set day", show=False)
    sv("weather clear", show=False)
    platform(0, 0, 40, 40, y=199, clear=12)
    kit(ME)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(2.5)

def spawn_cube(item_cmd=None):
    """Give the spawn egg and use it on the floor in front of the player (real right click)."""
    give(ME, "minecraft:sulfur_cube_spawn_egg")
    time.sleep(0.4)
    a.select("minecraft:sulfur_cube_spawn_egg")
    st = pos(a)
    # look at the floor 3 blocks ahead (+z)
    a.lookat(st[0], 199.5 + 0.5, st[2] + 3)
    time.sleep(0.5)
    before = {e["id"] for e in ents_of(a, "sulfur_cube")}
    a.mouse("right", "tap")
    time.sleep(1.2)
    new = [e for e in ents_of(a, "sulfur_cube") if e["id"] not in before]
    return new[0] if new else None

def cube_info(eid):
    d = a.cmd(f"entdata {eid}")
    e = json.loads(a.cmd(f"ent {eid}"))
    return d, e



def fresh_cube(x=20.5, z=23.5, ai=False, extra=""):
    """A cube standing still in front of the player (NoAI unless ai) - summoned, the egg is used once in S1."""
    clear_area()
    if a.screen() == "DeathScreen" or a.state(0)["health"] <= 0:
        F("(player was dead, respawning)")
        alive(a)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    a.release()
    time.sleep(0.6)
    before = {e["id"] for e in ents_of(a, "sulfur_cube", 40)}
    sv(f"summon minecraft:sulfur_cube {x} 200 {z} {{Size:2{'' if ai else ',NoAI:1b'}{extra}}}", show=False)
    time.sleep(1.2)
    new = [e for e in ents_of(a, "sulfur_cube", 40) if e["id"] not in before]
    return new[0]["id"] if new else None

def cube_pos(eid):
    for e in ents_of(a, "sulfur_cube", 40):
        if e["id"] == eid:
            return e
    return None

def swallow(eid, item):
    give(ME, item, 1)
    time.sleep(0.3)
    return use_on_entity(a, eid, item)

def lines_since():
    return json.loads(a.cmd("spylog clear"))

def run():
    prep()
    time.sleep(4)
    # S1: punch
    clear_area()
    cube = spawn_cube()
    eid = cube["id"]
    time.sleep(1.5)
    p0 = cube_pos(eid)
    interact_ent(a, eid, button="left", tries=3)
    time.sleep(1.5)
    p1 = cube_pos(eid)
    F("S1 punch: moved", dist((p0["x"], 0, p0["z"]), (p1["x"], 0, p1["z"])), "hp", p0["hp"], "->", p1["hp"] if p1 else None)
    # S2: archetype swallow: equipment line + inventory
    for item in ("minecraft:stone", "minecraft:slime_block", "minecraft:iron_block", "minecraft:tnt", "minecraft:magma_block", "minecraft:oak_log", "minecraft:glass", "minecraft:diamond"):
        eid = fresh_cube()
        rec_start(a)
        ok = swallow(eid, item)
        time.sleep(0.8)
        ls = lines_since()
        F("S2 swallow", item, "ok", ok, [l for l in ls if l.startswith("equipment") or "absorb" in l or "eject" in l][:4], "inv", a.inv().get(item, 0))
    # S3: TNT cube, fuse and blast
    eid = fresh_cube()
    swallow(eid, "minecraft:tnt")
    time.sleep(0.5)
    sv(f"effect give {ME} minecraft:absorption 60 4 true", show=False)
    give(ME, "minecraft:flint_and_steel")
    h0 = a.state(0)["health"]
    use_on_entity(a, eid, "minecraft:flint_and_steel")
    g0 = a.state(0)["gameTime"]
    a.look(-90, 0)
    a.key("w", "down"); time.sleep(0.6); a.key("w", "up")
    boomed = None
    for i in range(160):
        time.sleep(0.1)
        if not cube_pos(eid):
            boomed = a.state(0)["gameTime"] - g0
            break
    time.sleep(1.0)
    ls = lines_since()
    F("S3 tnt cube: exploded after ticks", boomed, "health", h0, "->", a.state(0)["health"], [l[:100] for l in ls if "explode" in l or "tnt" in l][:3])
    craters = a.findbox("air", 12, 195, 12, 30, 199, 30, n=400)
    F("S3 crater blocks (air under y200 in box):", len(craters))
    sv(f"effect clear {ME}", show=False)
    platform(0, 0, 40, 40, y=199, clear=12)
    # S4: hot cube contact damage
    sv(f"tp {ME} 20.5 200 20.5", show=False); time.sleep(1.5)
    eid = fresh_cube(ai=True)
    swallow(eid, "minecraft:magma_block")
    sv(f"effect clear {ME}", show=False)
    h0 = a.state(0)["health"]
    t0 = time.time(); hmin = h0
    while time.time() - t0 < 12:
        e = cube_pos(eid)
        if e:
            a.walk_to(e["x"], e["z"], timeout=1.0, tol=0.8)
        hmin = min(hmin, a.state(0)["health"])
    F("S4 hot cube: health", h0, "min", hmin)
    clear_area()
    alive(a)
    sv(f"effect give {ME} minecraft:instant_health 1 5 true", show=False)
    # S5: shears
    eid = fresh_cube()
    swallow(eid, "minecraft:stone")
    time.sleep(0.5)
    give(ME, "minecraft:shears")
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:shears")
    time.sleep(1.0)
    F("S5 shears ok", ok, "ground items:", [(e.get("item"), e.get("n")) for e in a.state(10)["entities"] if e["t"] == "item"], [l for l in lines_since() if "equipment" in l or "shear" in l][:3])
    # S6: bucket
    eid = fresh_cube()
    give(ME, "minecraft:bucket")
    ok = use_on_entity(a, eid, "minecraft:bucket")
    time.sleep(0.8)
    inv = a.inv()
    F("S6 bucket ok", ok, "inv", {k: v for k, v in inv.items() if "bucket" in k}, "cubes left", len(ents_of(a, "sulfur_cube")))
    if inv.get("minecraft:sulfur_cube_bucket"):
        a.select("minecraft:sulfur_cube_bucket")
        st = pos(a)
        a.lookat(st[0], 199.5, st[2] + 3)
        time.sleep(0.4)
        a.mouse("right", "tap")
        time.sleep(1.2)
        F("S6 bucket placed: cubes", len(ents_of(a, "sulfur_cube")), "inv", {k: v for k, v in a.inv().items() if "bucket" in k})
    # S7: slime ball on baby
    clear_area()
    sv("summon minecraft:sulfur_cube 20.5 200 24.5 {Size:1,Age:-24000}", show=False)
    time.sleep(1.0)
    cs = ents_of(a, "sulfur_cube")
    F("S7 summoned baby:", [(c.get("baby"), c["bbh"]) for c in cs])
    if cs:
        give(ME, "minecraft:slime_ball", 4)
        ok = use_on_entity(a, cs[0]["id"], "minecraft:slime_ball")
        F("S7 slime ball ok", ok, "left", a.inv().get("minecraft:slime_ball", 0), a.cmd(f"entdata {cs[0]['id']}")[:60])
    clear_area()

if __name__ == "__main__":
    run()
