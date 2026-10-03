"""Recent features seen by the real 26.3 client (wp35): start a release server, join it with the
client (KilnView) and screenshot the game window (PrintWindow only) for each scene:
  chestboat  a chest boat and its menu (a sneaking click)
  donkey     a tamed saddled donkey with a chest: its inventory screen (a sneaking click), a llama
  parrots    a second player (a kiln-bot) with a parrot on its shoulder, and parrots dancing to a disc
  knots      leash knots on a fence, mobs on the player's lead
  trader     a wandering trader and its trade screen
  riders     a zombie horse with a rider, a camel husk with a parched, nautili and zombie nautili
             with riders in a water tank, a spider jockey
  breeze     a breeze with arrows shot at it
Reports client decode errors and server warnings at the end.

usage: python tools/wp35_view.py [--port 25584] [--keep] [--scene NAME ...] [--no-build]
Set KILN_DATAPACK to the vanilla datapack (work/generated) and KILN_WORK to a work directory of
your own (the client's game directory and the screenshots go there) when running from a worktree.
"""

import argparse
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
sys.dont_write_bytecode = True
import blocks_view  # noqa: E402
import e2e  # noqa: E402
from leash_view import offline_uuid  # noqa: E402

ALL = ["chestboat", "donkey", "parrots", "knots", "trader", "riders", "breeze"]


def uuid_nbt(name):
    return "[I;" + ",".join(str(i) for i in offline_uuid(name)) + "]"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25584)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    ap.add_argument("--scene", action="append", help="only these scenes: " + ", ".join(ALL))
    ap.add_argument("--no-build", action="store_true")
    a = ap.parse_args()
    scenes = a.scene or ALL
    if not a.no_build and subprocess.run(["cargo", "build", "--release", "-p", "kiln-server", "-p", "kiln-bot"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    os.environ["KILN_OPS"] = "KilnView,Bob*"
    server = blocks_view.start_server(a.port)

    def console(cmd, wait=0.3):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(wait)

    name = "KilnView"
    pid = None
    for attempt in range(3):
        try:
            pid = e2e.launch(a.port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{name} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid):
                break
            time.sleep(1)
        if e2e.alive(pid) and f"{name} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")

    def shot(tag):
        out = e2e.WORK / f"wp35-view-{tag}.png"
        e2e.screenshot(pid, out)
        print("screenshot:", out, flush=True)

    def clear():
        console("kill @e[type=!minecraft:player]", 0.8)
        console("fill -4 -60 4 24 -40 30 air", 0.3)

    me = uuid_nbt(name)
    time.sleep(3)
    console("difficulty normal")
    console("gamerule minecraft:spawn_mobs false")
    console("gamerule minecraft:spawn_wandering_traders false")
    console("time set 6000")
    console(f"gamemode creative {name}")
    # The player stands at z 20 looking north; the scenes are 6 to 10 blocks in front.
    face = f"tp {name} 8.5 -60 20.5 180 12"

    if "chestboat" in scenes:
        clear()
        console(face)
        console(
            "summon minecraft:oak_chest_boat 8.5 -60 13.5 {Items:[{Slot:0b,id:\"minecraft:diamond\",count:12},"
            "{Slot:4b,id:\"minecraft:oak_log\",count:64},{Slot:13b,id:\"minecraft:ender_pearl\",count:16},"
            "{Slot:26b,id:\"minecraft:iron_sword\",count:1}],CustomName:\"Kiln Cargo\"}"
        )
        console("summon minecraft:birch_chest_boat 12.5 -60 13.5 {Rotation:[40f,0f]}")
        console("summon minecraft:bamboo_chest_raft 4.5 -60 13.5 {Rotation:[-40f,0f]}")
        time.sleep(3)
        shot("chestboat")
        console(f"kiln interact {name} 8 -60 13 sneak")
        time.sleep(3)
        shot("chestboat-menu")

    if "donkey" in scenes:
        clear()
        console(face)
        # A tamed donkey with a saddle and a chest, a tamed llama with a carpet and chest.
        console(
            f"summon minecraft:donkey 8.5 -60 13.5 {{Tame:1b,Owner:{me},ChestedHorse:1b,PersistenceRequired:1b,"
            "equipment:{saddle:{id:\"minecraft:saddle\",count:1}},"
            "Items:[{Slot:2b,id:\"minecraft:bread\",count:5},{Slot:3b,id:\"minecraft:diamond\",count:2},{Slot:14b,id:\"minecraft:carrot\",count:9}]}"
        )
        console(
            f"summon minecraft:llama 12.5 -60 13.5 {{Tame:1b,Owner:{me},ChestedHorse:1b,PersistenceRequired:1b,Strength:5,"
            "equipment:{body:{id:\"minecraft:red_carpet\",count:1}},Variant:1,"
            "Items:[{Slot:2b,id:\"minecraft:wheat\",count:3}]}"
        )
        console("summon minecraft:mule 4.5 -60 13.5 {Tame:1b,ChestedHorse:1b,PersistenceRequired:1b}")
        time.sleep(3)
        shot("donkey")
        console(f"kiln interact {name} 8 -60 13 sneak")
        time.sleep(3)
        shot("donkey-menu")
        console(f"kiln interact {name} 12 -60 13 sneak")
        time.sleep(3)
        shot("llama-menu")

    if "parrots" in scenes:
        clear()
        console(face)
        bot = subprocess.Popen(
            [str(e2e.ROOT / "target" / "release" / ("kiln-bot.exe" if os.name == "nt" else "kiln-bot")), "--addr", f"127.0.0.1:{a.port}",
             "--count", "1", "--name-prefix", "Bob", "--behavior", "idle", "--duration", "120", "--view-distance", "4"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        deadline = time.time() + 30
        while time.time() < deadline and "Bob0 joined" not in e2e.server_log():
            time.sleep(0.5)
        bob = uuid_nbt("Bob0")
        console("gamemode survival Bob0")
        console("tp Bob0 8.5 -60 12.5 0 0")
        # Bob's own tamed parrots: one he picks up on his shoulder, two dancing next to a jukebox.
        console(f"summon minecraft:parrot 9.5 -60 11.5 {{Tame:1b,Owner:{bob},Variant:1,PersistenceRequired:1b}}")
        time.sleep(1.5)
        console("kiln interact Bob0 9 -60 11 sneak")
        time.sleep(2)
        console("tp Bob0 8.5 -60 10.5 0 0")
        console(f"tp {name} 8.5 -60 15.5 0 5")
        time.sleep(2)
        shot("parrot-shoulder")
        console(f"tp {name} 8.5 -60 20.5 180 12")
        console("tp Bob0 30.5 -60 30.5")
        console("setblock 8 -60 12 minecraft:jukebox")
        console(f"summon minecraft:parrot 7.5 -60 13.5 {{Tame:1b,Owner:{me},Variant:0,PersistenceRequired:1b,NoAI:1b}}")
        console(f"summon minecraft:parrot 9.5 -60 13.5 {{Tame:1b,Owner:{me},Variant:3,PersistenceRequired:1b,NoAI:1b}}")
        console(f"item replace entity {name} hotbar.0 with minecraft:music_disc_13")
        time.sleep(1)
        console(f"kiln use {name} 8 -60 12")
        time.sleep(2.5)
        shot("parrot-dance")
        time.sleep(1.2)
        shot("parrot-dance-2")
        bot.terminate()

    if "knots" in scenes:
        clear()
        console(face)
        console("setblock 14 -61 14 minecraft:stone", 0.1)
        console("setblock 14 -60 14 minecraft:oak_fence", 0.1)
        console("setblock 2 -61 14 minecraft:stone", 0.1)
        console("setblock 2 -60 14 minecraft:oak_fence", 0.1)
        console("summon minecraft:sheep 12.5 -60 12.5 {PersistenceRequired:1b,leash:[I;14,-60,14]}", 0.1)
        console("summon minecraft:sheep 16.5 -60 12.5 {PersistenceRequired:1b,leash:[I;14,-60,14]}", 0.1)
        console("summon minecraft:cow 4.5 -60 12.5 {PersistenceRequired:1b,leash:[I;2,-60,14]}", 0.1)
        console(f"summon minecraft:cow 8.5 -60 12.5 {{PersistenceRequired:1b,leash:{{UUID:{me}}}}}", 0.1)
        time.sleep(3)
        shot("knots")
        # Remove one fence: its knot goes and the leads drop.
        console("setblock 2 -60 14 minecraft:air", 0.5)
        time.sleep(1.5)
        shot("knots-broken")

    if "trader" in scenes:
        clear()
        console(face)
        console("summon minecraft:wandering_trader 8.5 -60 13.5 {NoAI:1b,PersistenceRequired:1b,DespawnDelay:48000}")
        time.sleep(2)
        shot("trader")
        console(f"kiln interact {name} 8 -60 13")
        time.sleep(3)
        shot("trader-menu")

    if "riders" in scenes:
        clear()
        console(face)
        console("summon minecraft:zombie_horse 4.5 -60 13.5 {PersistenceRequired:1b,Passengers:[{id:\"minecraft:zombie\",PersistenceRequired:1b}]}")
        console("summon minecraft:camel_husk 9.5 -60 13.5 {PersistenceRequired:1b,Passengers:[{id:\"minecraft:parched\",PersistenceRequired:1b}]}")
        console("summon minecraft:spider 14.5 -60 13.5 {PersistenceRequired:1b,Passengers:[{id:\"minecraft:skeleton\",PersistenceRequired:1b}]}")
        console("summon minecraft:skeleton_horse 19.5 -60 13.5 {PersistenceRequired:1b,Tame:1b,Passengers:[{id:\"minecraft:skeleton\",PersistenceRequired:1b}]}")
        time.sleep(4)
        shot("riders")
        console("kill @e[type=!minecraft:player]", 0.5)
        # A tank: nautili and zombie nautili, one with a drowned on its back.
        console("fill -2 -61 4 16 -61 18 minecraft:stone")
        console("fill -2 -60 4 16 -54 18 minecraft:water")
        console("tp KilnView 8.5 -60 3.5 180 5")
        console("summon minecraft:nautilus 4.5 -57 10.5 {PersistenceRequired:1b}")
        console("summon minecraft:zombie_nautilus 9.5 -57 10.5 {PersistenceRequired:1b,Passengers:[{id:\"minecraft:drowned\",PersistenceRequired:1b}]}")
        console("summon minecraft:nautilus 13.5 -58 12.5 {PersistenceRequired:1b,Baby:1b}")
        time.sleep(4)
        shot("nautili")
        console("fill -2 -60 4 16 -54 18 minecraft:air", 0.5)

    if "breeze" in scenes:
        clear()
        console(face)
        console("summon minecraft:breeze 8.5 -60 12.5 {PersistenceRequired:1b,NoAI:1b}")
        console(f"summon minecraft:skeleton 8.5 -60 4.5 {{PersistenceRequired:1b,NoAI:1b,Rotation:[0f,0f],equipment:{{mainhand:{{id:\"minecraft:bow\",count:1}}}}}}")
        for i in range(6):
            # Arrows flying at the breeze from the south (the player's side).
            console("summon minecraft:arrow 8.5 -59 17.5 {Motion:[0.0d,0.0d,-1.5d],PersistenceRequired:1b}", 0.15)
        time.sleep(0.5)
        shot("breeze-arrows")
        time.sleep(1.5)
        shot("breeze-after")

    time.sleep(1)
    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]|Skipping|Received unknown", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN", l):
            print("server:", l)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
