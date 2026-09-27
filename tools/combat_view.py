"""Player combat seen by the real 26.3 client: start a release server, join it with the client
(KilnView, survival), let a scripted player (Striker, a raw-protocol bot with a diamond sword)
hit it: screenshots of the game window after two hits (hearts, hurt tilt) and after the
killing blow (death screen naming Striker), then report client decode errors.

usage: python tools/combat_view.py [--port 25584] [--keep]
"""

import argparse
import os
import re
import struct
import subprocess
import sys
import threading
import time
import uuid

sys.path.insert(0, os.path.dirname(__file__))
sys.dont_write_bytecode = True
import blocks_view  # noqa: E402
import e2e  # noqa: E402
from smoke_client import Buf, Conn, pid, string, varint  # noqa: E402

import json  # noqa: E402

PLAYER_TYPE = json.loads((e2e.WORK / "generated/reports/registries.json").read_text())[
    "minecraft:entity_type"]["entries"]["minecraft:player"]["protocol_id"]


class Striker:
    """A scripted player: logs in, keeps the connection alive and attacks on request."""

    def __init__(self, port, name="Striker"):
        self.c = Conn("127.0.0.1", port)
        self.lock = threading.Lock()
        self.players = {}  # entity id -> uuid of other players it sees
        self.uuid = uuid.uuid4()
        self.errors = []
        self._login(port, name)
        threading.Thread(target=self._loop, daemon=True).start()

    def send(self, name, body=b""):
        with self.lock:
            self.c.send(pid("play", "serverbound", name), body)

    def _login(self, port, name):
        c = self.c
        c.send(0, varint(777) + string("127.0.0.1") + struct.pack(">H", port) + varint(2))
        c.send(pid("login", "serverbound", "hello"), string(name) + self.uuid.bytes)
        while True:
            i, b = c.recv()
            if i == pid("login", "clientbound", "login_compression"):
                c.threshold = b.varint()
            elif i == pid("login", "clientbound", "login_finished"):
                break
        c.send(pid("login", "serverbound", "login_acknowledged"))
        cb = lambda n: pid("configuration", "clientbound", n)
        sb = lambda n: pid("configuration", "serverbound", n)
        info = string("en_us") + bytes([8]) + varint(0) + b"\x01" + bytes([0x7F]) + varint(1) + b"\x00\x01" + varint(0)
        c.send(sb("client_information"), info)
        while True:
            i, b = c.recv()
            if i == cb("select_known_packs"):
                packs = [(b.string(), b.string(), b.string()) for _ in range(b.varint())]
                c.send(sb("select_known_packs"), varint(len(packs)) + b"".join(string(x) + string(y) + string(z) for x, y, z in packs))
            elif i == cb("keep_alive"):
                c.send(sb("keep_alive"), b.d[b.i:])
            elif i == cb("finish_configuration"):
                break
        c.send(sb("finish_configuration"))

    def _loop(self):
        cb = lambda n: pid("play", "clientbound", n)
        loaded = False
        while True:
            try:
                i, b = self.c.recv()
            except Exception as e:  # noqa: BLE001
                self.errors.append(str(e))
                return
            if i == cb("keep_alive"):
                self.send("keep_alive", b.d[b.i:])
            elif i == cb("player_position"):
                tid = b.varint()
                pos = (b.f64(), b.f64(), b.f64())
                self.send("accept_teleportation", varint(tid))
                if not loaded:
                    self.send("player_loaded")
                    loaded = True
            elif i == cb("chunk_batch_finished"):
                self.send("chunk_batch_received", struct.pack(">f", 64.0))
            elif i == cb("add_entity"):
                eid = b.varint()
                u = uuid.UUID(bytes=b.take(16))
                if b.varint() == PLAYER_TYPE and u != self.uuid:
                    self.players[eid] = u

    def hit(self, entity_id, sprint=False):
        if sprint:
            self.send("player_command", varint(0) + varint(1) + varint(0))
        self.send("attack", varint(entity_id))
        self.send("punch")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25584)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = blocks_view.start_server(a.port)

    def console(cmd):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(0.3)

    name = "KilnView"
    pid_ = None
    for attempt in range(3):
        try:
            pid_ = e2e.launch(a.port, name)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{name} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid_):
                break
            time.sleep(1)
        if e2e.alive(pid_) and f"{name} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")

    bot = Striker(a.port)
    time.sleep(3)
    console("time set 6000")
    console("gamerule minecraft:natural_health_regeneration false")
    console(f"gamemode survival {name}")
    console("gamemode survival Striker")
    console("give Striker minecraft:diamond_sword")
    # KilnView looks north at Striker, who faces it from two blocks away.
    console(f"tp {name} 8.5 -60 12.5 180 10")
    console("tp Striker 8.5 -60 10.5 0 0")
    time.sleep(3)
    target = next(iter(bot.players), None)
    if target is None:
        sys.exit(f"Striker never saw {name}")
    print("Striker attacks entity", target)
    bot.hit(target)
    time.sleep(1.5)
    bot.hit(target, sprint=True)
    time.sleep(0.4)
    first = e2e.WORK / "combat-view-hit.png"
    e2e.screenshot(pid_, first)
    print("screenshot:", first)
    time.sleep(1.5)
    console(f"tp {name} 8.5 -60 12.5 180 10")
    time.sleep(1.0)
    bot.hit(target)
    time.sleep(3)
    dead = e2e.WORK / "combat-view-death.png"
    e2e.screenshot(pid_, dead)
    print("screenshot:", dead)

    client_log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for l in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception|Error|\[CHAT\]", l) and "Realms" not in l:
            print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"panicked|ERROR|WARN|died", l):
            print("server:", l)
    print("bot errors:", bot.errors)
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid_), "/F"], capture_output=True)
        e2e.stop_server(a.port)


if __name__ == "__main__":
    main()
