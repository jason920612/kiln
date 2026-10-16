"""wp68 #2: happy ghast with two real clients: dried ghast hydration -> ghastling, snowball feeding, growing up, harness, riding with
two players, steering, dismount, lead; Kiln (TARGET=K) vs vanilla (TARGET=V)."""
from p68 import *
import sys

a, b = two()
scenario("ghast")


def lines():
    return json.loads(a.cmd("spylog clear"))


def flt(ls, *keys):
    return [l[:140] for l in ls if any(k in l for k in keys)]


def server_ghast(path=""):
    return sv(f"data get entity @e[type=happy_ghast,limit=1] {path}".strip(), show=False)


def kill_all_mobs():
    for _ in range(3):
        sv("kill @e[type=!player]", show=False, wait=0.3)


def sprint_ticks(n):
    sv(f"tick sprint {n}", show=False, wait=1.5)
    for _ in range(120):
        time.sleep(1.0)
        if "sprint" not in sv("tick query", show=False, wait=0.4).lower():
            break


def hydrate():
    kill_all_mobs()
    sv("fill 10 200 10 36 204 36 minecraft:air", show=False)
    # real: place the dried ghast on the floor, then pour water on it
    give("KilnTest", "minecraft:dried_ghast", 1)
    give("KilnTest", "minecraft:water_bucket", 1)
    time.sleep(0.5)
    a.select("minecraft:dried_ghast")
    sv("tp KilnTest 20.5 200 20.5", show=False)
    time.sleep(1.5)
    a.lookat(21.5, 199.99, 23.5)
    time.sleep(0.8)
    F("aim:", a.state(0).get("hit"), pos(a))
    a.mouse("right", "tap")
    time.sleep(0.8)
    hits = a.find("dried_ghast", 8, 4)
    F("dried ghast placed at", hits, blk(a, *hits[0]) if hits else None)
    if not hits:
        return None
    x, y, z = hits[0]
    a.select("minecraft:water_bucket")
    a.lookat(x + 0.5, y + 0.5, z + 0.5)
    time.sleep(0.5)
    a.mouse("right", "tap")
    time.sleep(0.8)
    F("after the bucket:", blk(a, x, y, z), "above:", blk(a, x, y + 1, z))
    sv("tp KilnTest 6.5 200 6.5", show=False)
    time.sleep(1.0)
    rec_start(a)
    sv("gamerule random_tick_speed 3000", show=False)
    # hydration steps (5000 ticks each, started by a random tick) then the spawn; short sprints so the new
    # ghastling is still near when the block turns to water
    for step in range(120):
        sprint_ticks(250)
        b_ = blk(a, x, y, z)
        if step % 5 == 0 or "dried_ghast" not in b_:
            F("hydration step", step, b_, "ghasts:", len(ents_of(a, "happy_ghast", 30)), "player", pos(a), a.state(0)["health"], [(e["t"], round(e["x"], 1), round(e["y"], 1), round(e["z"], 1)) for e in a.state(30)["entities"]][:6])
        if "dried_ghast" not in b_:
            break
    time.sleep(1.0)
    F("server ghast pos:", re.findall(r"Pos: \[[^\]]*\]", server_ghast()), "player", pos(a))
    sv("gamerule random_tick_speed 3", show=False)
    ls = lines()
    F("hydration lines:", [l[:90] for l in ls if "sound" in l or "particle" in l][:10])
    gs = ents_of(a, "happy_ghast", 30)
    if not gs:
        sv("tp @e[type=happy_ghast,limit=1] 8.5 202 8.5", show=False)
        time.sleep(1.5)
        gs = ents_of(a, "happy_ghast", 30)
    return gs[0] if gs else None


def baby_and_growth(g):
    eid = g["id"]
    F("ghastling:", {k: g[k] for k in ("pose", "bbh", "bbw", "hp") if k in g}, "baby", g.get("baby"), a.cmd(f"entdata {eid}")[:120])
    F("server nbt:", re.findall(r"Age: -?\d+|Health: [\d.]+f", server_ghast()))
    give("KilnTest", "minecraft:snowball", 8)
    ok = use_on_entity(a, eid, "minecraft:snowball", dy=0.5)
    time.sleep(1.0)
    F("snowball ok", ok, "server Age:", re.findall(r"Age: -?\d+", server_ghast()), "snowballs on the server:", re.findall(r'count: (\d+), Slot: \d+b, id: "minecraft:snowball"', sv("data get entity KilnTest Inventory", show=False)))
    sv("data merge entity @e[type=happy_ghast,limit=1] {Age:-60}", show=False)
    time.sleep(5)
    g2 = ents_of(a, "happy_ghast", 30)[0]
    F("after growing:", {k: g2[k] for k in ("pose", "bbh", "bbw", "hp") if k in g2}, "baby", g2.get("baby"), "server Age:", re.findall(r"Age: -?\d+", server_ghast()))
    return g2


def ride():
    g = ents_of(a, "happy_ghast", 30)[0]
    eid = g["id"]
    sv("tp @e[type=happy_ghast] 26.5 202 26.5", show=False)
    sv("data merge entity @e[type=happy_ghast,limit=1] {NoAI:1b}", show=False)
    give("KilnTest", "minecraft:blue_harness", 1)
    sv("tp KilnTest 26.5 200 22.5", show=False)
    sv("tp Obs 28.5 200 22.5", show=False)
    time.sleep(2.0)
    rec_start(a)
    ok = use_on_entity(a, eid, "minecraft:blue_harness", dy=1.5)
    time.sleep(1.0)
    F("harness ok", ok, "server body:", re.findall(r"body: \{[^}]*\}", server_ghast()), "A harnesses left (client):", a.inv().get("minecraft:blue_harness", 0), flt(lines(), "equipment", "sound")[:5])
    rec_start(a)
    ok = interact_ent(a, eid, button="right", tries=8, dy=1.5)
    time.sleep(1.0)
    F("A rides?", ok, a.state(0)["vehicle"], "passengers:", json.loads(a.cmd(f"ent {eid}")).get("passengers"), flt(lines(), "sound", "Passengers")[:5])
    ok = interact_ent(b, eid, button="right", tries=8, dy=1.5)
    time.sleep(1.0)
    F("B rides?", ok, b.state(0)["vehicle"], "passengers as A sees:", json.loads(a.cmd(f"ent {eid}")).get("passengers"), "as B sees:", json.loads(b.cmd(f"ent {eid}")).get("passengers"))
    F("positions A,B:", pos(a), pos(b), "ghast", [(round(e["x"], 2), round(e["y"], 2), round(e["z"], 2)) for e in ents_of(a, "happy_ghast", 30)])
    return eid


def ghast_of(c, eid):
    r = [e for e in ents_of(c, "happy_ghast", 60) if e["id"] == eid]
    return r[0] if r else None


def steer(eid):
    # (NoAI stays on: `/data merge entity` on a vehicle drops its riders on Kiln.)
    time.sleep(1.0)
    g0 = ghast_of(a, eid)
    p0 = (g0["x"], g0["y"], g0["z"])
    a.look(-90, -10)
    a.key("w", "down")
    time.sleep(4.0)
    a.key("w", "up")
    g1 = ghast_of(a, eid)
    p1 = (g1["x"], g1["y"], g1["z"])
    F("steered 4 s forward (yaw -90, pitch -10): moved", [round(p1[i] - p0[i], 2) for i in range(3)], "server pos:", re.findall(r"Pos: \[[^\]]*\]", server_ghast()))
    a.key("space", "down")
    time.sleep(2.0)
    a.key("space", "up")
    g2 = ghast_of(a, eid)
    F("space 2 s: dy", round(g2["y"] - p1[1], 2), "rider yaw", round(a.state(0)["yaw"], 1))
    gb = ghast_of(b, eid)
    F("B sees ghast at", gb and (round(gb["x"], 1), round(gb["y"], 1), round(gb["z"], 1)), "A at", pos(a), "B at", pos(b), "server:", re.findall(r"Pos: \[[^\]]*\]", server_ghast()))


def dismount(eid):
    a.key("shift", "down")
    time.sleep(0.3)
    a.key("shift", "up")
    time.sleep(1.0)
    b.key("shift", "down")
    time.sleep(0.3)
    b.key("shift", "up")
    time.sleep(1.0)
    F("after shift: vehicles", a.state(0)["vehicle"], b.state(0)["vehicle"], "positions", pos(a), pos(b))


def lead():
    kill_all_mobs()
    sv("tp KilnTest 26.5 200 22.5", show=False)
    sv("summon minecraft:happy_ghast 26.5 201 24.5 {NoAI:1b}", show=False)
    time.sleep(1.5)
    g = ents_of(a, "happy_ghast", 30)[0]
    sv("setblock 28 200 26 minecraft:oak_fence", show=False)
    give("KilnTest", "minecraft:lead", 2)
    rec_start(a)
    ok = use_on_entity(a, g["id"], "minecraft:lead", dy=1.5)
    time.sleep(1.0)
    F("lead ok", ok, "leash (server):", re.findall(r"[Ll]eash: \{[^}]*\}", server_ghast()), flt(lines(), "Leash", "sound")[:4])


if __name__ == "__main__":
    setup_two()
    try:
        g = hydrate()
        F("ghast from hydration:", g and g["id"])
        if g:
            baby_and_growth(g)
            eid = ride()
            steer(eid)
            dismount(eid)
        lead()
    except Exception as ex:
        import traceback
        traceback.print_exc()
        F("ERROR", repr(ex)[:300])
