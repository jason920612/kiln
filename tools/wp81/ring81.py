"""wp81: what a chunk ticket other than the player's loads and ticks, Kiln (K) against vanilla (V), lock step (`tick freeze` + `tick step`).
usage: ring81.py MODE [args]
  forced           /forceload add at (CX,CZ): probes (a baby cow's Age = entity ticking, a water source's spread = block ticking, `execute if loaded`) in the forced chunk
                   and the chunks east of it (rings 0..4)
  pearl [KILL_AT]  the same probes around a player's ender pearl (summoned hovering, Owner = W0) after the forceload that made the probes is removed; then the pearl is killed
                   after KILL_AT ticks (default 10) and the cow of its chunk is read every tick: how long the ticket works after the pearl's last renewal
  portal           a baby cow walks into an overworld nether portal built beside W0; read in the nether (nobody there) how long its Age keeps rising (the portal ticket, 300 ticks)
Both servers: seed 77 (sv.sh), the bot W0 in creative at (1000, 220, 1000) far from the probes at (CX*16+8, 3000)."""
import os, re, sys, time
from lk import *

CX, CZ = int(os.environ.get("CX", "187")), int(os.environ.get("CZ", "187"))   # chunk of block (3000, 3000)
Y = 190                    # probes' height
MODE = sys.argv[1] if len(sys.argv) > 1 else "forced"


def block_x(ring):
    return (CX + ring) * 16 + 8


def ageof(t, tag, dim=None):
    pre = f"execute in minecraft:{dim} run " if dim else ""
    out = ask(t, f"{pre}data get entity @e[tag={tag},limit=1] Age", wait=0.2)
    m = re.findall(r"(-?\d+)(?!.*\d)", out)
    return int(m[-1]) if m else None


def isloaded(t, x, y, z, dim=None):
    pre = f"execute in minecraft:{dim} run " if dim else ""
    out = ask(t, f"{pre}execute if loaded {x} {y} {z}", wait=0.15)
    return "pass" in out.lower()


def place_probes(rings):
    for t in "KV":
        for d in rings:
            x = block_x(d)
            w(t, f"summon cow {x + 0.5} {Y} {CZ * 16 + 8.5} {{Age:-24000,NoGravity:1b,PersistenceRequired:1b,Tags:[\"rt{d}\"]}}")
            w(t, f"fill {x - 2} {Y - 12} {CZ * 16 + 6} {x + 3} {Y - 12} {CZ * 16 + 10} minecraft:stone")
            w(t, f"fill {x - 2} {Y - 11} {CZ * 16 + 6} {x + 3} {Y - 8} {CZ * 16 + 10} minecraft:glass")
            w(t, f"fill {x - 1} {Y - 11} {CZ * 16 + 7} {x + 2} {Y - 11} {CZ * 16 + 9} minecraft:air")
            w(t, f"setblock {x} {Y - 11} {CZ * 16 + 8} minecraft:water")


def water_spread(t, ring):
    out = ask(t, f"execute if block {block_x(ring) + 1} {Y - 11} {CZ * 16 + 8} minecraft:water", wait=0.15)
    return "pass" in out.lower()


def placed(t, ring):
    """The water source placed in the chunk is there: the chunk accepted the command (it was loaded)."""
    out = ask(t, f"execute if block {block_x(ring)} {Y - 11} {CZ * 16 + 8} minecraft:water", wait=0.15)
    return "pass" in out.lower()


def report(label, rings=range(0, 5)):
    res = {}
    for t in "KV":
        res[t] = {d: (ageof(t, f"rt{d}") if d < 3 else None, placed(t, d), water_spread(t, d), isloaded(t, block_x(d), Y - 5, CZ * 16 + 8)) for d in rings}
    print(label, "  (Age, source placed, water spread, execute if loaded = entity ticking)")
    for d in rings:
        print(" ring", d, "K", res["K"][d], "  V", res["V"][d])
    sys.stdout.flush()
    return res


def setup():
    for t in "KV":
        w(t, "gamerule spawn_mobs false")
        w(t, "gamerule advance_time false")
        w(t, "gamemode creative W0")
        w(t, "tp W0 1008.5 220 1008.5")
    time.sleep(8)


if MODE in ("forced", "pearl"):
    setup()
    both(f"forceload add {block_x(0)} {CZ * 16 + 8}")
    time.sleep(40)
    place_probes([0, 1, 2])
    time.sleep(1)
    if MODE == "forced":
        both("tick freeze")
        time.sleep(0.5)
        base = report("before")
        step(40)
        time.sleep(1)
        report("after 40 ticks of the forced ticket")
        both(f"forceload remove {block_x(0)} {CZ * 16 + 8}")
        step(60)
        time.sleep(1)
        report("60 ticks after /forceload remove (nothing holds the chunks any more; unloading is not lock step)")
    else:
        uuid_out = ask("V", "data get entity W0 UUID", wait=0.3)
        nums = re.findall(r"-?\d+", uuid_out.split("data")[-1] if False else uuid_out)
        # vanilla prints: "W0 has the following entity data: [I; a, b, c, d]"
        m = re.search(r"\[I;\s*(-?\d+),\s*(-?\d+),\s*(-?\d+),\s*(-?\d+)\]", uuid_out)
        uuid = m.groups()
        owner = "[I;" + ",".join(uuid) + "]"
        print("owner", owner)
        both("tick freeze")
        time.sleep(0.5)
        for t in "KV":
            w(t, f"summon ender_pearl {block_x(0) + 0.5} {Y + 3} {CZ * 16 + 8.5} {{Owner:{owner},NoGravity:1b,Motion:[0.0d,0.0d,0.0d],Tags:[\"pearl\"]}}")
        time.sleep(0.5)
        # the pearl ticks once with the forced chunk still there (its first tick places its ticket); only then the forceload goes
        step(3)
        both(f"forceload remove {block_x(0)} {CZ * 16 + 8}")
        step(2)
        for t in "KV":
            print(t, "forceload list:", ask(t, "forceload query", wait=0.3).strip()[-100:])
        report("2 ticks after the pearl was summoned and the forceload removed")
        step(58)
        report("60 ticks: the pearl's ticket holds the chunks")
        kill_at = int(sys.argv[2]) if len(sys.argv) > 2 else 10
        step(kill_at)
        both("kill @e[tag=pearl]")
        step(1)
        series = {t: [] for t in "KV"}
        for i in range(48):
            for t in "KV":
                series[t].append(ageof(t, "rt0"))
            step(1)
        for t in "KV":
            a = series[t]
            rises = [a[i + 1] - a[i] if a[i] is not None and a[i + 1] is not None else None for i in range(len(a) - 1)]
            ticking = sum(1 for r in rises if r)
            print(t, "ticks the cow still aged after the pearl was killed:", ticking, " series", a[:3], "...", a[-3:])
        report("after the ticket ran out")
elif MODE == "portal":
    # an overworld portal beside W0 at (1000, 200..204, 1000): obsidian frame on x, portal blocks inside; the cow stands in it.
    setup()
    px, pz, py = 1010, 1010, 190
    for t in "KV":
        w(t, f"fill {px - 4} {py - 1} {pz - 4} {px + 5} {py - 1} {pz + 4} minecraft:stone")
        w(t, f"fill {px - 2} {py} {pz - 2} {px + 3} {py + 6} {pz + 2} minecraft:air")
        w(t, f"fill {px} {py} {pz} {px + 3} {py + 4} {pz} minecraft:obsidian")
        w(t, f"fill {px + 1} {py + 1} {pz} {px + 2} {py + 3} {pz} minecraft:nether_portal[axis=x]")
    time.sleep(2)
    both("tick freeze")
    time.sleep(0.5)
    for t in "KV":
        w(t, f"summon cow {px + 1.5} {py + 1} {pz + 0.5} {{Age:-24000,PersistenceRequired:1b,Tags:[\"trav\"]}}")
    # Run until the cow has gone: it leaves the overworld at once (entities have no portal wait), then arrives in the nether where nobody is.
    arrived = {"K": None, "V": None}
    ages = {"K": [], "V": []}
    for i in range(420):
        step(1)
        for t in "KV":
            a = ageof(t, "trav", "the_nether")
            ages[t].append(a)
            if a is not None and arrived[t] is None:
                arrived[t] = i
        if i % 50 == 0:
            print(i, "K", ages["K"][-1], "V", ages["V"][-1])
            sys.stdout.flush()
    for t in "KV":
        a = [x for x in ages[t] if x is not None]
        print(t, "arrived at step", arrived[t], "Age first", a[0] if a else None, "last", a[-1] if a else None, "samples", len(a),
              "ticks it aged:", (a[-1] - a[0]) if a else None)
else:
    sys.exit("unknown mode")
