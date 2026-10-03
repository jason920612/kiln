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

ALL = ["chestboat", "donkey", "parrots", "knots", "trader", "riders", "horses", "breeze"]


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
    # No chat overlay over the scenes (the server's replies still reach the client's log).
    options = e2e.WORK / "client" / "options.txt"
    if options.exists():
        text = options.read_text(encoding="utf-8")
        text = re.sub(r"^chatVisibility:.*$", "chatVisibility:2", text, flags=re.M)
        text = re.sub(r"^gamma:.*$", "gamma:1.0", text, flags=re.M)
        options.write_text(text, encoding="utf-8")
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
        # A window caught between frames comes out black (a few hundred bytes of PNG): again.
        for _ in range(4):
            e2e.screenshot(pid, out)
            if out.stat().st_size > 20000:
                break
            time.sleep(1.5)
        print("screenshot:", out, flush=True)

    def clear():
        console("kill @e[type=!minecraft:player]", 0.8)
        console("kill @e[type=minecraft:item]", 0.5)
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
        while time.time() < deadline and "Bob0 (" not in e2e.server_log():
            time.sleep(0.5)
        time.sleep(2)
        bob = uuid_nbt("Bob0")
        console("gamemode survival Bob0")
        console("tp Bob0 8.5 -60 13.5 0 0")
        # Bob's own tamed parrots (one of each side): they hop on his shoulders once he stands still.
        console(f"summon minecraft:parrot 9.5 -60 13.5 {{Tame:1b,Owner:{bob},Variant:1,PersistenceRequired:1b}}")
        console(f"summon minecraft:parrot 7.5 -60 13.5 {{Tame:1b,Owner:{bob},Variant:3,PersistenceRequired:1b}}")
        console(f"tp {name} 8.5 -60 17.5 180 10")
        time.sleep(6)
        shot("parrot-shoulder")
        time.sleep(2)
        shot("parrot-shoulder-2")
        console("tp Bob0 30.5 -60 30.5")
        console("kill @e[type=minecraft:parrot]", 0.5)
        console(f"tp {name} 8.5 -60 17.5 180 10")
        console("setblock 8 -60 13 minecraft:jukebox")
        console(f"summon minecraft:parrot 7.5 -60 13.5 {{Tame:1b,Owner:{me},Variant:0,PersistenceRequired:1b,NoAI:1b}}")
        console(f"summon minecraft:parrot 9.5 -60 13.5 {{Tame:1b,Owner:{me},Variant:2,PersistenceRequired:1b,NoAI:1b}}")
        console(f"item replace entity {name} hotbar.0 with minecraft:music_disc_13")
        time.sleep(1)
        shot("parrot-before-disc")
        console(f"kiln use {name} 8 -60 13")
        time.sleep(1.5)
        shot("parrot-dance")
        time.sleep(0.6)
        shot("parrot-dance-2")
        bot.terminate()

    if "knots" in scenes:
        clear()
        console(f"tp {name} 9.5 -60 19.5 180 8")
        console("setblock 9 -61 14 minecraft:stone", 0.1)
        console("setblock 9 -60 14 minecraft:oak_fence", 0.1)
        console("setblock 4 -61 14 minecraft:stone", 0.1)
        console("setblock 4 -60 14 minecraft:oak_fence", 0.1)
        console("summon minecraft:sheep 7.5 -60 13.5 {PersistenceRequired:1b,leash:[I;9,-60,14]}", 0.1)
        console("summon minecraft:sheep 11.5 -60 13.5 {PersistenceRequired:1b,leash:[I;9,-60,14]}", 0.1)
        console("summon minecraft:cow 2.5 -60 13.5 {PersistenceRequired:1b,leash:[I;4,-60,14]}", 0.1)
        time.sleep(3)
        shot("knots")
        # Remove one fence: its knot goes and the lead drops.
        console("setblock 4 -60 14 minecraft:air", 0.5)
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
        # Dusk: the undead do not burn.
        console("time set 13200")
        console(f"tp {name} 8.5 -60 25.5 180 8")

        def mounted(rider, vehicle, x, extra=""):
            """A vehicle at x with a rider on it (`ride`, as Passengers of a summon are not read)."""
            console(f"summon minecraft:{vehicle} {x} -60 17.5 {{PersistenceRequired:1b{extra}}}", 0.3)
            console(f"summon minecraft:{rider} {x} -60 20.5 {{PersistenceRequired:1b}}", 0.3)
            console(f"ride @e[type=minecraft:{rider},limit=1,sort=nearest] mount @e[type=minecraft:{vehicle},limit=1,sort=nearest]", 0.3)

        mounted("zombie", "zombie_horse", 2.5)
        mounted("parched", "camel_husk", 6.5)
        console("summon minecraft:camel 10.5 -60 17.5 {PersistenceRequired:1b}")
        mounted("skeleton", "spider", 14.5)
        time.sleep(4)
        shot("riders")
        console("kill @e[type=!minecraft:player]", 0.5)
        mounted("skeleton", "skeleton_horse", 2.5, ",Tame:1b")
        console("summon minecraft:zombie_horse 6.5 -60 17.5 {PersistenceRequired:1b,Tame:1b}")
        mounted("husk", "camel_husk", 10.5)
        mounted("zombie", "zombie_horse", 14.5, ",Tame:1b")
        time.sleep(3)
        shot("riders-2")
        console("kill @e[type=!minecraft:player]", 0.5)
        console("time set 6000")
        # A glass tank (the player inside it): nautili and zombie nautili, one with a drowned on its back.
        console("fill 0 -61 8 16 -61 22 minecraft:stone")
        console("fill 0 -60 8 16 -55 22 minecraft:glass hollow")
        console("fill 1 -60 9 15 -56 21 minecraft:water")
        console(f"gamemode spectator {name}")
        console(f"tp {name} 8.5 -58 10.5 0 5")
        console("summon minecraft:nautilus 5.5 -58 15.5 {PersistenceRequired:1b}")
        console("summon minecraft:nautilus 11.5 -58 17.5 {PersistenceRequired:1b,Baby:1b}")
        console("summon minecraft:zombie_nautilus 8.5 -58 14.5 {PersistenceRequired:1b}")
        console("summon minecraft:drowned 8.5 -58 15.5 {PersistenceRequired:1b}")
        console("ride @e[type=minecraft:drowned,limit=1] mount @e[type=minecraft:zombie_nautilus,limit=1]")
        time.sleep(3)
        shot("nautili")
        console(f"tp {name} 8.5 -58 10.5 40 5")
        time.sleep(1.5)
        shot("nautili-2")
        # Next to the zombie nautilus and its rider, wherever they swam to.
        console(f"execute at @e[type=minecraft:zombie_nautilus,limit=1] run tp {name} ~ ~-2 ~-3 0 -20", 1.0)
        time.sleep(1.0)
        shot("nautili-zombie")
        console(f"gamemode creative {name}")
        console("kill @e[type=!minecraft:player]", 0.5)
        console("fill 0 -61 8 16 -55 22 minecraft:air", 0.5)
        console("fill 0 -61 8 16 -61 22 minecraft:grass_block", 0.5)

    if "horses" in scenes:
        clear()
        console(f"tp {name} 8.5 -60 22.5 180 8")
        for x, kind, extra in ((2.5, "skeleton_horse", ""), (6.5, "skeleton_horse", ",Tame:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}"),
                               (10.5, "zombie_horse", ",Tame:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}"), (14.5, "horse", ",Tame:1b,Variant:1029")):
            console(f"summon minecraft:{kind} {x} -60 17.5 {{PersistenceRequired:1b{extra}}}", 0.3)
        time.sleep(3)
        shot("horses")

    if "breeze" in scenes:
        clear()
        console(f"tp {name} 8.5 -60 18.5 180 6")
        console("summon minecraft:breeze 8.5 -60 12.5 {PersistenceRequired:1b,NoAI:1b}")
        time.sleep(1.5)
        shot("breeze")
        # Frozen ticks stepped by hand: an arrow flies at the breeze, which turns it back.
        console("tick freeze")
        console("summon minecraft:arrow 8.5 -59.2 16.5 {Motion:[0.0d,0.0d,-0.7d]}")
        console("summon minecraft:arrow 7.9 -59.0 16.5 {Motion:[0.0d,0.0d,-0.7d]}")
        console("summon minecraft:arrow 9.1 -59.4 16.5 {Motion:[0.0d,0.0d,-0.7d]}")
        for n, tag in ((2, "breeze-arrows-1"), (3, "breeze-arrows-2"), (3, "breeze-arrows-3"), (4, "breeze-arrows-4")):
            console(f"tick step {n}", 1.2)
            shot(tag)
        console("tick unfreeze")


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
