"""wp80 item 8 (real client, par.sh): a lodestone compass (use on a lodestone, break the lodestone, the other dimension), riptide in rain / water / dry, a lightning rod in a
thunderstorm, copper oxidation and waxing under lightning."""
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
scenario("small8")
which = sys.argv[1].split(",") if len(sys.argv) > 1 else ["lodestone", "riptide"]
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("difficulty normal", show=False); sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=14)
kill_all()


def comps_of_hand():
    st = a.state(0)
    sel = st.get("selected", 0)
    return a.cmd(f"item {sel}")[:300]


if "lodestone" in which:
    kit(ME)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    sv("setblock 24 200 20 minecraft:lodestone", show=False)
    give(ME, "minecraft:compass", 2)
    a.select("minecraft:compass")
    F("compass before:", comps_of_hand())
    a.lookat(24.5, 200.5, 20.5)
    time.sleep(0.6)
    rec_start(a)
    a.mouse("right", "tap")
    time.sleep(1.0)
    F("compass after use on the lodestone:", comps_of_hand())
    F("fx:", sorted(set(re.sub(r"-?\d+\.\d+(E-?\d+)?", "#", l)[:80] for l in json.loads(a.cmd("spylog clear")) if "sound" in l or "particle" in l))[:6])
    F("inventory compasses:", a.inv().get("minecraft:compass"))
    sv("setblock 24 200 20 minecraft:air", show=False)
    time.sleep(2.5)
    F("compass after the lodestone is gone:", comps_of_hand())
    sv("setblock 24 200 20 minecraft:lodestone", show=False)
    time.sleep(1.5)
    F("compass after a lodestone is back:", comps_of_hand())
    # the other dimension
    sv(f"execute in minecraft:the_nether run tp {ME} 0 80 0", show=False)
    time.sleep(6)
    F("in the nether:", a.state(0).get("dimension"), comps_of_hand())
    sv(f"execute in minecraft:overworld run tp {ME} 20.5 200 20.5", show=False)
    time.sleep(4)

if "riptide" in which:
    kit(ME)
    sv(f"gamemode survival {ME}", show=False)
    give(ME, "minecraft:trident[enchantments={riptide:3}]", 1)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    a.select("minecraft:trident")

    def charge(label):
        before = a.state(0)
        a.look(0, 0)
        rec_start(a)
        a.mouse("right", "down")
        time.sleep(1.4)
        a.mouse("right", "up")
        time.sleep(1.2)
        after = a.state(0)
        ls = json.loads(a.cmd("spylog clear"))
        F(label, "moved", round(math.hypot(after["x"] - before["x"], after["z"] - before["z"]), 2), "dy", round(after["y"] - before["y"], 2), "fx",
          sorted(set(re.sub(r"-?\d+\.\d+(E-?\d+)?", "#", l)[:70] for l in ls if "sound" in l or "particle" in l or "SetEntityMotion" in l))[:5])
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        time.sleep(1.5)

    charge("riptide dry land")
    sv("weather rain", show=False)
    time.sleep(8)
    charge("riptide in rain (no sky cover)")
    sv("weather clear", show=False)
    sv("fill 18 200 18 22 202 22 minecraft:water", show=False)
    time.sleep(1.0)
    sv(f"tp {ME} 20.5 200 20.5", show=False)
    time.sleep(1.5)
    charge("riptide in water")
    sv("fill 18 200 18 22 202 22 minecraft:air", show=False)
kill_all()
