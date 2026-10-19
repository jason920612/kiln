"""wp80 item 3: Bad Omen -> Raid Omen -> a raid, in a village (real players, each server on its own: par.sh p80_omen.py). The effect list of the player every 2 s, the raid bar,
how many raiders there are, for SECS seconds. usage: p80_omen.py [secs] [amplifier]"""
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
SECS = int(sys.argv[1]) if len(sys.argv) > 1 else 90
AMP = int(sys.argv[2]) if len(sys.argv) > 2 else 1
scenario("omen")
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("difficulty normal", show=False); sv("gamerule raids true", show=False); sv("time set day", show=False)
kill_all()
kit(ME)
sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
out = sv("execute in minecraft:overworld positioned 0 64 0 run locate structure #minecraft:village", show=False, wait=8)
m = None
for m in re.finditer(r"\[(-?\d+), (-?\d+|~), (-?\d+)\]", out):
    pass
if not m:
    F("no village found", out[-200:])
    sys.exit(0)
vx, vz = int(m.group(1)), int(m.group(3))
F("village at", vx, vz)
sv(f"execute in minecraft:overworld run forceload add {vx - 40} {vz - 40} {vx + 40} {vz + 40}", show=False)
time.sleep(10)
sv(f"execute in minecraft:overworld run tp {ME} {vx}.5 150 {vz}.5", show=False)
time.sleep(14)
# stand on the ground
sv(f"execute in minecraft:overworld run tp {ME} {vx}.5 ~ {vz}.5", show=False)
F("player", pos(a), "villagers near", len(ents_of(a, "villager", 60)))
sv(f"effect give {ME} minecraft:bad_omen 6000 {AMP} true", show=False)
t0 = time.time()
last = None
while time.time() - t0 < SECS:
    time.sleep(2)
    st = a.state(0)
    effs = [(e["id"].replace("minecraft:", ""), e.get("amp", e.get("amplifier")), e.get("dur", e.get("duration"))) for e in st.get("effects", [])]
    bars = a.cmd("bossbars")
    raiders = len([e for e in a.state(100)["entities"] if e["t"] in ("pillager", "vindicator", "evoker", "ravager", "witch", "vex")])
    cur = (tuple((n, amp) for n, amp, _ in effs if n in ("bad_omen", "raid_omen", "hero_of_the_village")), raiders, bars[:80])
    if cur != last:
        F(f"t={int(time.time() - t0):3d}s", effs, "raiders", raiders, "bars", bars[:120])
        last = cur
sv("effect clear KilnTest", show=False)
kill_all()
sv(f"execute in minecraft:overworld run forceload remove {vx - 40} {vz - 40} {vx + 40} {vz + 40}", show=False)
