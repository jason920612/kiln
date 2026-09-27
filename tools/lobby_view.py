"""Lobby toolkit seen by the real 26.3 client: start a release server, join it with the client
and an idle kiln-bot, set up a sidebar objective, colored teams with prefixes, a boss bar and a
title from the console, then take a screenshot of the game window (work/lobby-view.png).

With --world DIR the server saves into DIR; --restart then stops it, starts it again on the
same world and rejoins, so the second screenshot (work/lobby-view-restart.png) shows the
scoreboard, teams and boss bar loaded from data/minecraft/*.dat.

usage: python tools/lobby_view.py [--port 25583] [--world DIR [--restart]] [--keep]
"""

import argparse
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402

BOT = "LobbyBot0"

# Superflat: grass at y -61. The client looks north at the bot from z 21.
SCENE = f"""
gamemode creative KilnView
time set 6000
fill 2 -60 10 14 -55 24 air
scoreboard objectives add lobby dummy {{"text":"Kiln Lobby","color":"gold","bold":true}}
scoreboard objectives setdisplay sidebar lobby
scoreboard players set KilnView lobby 12
scoreboard players set {BOT} lobby 7
scoreboard players set #coins lobby 42
scoreboard players display name #coins lobby {{"text":"Coins","color":"yellow"}}
scoreboard players display numberformat #coins lobby styled {{"color":"green"}}
team add red {{"text":"Red Team"}}
team modify red color red
team modify red prefix {{"text":"[RED] ","color":"dark_red","bold":true}}
team join red {BOT}
team add blue
team modify blue color aqua
team modify blue prefix "[B] "
team join blue KilnView
bossbar add kiln:lobby {{"text":"Welcome to the Kiln lobby","color":"aqua"}}
bossbar set kiln:lobby color purple
bossbar set kiln:lobby style notched_10
bossbar set kiln:lobby value 70
bossbar set kiln:lobby players @a
tp {BOT} 8 -60 17 0 0
tp KilnView 8 -60 21 180 5
title KilnView times 10 400 20
title KilnView subtitle {{"text":"scoreboard, teams, boss bars, titles","color":"gray"}}
title KilnView title {{"text":"Kiln M3","color":"gold","bold":true}}
"""

AFTER_RESTART = f"""
tp {BOT} 8 -60 17 0 0
tp KilnView 8 -60 21 180 5
title KilnView actionbar {{"text":"loaded from the world save","color":"green"}}
"""


def start_server(port, world):
    e2e.stop_server(port)
    env = dict(os.environ, KILN_PORT=str(port), RUST_LOG="info", KILN_OPS="KilnView")
    if world:
        env["KILN_WORLD"] = str(world)
    exe = e2e.WORK / f"kiln-{port}{e2e.EXE.suffix}"
    shutil.copy2(e2e.EXE, exe)
    log = open(e2e.SERVER_LOG, "w", encoding="utf-8")
    flags = 0x00000200 | 0x01000000 if os.name == "nt" else 0
    p = subprocess.Popen([str(exe)], cwd=e2e.ROOT, env=env, stdin=subprocess.PIPE, stdout=log,
                         stderr=subprocess.STDOUT, creationflags=flags, text=True)
    (e2e.WORK / f"server-{port}.pid").write_text(str(p.pid))
    for _ in range(300):
        if "listening on" in e2e.server_log():
            return p
        time.sleep(0.1)
    sys.exit("server did not start:\n" + e2e.server_log())


def join_client(port, name):
    for attempt in range(3):
        try:
            pid = e2e.launch(port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{name} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid):
                break
            time.sleep(1)
        if e2e.alive(pid) and f"{name} finished loading terrain" in e2e.server_log():
            return pid
        print(f"client exited before joining (attempt {attempt + 1})")
    return None


def start_bot(port):
    exe = e2e.WORK / f"kiln-bot-{port}{e2e.EXE.suffix}"
    shutil.copy2(e2e.ROOT / "target" / "release" / f"kiln-bot{e2e.EXE.suffix}", exe)
    argv = [str(exe), "--addr", f"127.0.0.1:{port}", "--count", "1", "--behavior", "idle", "--duration", "3600",
            "--name-prefix", "LobbyBot", "--report-interval", "3600"]
    bot = subprocess.Popen(argv, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
    deadline = time.time() + 60
    while time.time() < deadline and f"{BOT} joined the game" not in e2e.server_log():
        time.sleep(0.5)
    return bot


def run_scene(server, scene):
    for line in scene.strip().splitlines():
        server.stdin.write(line + "\n")
        server.stdin.flush()
        time.sleep(0.2)


def report():
    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    problems = [l for l in client_log.splitlines()
                if re.search(r"disconnect|Failed to decode|Exception|Error", l) and "Realms" not in l]
    for l in problems[-20:]:
        print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN", l):
            print("server:", l)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25583)
    ap.add_argument("--world", help="world directory to save into (created by the server if missing)")
    ap.add_argument("--restart", action="store_true", help="restart on the same world and look again")
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    world = os.path.abspath(a.world) if a.world else None
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = start_server(a.port, world)
    bot = start_bot(a.port)
    name = "KilnView"
    pid = join_client(a.port, name)
    if pid is None:
        bot.kill()
        e2e.stop_server(a.port)
        sys.exit("the client did not join")
    run_scene(server, SCENE)
    time.sleep(5)
    shot = e2e.WORK / "lobby-view.png"
    e2e.screenshot(pid, shot)
    print("screenshot:", shot)
    report()
    if world and a.restart:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        bot.kill()
        server.stdin.write("stop\n")
        server.stdin.flush()
        server.wait(timeout=60)
        server = start_server(a.port, world)
        bot = start_bot(a.port)
        pid = join_client(a.port, name)
        if pid is None:
            bot.kill()
            e2e.stop_server(a.port)
            sys.exit("the client did not rejoin")
        run_scene(server, AFTER_RESTART)
        time.sleep(5)
        shot = e2e.WORK / "lobby-view-restart.png"
        e2e.screenshot(pid, shot)
        print("screenshot:", shot)
        report()
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        bot.kill()
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
