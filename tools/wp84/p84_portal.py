"""wp84 item 1d: nether portal travel timing with a real survival player (par.sh): a frame built on a platform (setup), the player walks in; time until the dimension changes (N trials);
then trials where a zombie keeps hitting the player in the portal (knockback may push it out) and a pig that stands in the portal."""
import sys
from p72 import *
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
N = int(sys.argv[1]) if len(sys.argv) > 1 else 4
X, Y, Z = 1000, 200, 1000
def say(*x): print(*x, flush=True)
def O(cmd, wait=0.4): return sv("execute in minecraft:overworld run " + cmd, show=False, wait=wait)
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("difficulty normal", show=False); sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing false", show=False)
sv("gamerule players_nether_portal_default_delay 80", show=False) if False else None
O(f"forceload add {X - 10} {Z - 10} {X + 10} {Z + 10}")
sv(f"gamemode creative {ME}", show=False)
O(f"tp {ME} {X} {Y + 3} {Z}")
time.sleep(12)
O(f"fill {X - 8} {Y - 1} {Z - 8} {X + 8} {Y + 8} {Z + 8} minecraft:air", wait=1)
O(f"fill {X - 8} {Y - 1} {Z - 8} {X + 8} {Y - 1} {Z + 8} minecraft:stone", wait=1)
# portal frame along x: obsidian 4 wide 5 tall, portal blocks inside
O(f"fill {X} {Y - 1} {Z} {X + 3} {Y + 3} {Z} minecraft:obsidian")
O(f"fill {X + 1} {Y} {Z} {X + 2} {Y + 2} {Z} minecraft:nether_portal[axis=x]")
def dim(): return a.state(0)["dimension"].split("/")[-1].strip(" ]")
def back():
    sv(f"gamemode creative {ME}", show=False)
    sv(f"execute in minecraft:overworld run tp {ME} {X + 1.5} {Y} {Z - 3}", show=False)
    time.sleep(6)
    sv("kill @e[type=zombie]", show=False)
    sv(f"gamemode survival {ME}", show=False)
    sv(f"effect clear {ME}", show=False)
    time.sleep(1.5)
def trial(label, hit=False):
    back()
    if hit:
        sv(f"execute in minecraft:overworld run summon minecraft:zombie {X + 1.5} {Y} {Z + 1.5} {{PersistenceRequired:1b,Tags:[\"hitter\"]}}", show=False)
    O(f"tp {ME} {X + 1.5} {Y} {Z - 2.5}")
    time.sleep(0.5)
    t0 = time.time()
    a.look(0, 5)
    changed = None
    hpmin = 20
    a.key("w", "down")
    entered = None
    while time.time() - t0 < 14:
        s = a.state(0)
        hpmin = min(hpmin, s["health"])
        if entered is None and s["z"] >= Z + 0.45:
            entered = time.time() - t0
            a.key("w", "up")
        if "overworld" not in s["dimension"]:
            changed = time.time() - t0
            break
        time.sleep(0.05)
    a.key("w", "up")
    if changed is not None and entered is not None:
        changed = changed - entered
    sv("kill @e[tag=hitter]", show=False)
    say(label, "dimension changed after", None if changed is None else round(changed, 2), "s; lowest health", round(hpmin, 1))
    return changed
for i in range(N):
    trial(f"survival walk-in {i}")
for i in range(2):
    trial(f"with a zombie hitting (counted from standing in the portal) {i}", hit=True)
# a pig standing in the portal: where is it after 10 s (it travels, then has a cooldown)
sv(f"gamemode creative {ME}", show=False)
O(f"tp {ME} {X + 1.5} {Y} {Z - 3}")
time.sleep(4)
sv("kill @e[type=pig]", show=False)
O(f"summon minecraft:pig {X + 1.5} {Y + 1} {Z + 0.5} {{NoAI:0b,PersistenceRequired:1b}}")
for s in range(8):
    time.sleep(1.5)
    o1 = sv("execute in minecraft:overworld if entity @e[type=pig]", show=False, wait=0.4)
    o2 = sv("execute in minecraft:the_nether if entity @e[type=pig]", show=False, wait=0.4)
    say("pig after", round((s + 1) * 1.5, 1), "s: overworld", ("Test passed" in o1 or "pass" in o1) and "fail" not in o1.lower(), "nether", ("Test passed" in o2 or "pass" in o2) and "fail" not in o2.lower())
