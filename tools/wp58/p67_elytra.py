"""wp67: one real 26.3 client glides with elytra and firework rockets in a straight line over fresh terrain (like wp65's
p65_elytra.py), on the wp67 server (tools/wp58/wp67_start.sh: console ~/wp67/rc/console, tick trace ~/wp67/rc/ticks).
Prints the flight (position and speed every 10 s) and the server's MSPT over the flight from the per-tick trace.

usage: KILN_PLAY_DIR=wp67/rc KILN_AGENT_PORT=26067 python3 p67_elytra.py DIM X Z DX DZ SECONDS [SY]
(start at (X, SY, Z), SY 120 by default, fly in direction (DX, DZ); DIM overworld or the_nether)
Against the vanilla reference server (tools/wp58/wp67_vanilla.sh) set VANILLA=1 and KILN_PLAY_DIR=wp67/rc-v: there is no tick
trace, so the script asks `tick query` every 5 s and prints what vanilla answered."""
import os
import re
import math
import sys
import time
from pathlib import Path

from ctl import C, server, PLAY

LOG = PLAY / "server.log"
TRACE = PLAY / "ticks"


def sv(cmd, wait=0.4):
    server(cmd)
    time.sleep(wait)


def stats(ms):
    if not ms:
        return {}
    v = sorted(ms)
    at = lambda q: v[min(len(v), max(1, math.ceil(len(v) * q))) - 1]
    hist = [0] * 7
    for x in v:
        hist[next((i for i, e in enumerate((2, 5, 10, 15, 25, 50)) if x < e), 6)] += 1
    return {"ticks": len(v), "mean": round(sum(v) / len(v), 2), "p50": at(0.5), "p99": at(0.99), "max": v[-1], "hist(<2,<5,<10,<15,<25,<50,>=50)": hist}


def trace(t0, t1):
    out = []
    for line in TRACE.read_text().splitlines():
        w = line.split()
        if len(w) == 3 and t0 <= int(w[0]) < t1:
            out.append(int(w[2]) / 1000.0)
    return out


def fly(dimn, x0, z0, dx, dz, secs, sy):
    c = None
    for _ in range(180):
        try:
            c = c or C()
            if "x" in c.state(0):
                break
        except Exception:
            c = None
        time.sleep(1)
    e = "minecraft:" + dimn
    sv("clear KilnTest")
    sv("gamemode survival KilnTest")
    sv("item replace entity KilnTest armor.chest with minecraft:elytra")
    for _ in range(3):
        sv("give KilnTest minecraft:firework_rocket[fireworks={flight_duration:3}] 64")
    sv("effect give KilnTest minecraft:resistance 3000 4 true")
    sv("effect give KilnTest minecraft:fire_resistance 3000 0 true")
    sv("time set day")
    # A short platform to load the start, then into the air above it: the wings open while falling (pressed on the
    # ground or just after a jump, the server may still hold the player on the ground and refuse the start).
    sv(f"execute in {e} run forceload add {x0 - 8} {z0 - 8} {x0 + 8} {z0 + 8}", 3)
    sv(f"execute in {e} run fill {x0 - 3} {sy - 1} {z0 - 3} {x0 + 3} {sy - 1} {z0 + 3} minecraft:stone", 1)
    sv(f"execute in {e} run tp KilnTest {x0 + 0.5} {sy} {z0 + 0.5}", 8)
    sv(f"execute in {e} run forceload remove {x0 - 8} {z0 - 8} {x0 + 8} {z0 + 8}")
    n = math.hypot(dx, dz)
    ux, uz = dx / n, dz / n
    yaw = math.degrees(math.atan2(-ux, uz))
    c.look(yaw, 5)
    sv(f"execute in {e} run tp KilnTest {x0 + 0.5} {sy + 30} {z0 + 0.5} {yaw:.1f} 5", 1.5)
    c.select("minecraft:firework_rocket")
    st = c.state(0)
    print("selected", st.get("selected"), "rockets", [e for e in st.get("inv", []) if "rocket" in e.get("id", "")][:3], flush=True)
    # falling: open the wings, then rocket
    c.key("space", "tap"); time.sleep(1.0)
    print("gliding", c.state(0).get("fallFlying"), flush=True)
    t0 = time.time()
    start = c.state(0)
    last, lastp = t0, (start["x"], start["z"])
    rockets = 0
    next_rocket = t0
    vanilla = os.environ.get("VANILLA") == "1"
    log_from = LOG.stat().st_size
    next_query = t0 + 20
    while time.time() - t0 < secs:
        if vanilla and time.time() >= next_query:
            server("tick query")
            next_query += 5
        st = c.state(0)
        if st["health"] <= 0:
            print("died", flush=True)
            break
        if not st.get("fallFlying"):
            c.key("space", "tap"); time.sleep(0.3); c.key("space", "tap")
        pitch = -12
        if st["y"] < sy - 30:
            pitch = -35
        elif st["y"] > sy + 40:
            pitch = 15
        c.look(yaw, pitch)
        if time.time() >= next_rocket:
            held = [i for i in st.get("inv", []) if i.get("slot") == st.get("selected")]
            if not held or held[0].get("id") != "minecraft:firework_rocket":
                c.select("minecraft:firework_rocket")
            # (pressed for a moment: a slow software-rendered client may miss a tap)
            c.mouse("right", "down"); time.sleep(0.12); c.mouse("right", "up")
            rockets += 1
            next_rocket += 2.0
        time.sleep(0.25)
        if time.time() - last >= 10:
            p = (st["x"], st["z"])
            sp = math.hypot(p[0] - lastp[0], p[1] - lastp[1]) / (time.time() - last)
            print(f"t+{time.time() - t0:.0f}s at {st['x']:.0f},{st['y']:.0f},{st['z']:.0f} speed {sp:.1f} b/s", flush=True)
            last, lastp = time.time(), p
    t1 = time.time()
    end = c.state(0)
    left = sum(e["n"] for e in end.get("inv", []) if e.get("id") == "minecraft:firework_rocket")
    left -= 64  # (three stacks were given)
    dist = math.hypot(end["x"] - start["x"], end["z"] - start["z"])
    print(f"flew {dist:.0f} blocks in {t1 - t0:.0f} s ({dist / (t1 - t0):.1f} b/s), {rockets} rocket clicks, {128 - left} rockets used", flush=True)
    if vanilla:
        with open(LOG, "rb") as f:
            f.seek(log_from)
            txt = f.read().decode(errors="replace")
        avg = [float(m) for m in re.findall(r"Average time per tick: ([0-9.]+)", txt)]
        print("vanilla tick query: ", len(avg), "answers; average time per tick: mean of answers", round(sum(avg) / max(1, len(avg)), 2),
              "max", max(avg or [0]), flush=True)
        for l in txt.splitlines():
            if "Percentile" in l or "P50" in l or "Average time per tick" in l:
                print("  ", l.strip()[-160:], flush=True)
        return
    # MSPT over the flight after 20 s of warmup (the trace is flushed with each 30 s report)
    time.sleep(32)
    print("mspt (flight after 20 s):", stats(trace(int((t0 + 20) * 1000), int(t1 * 1000))), flush=True)
    lines = [l for l in LOG.read_text(errors="replace").splitlines() if "moved too quickly" in l or "moved wrongly" in l or "chunk totals" in l]
    print("server: moved-wrongly/too-quickly lines:", sum("moved" in l for l in lines), flush=True)
    print("last chunk totals:", (lines[-1].split("chunk totals:")[-1][:300] if lines else ""), flush=True)


if __name__ == "__main__":
    a = sys.argv[1:]
    fly(a[0], int(a[1]), int(a[2]), float(a[3]), float(a[4]), int(a[5]), int(a[6]) if len(a) > 6 else 120)
