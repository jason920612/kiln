"""wp84 #5: TWO real players against one Wither on the same server (switch.sh K|V both: KilnTest = agent 25999, Obs = agent 26000; TARGET=K|V picks the console).
A bedrock cage, the wither summoned the way the game spawns one (Invul 220), both players in survival with 400 health; every 3 s: both players' health and wither effect, the boss
bars each client shows, the wither's health, its charging state, how many wither skulls each client saw (AddEntity of wither_skull / dangerous ones), what targets it has.
Then the half-health armour phase (arrows deflected), then both players leave the cage's end: the bars disappear.
usage: TARGET=K python3 p84_wither2.py [seconds]"""
import json, os, re, sys, time, collections
from lock import *

T_ = os.environ.get("TARGET", "K")
a, b = C(25999), C(26000)
SECS = int(sys.argv[1]) if len(sys.argv) > 1 else 90
X, Y, Z = 600, 200, 600


def S(c, wait=0.3):
    return ask(T_, c, wait=wait)


def OBx(c, wait=0.3):
    return S("execute in minecraft:overworld run " + c, wait)


def nbt_(path, sel="@e[type=wither,limit=1]"):
    o = S(f"execute in minecraft:overworld run data get entity {sel} {path}", wait=0.35)
    return o.strip().splitlines()[-1][-160:] if o.strip() else ""


S("gamerule spawn_mobs false"); S("gamerule do_daylight_cycle false"); S("gamerule mob_griefing true"); S("time set midnight"); S("difficulty normal")
for n in ("KilnTest", "Obs"):
    if T_ == "V":
        S(f"op {n}")
    S(f"gamemode creative {n}")
S(f"forceload add {X - 24} {Z - 24} {X + 24} {Z + 24}")
OBx(f"tp KilnTest {X} {Y + 5} {Z}")
OBx(f"tp Obs {X + 2} {Y + 5} {Z}")
time.sleep(5)
OBx(f"fill {X - 20} {Y - 1} {Z - 20} {X + 20} {Y + 20} {Z + 20} minecraft:air")
OBx(f"fill {X - 20} {Y - 1} {Z - 20} {X + 20} {Y - 1} {Z + 20} minecraft:stone")
OBx(f"fill {X - 21} {Y - 1} {Z - 21} {X + 21} {Y + 21} {Z + 21} minecraft:bedrock hollow")
S("kill @e[type=!player]")
for n in ("KilnTest", "Obs"):
    S(f"clear {n}")
    S(f"effect clear {n}")
    S(f"gamemode survival {n}")
    S(f"attribute {n} minecraft:max_health base set 400")
    S(f"effect give {n} minecraft:instant_health 1 20 true")
    for slot, it in (("head", "iron_helmet"), ("chest", "iron_chestplate"), ("legs", "iron_leggings"), ("feet", "iron_boots")):
        S(f"item replace entity {n} armor.{slot} with minecraft:{it}")
OBx(f"tp KilnTest {X - 8} {Y} {Z - 6}")
OBx(f"tp Obs {X + 8} {Y} {Z + 6}")
time.sleep(3)
for c in (a, b):
    c.cmd("spy on"); c.cmd("spylog clear")
OBx(f"summon minecraft:wither {X} {Y + 1} {Z} {{Invul:220,PersistenceRequired:1b}}")
t0 = time.time()
skulls = {"A": collections.Counter(), "B": collections.Counter()}
rows = []
armour_done = False
seen_ids = set()
dangerous = collections.Counter()
while time.time() - t0 < SECS:
    for _ in range(30):
        time.sleep(0.1)
        for e in a.state(60)["entities"]:
            if e["t"] not in ("player", "wither", "item") and e["id"] not in seen_ids:
                seen_ids.add(e["id"])
                dangerous[(e["t"], round(time.time() - t0) // 20 * 20)] += 1
    row = {"t": round(time.time() - t0)}
    for k, c in (("A", a), ("B", b)):
        try:
            st = c.state(0)
            row[k] = {"hp": round(st["health"], 1), "bar": json.loads(c.cmd("bossbars")) if c.cmd("bossbars").startswith("[") else c.cmd("bossbars")[:120]}
            for l in json.loads(c.cmd("spylog clear")):
                if l.startswith("AddEntity"):
                    skulls[k]["add"] += 1
                if "wither_skull" in l:
                    skulls[k]["skull_data"] += 1
                if l.startswith("HurtAnimation"):
                    skulls[k]["hurt"] += 1
                if l.startswith("sound") and "wither" in l:
                    skulls[k][l.split(" ")[1].replace("minecraft:", "")] += 1
        except Exception as ex:
            row[k] = repr(ex)[:80]
    row["wither"] = {"health": nbt_("Health"), "invul": nbt_("Invul"), "pos": re.findall(r"-?\d+\.\d", nbt_("Pos"))[:3]}
    rows.append(row)
    print(T_, row, flush=True)
    if not armour_done and time.time() - t0 > SECS * 0.55:
        armour_done = True
        S("execute in minecraft:overworld run data merge entity @e[type=wither,limit=1] {Health:140f}")
for k in "AB":
    print(T_, "client", k, "summary", dict(skulls[k]))
print(T_, "wither skulls seen by A:", len(seen_ids), "per 10 s:", sorted(dangerous.items()))
S("kill @e[type=!player]")
OBx(f"tp KilnTest {X} {Y + 5} {Z + 40}")
OBx(f"tp Obs {X} {Y + 5} {Z + 40}")
