"""wp70 state check: the bench's commands (and more) on two Kiln builds, the saved worlds compared chunk by chunk
(blocks, biomes, block and sky light, block entities, scheduled ticks, heightmaps). Ticks are frozen and random
ticks off, so both runs see the same world when each command runs.

On the VM:  python3 state70.py run <kiln binary> <name>      world saved to ~/wp70/state_<name>
            python3 state70.py cmp <name A> <name B>
"""

import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import anvil  # noqa: E402
import bench70  # noqa: E402

W = Path.home() / "wp70"
TOOLS = Path(__file__).resolve().parent
# the bench's areas, and two far apart (separate regions) for a fill from one into the other
AREAS = [(0, 0, 111, 95), (96, 96, 143, 127), (192, 192, 239, 239), (288, 288, 447, 447), (512, 0, 543, 31), (1024, 0, 1055, 31)]

EXTRA = [
    # shapes, neighbours, block entities, fluids, redstone; a fill across cell (128) and region borders
    "fill 0 150 40 20 160 60 oak_fence",
    "fill 2 150 42 18 158 58 air hollow",
    "fill 0 150 40 20 150 60 redstone_wire",
    "setblock 10 151 50 redstone_block",
    "fill 0 152 40 20 152 60 water",
    "fill 0 150 40 20 160 60 cobblestone_wall replace air",
    "fill 0 150 40 20 160 60 torch replace cobblestone_wall",
    "setblock 30 150 30 oak_sign{front_text:{messages:['\"a\"','\"b\"','\"c\"','\"d\"']}}",
    "setblock 31 150 30 chest{Items:[{Slot:0b,id:\"minecraft:diamond\",count:3}]}",
    "clone 30 150 30 31 150 30 30 152 30",
    "fill 30 150 30 31 152 30 air destroy",
    "fill 100 60 100 160 70 105 glowstone",
    "fill 100 60 100 160 70 105 air",
    "fill 120 140 110 136 156 126 stone",
    "fill 121 141 111 135 155 125 sea_lantern hollow",
    "fill 0 150 0 31 181 31 stone",
    "fill 0 160 0 31 170 31 glass replace stone",
    "fill 0 150 0 31 181 31 air strict",
    "fill 200 30 200 231 61 lava",
    "fill 200 30 200 231 61 air",
    "fill 200 30 200 231 61 sand",
    "fill 520 150 0 1030 152 3 stone",
    "fill 520 149 4 1030 149 4 glowstone",
    "fill 520 150 0 1030 152 3 air",
]


def run(binary, name):
    subprocess.run([str(TOOLS / "wp70_servers.sh"), "K", "start", binary], check=True)
    s = bench70.Server("K")
    t0 = time.time()
    while True:
        try:
            s.query(timeout=5)
            break
        except (TimeoutError, FileNotFoundError):
            if time.time() - t0 > 300:
                sys.exit("server did not start")
    for c in ["tick freeze", "gamerule random_tick_speed 0", "gamerule spawn_mobs false", "gamerule advance_time false", "gamerule advance_weather false",
              "difficulty peaceful"] + [f"forceload add {a} {b} {c} {d}" for a, b, c, d in AREAS]:
        s.send(c)
    time.sleep(30)
    cmds = []
    for _, setup, cmd, _ in bench70.CASES:
        if cmd.startswith("place") or "summon" in cmd or "@e" in cmd or "kill" in cmd or "tp_each" in cmd:
            continue
        cmds += setup + [cmd]
    cmds += EXTRA
    for i, c in enumerate(cmds):
        s.send(c)
        s.send(f"wp70state{i}")
        s.wait_for(f"wp70state{i}")
    s.send("save-all flush")
    s.wait_for("Saved the game", timeout=300)
    subprocess.run([str(TOOLS / "wp70_servers.sh"), "K", "stop"])
    dest = W / f"state_{name}"
    shutil.rmtree(dest, ignore_errors=True)
    shutil.copytree(W / "K" / "world", dest)
    print("saved", dest, len(cmds), "commands")


SKIP = {"LastUpdate", "InhabitedTime", "isLightOn", "Status", "PostProcessing", "structures", "blending_data", "CarvingMask"}


def norm(tag):
    if isinstance(tag, dict):
        return {k: norm(v) for k, v in tag.items() if k not in SKIP}
    if isinstance(tag, list):
        return [norm(v) for v in tag]
    if isinstance(tag, (bytes, bytearray)):
        return bytes(tag)
    return tag


def cmp(a, b):
    diffs = 0
    chunks = 0
    for x0, z0, x1, z1 in AREAS:
        for cx in range(x0 >> 4, (x1 >> 4) + 1):
            for cz in range(z0 >> 4, (z1 >> 4) + 1):
                ta, tb = anvil.chunk_nbt(W / f"state_{a}", cx, cz), anvil.chunk_nbt(W / f"state_{b}", cx, cz)
                chunks += 1
                if ta is None or tb is None:
                    if (ta is None) != (tb is None):
                        print(f"chunk {cx},{cz}: only in one world")
                        diffs += 1
                    continue
                na, nb = norm(ta), norm(tb)
                for key in sorted(set(na) | set(nb)):
                    if key == "sections":
                        sa = {s.get("Y"): s for s in na.get(key, [])}
                        sb = {s.get("Y"): s for s in nb.get(key, [])}
                        for y in sorted(set(sa) | set(sb), key=lambda v: (v is None, v)):
                            for k in sorted(set(sa.get(y, {})) | set(sb.get(y, {}))):
                                if sa.get(y, {}).get(k) != sb.get(y, {}).get(k):
                                    diffs += 1
                                    if diffs <= 40:
                                        print(f"chunk {cx},{cz} section {y}: {k} differs")
                    elif na.get(key) != nb.get(key):
                        diffs += 1
                        if diffs <= 40:
                            va, vb = str(na.get(key))[:300], str(nb.get(key))[:300]
                            print(f"chunk {cx},{cz}: {key} differs\n  {a}: {va}\n  {b}: {vb}")
    print(f"{chunks} chunks compared, {diffs} differences")
    return diffs


if __name__ == "__main__":
    if sys.argv[1] == "run":
        run(sys.argv[2], sys.argv[3])
    else:
        sys.exit(1 if cmp(sys.argv[2], sys.argv[3]) else 0)
