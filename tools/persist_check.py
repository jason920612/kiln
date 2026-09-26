"""Vanilla acceptance test for Kiln persistence (block entities, player data, level data).

1. Fixture: a copy of the reference world where the vanilla 26.3 server places block entities
   (signs, banner, skull, spawner, chest, ...) and saves a player with items, equipment, XP,
   a spawn point, creative mode, a position and a selected slot. The chunk with the block
   entities is captured from vanilla's own Chunk Data packet.
2. Kiln on that world: the same offline player joins; their saved position, rotation, game
   mode, held slot and inventory must be what vanilla saved, and Kiln's chunk packet must carry
   the same block entities and update tags as vanilla's (and decode with the vanilla codec).
   The player then changes items, places a chest and a sign, breaks the loaded chest, moves,
   sets the time and switches to survival; Kiln saves on leave and on `stop`.
3. Vanilla on the world Kiln saved: blocks and block entities, level time, and the player's
   position, rotation, mode, held slot, items (components intact), equipment, XP and spawn
   point must be as Kiln left them.

usage: python tools/persist_check.py [--out DIR] [--kiln-port 25585] [--vanilla-port 25591]
                                     [--fresh] [--skip-build]
"""

import argparse
import gzip
import hashlib
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
os.environ["KILN_WORK"] = str(WORK)
os.environ.setdefault("KILN_DUMP_DIR", str(WORK / "persist-check" / "dumps"))
sys.path.insert(0, str(ROOT / "tools"))
sys.dont_write_bytecode = True
from smoke_client import PACKETS, Buf, Conn, creative_slot, item_id, position, string, varint  # noqa: E402

NAME = "PersistBot"
CHUNK = (6, -3)  # every fixture block entity is in this chunk (x 96..111, z -48..-33)
PLATFORM_Y = 199
Y = 200
VANILLA_POS = (104.5, 200.0, -35.5, 45.0, 10.0)
KILN_TP = (100.5, 200.0, -36.5, 90.0, 15.0)
KILN_WALK = 1.0  # blocks walked along +x after the teleport
LOADED_CHEST = (100, Y, -44)
PLACED_CHEST = (98, Y, -37)
PLACED_SIGN = (97, Y, -37)

# Block entities placed by vanilla (all in CHUNK), as `setblock` arguments.
FIXTURE_BLOCKS = [
    ((97, Y, -46), 'oak_sign[rotation=4]{front_text:{messages:["Hello","Kiln","",""],color:"blue"},is_waxed:1b}'),
    ((99, Y, -46), 'white_banner{patterns:[{pattern:"minecraft:stripe_top",color:"red"}],components:{"minecraft:lore":["Lore line"]}}'),
    ((101, Y, -46), 'skeleton_skull{custom_name:"Skully",note_block_sound:"minecraft:block.note_block.bell"}'),
    ((103, Y, -46), 'spawner{SpawnData:{entity:{id:"minecraft:pig"}},RequiredPlayerRange:1s,Delay:20s,SpawnPotentials:[{weight:1,data:{entity:{id:"minecraft:cow"}}}]}'),
    ((105, Y, -46), 'campfire[lit=false]{Items:[{Slot:0b,id:"minecraft:beef",count:1}]}'),
    ((107, Y, -46), 'decorated_pot{sherds:{back:"minecraft:brick",front:"minecraft:angler_pottery_sherd"}}'),
    ((109, Y, -46), 'beacon{primary_effect:"minecraft:speed"}'),
    ((97, Y + 1, -44), 'stone'),  # holds up the hanging sign
    ((97, Y, -44), 'oak_hanging_sign[attached=true]{back_text:{messages:["","Back","",""]}}'),
    (LOADED_CHEST, 'chest{Items:[{Slot:0b,id:"minecraft:diamond",count:5},{Slot:1b,id:"minecraft:golden_apple",count:2}]}'),
    ((103, Y, -44), 'suspicious_sand{item:{id:"minecraft:emerald",count:1}}'),
    ((105, Y, -44), 'oak_shelf{Items:[{Slot:1b,id:"minecraft:apple",count:3}],align_items_to_bottom:1b}'),
    ((107, Y, -44), 'structure_block{mode:"SAVE",name:"minecraft:kiln_test",posY:2,sizeX:3,sizeY:3,sizeZ:3,author:"kiln"}'),
    ((109, Y, -44), 'furnace{Items:[{Slot:0b,id:"minecraft:raw_iron",count:4}],cooking_time_spent:0s}'),
    ((97, Y, -42), 'lectern[has_book=true]{Book:{id:"minecraft:writable_book",count:1,components:{"minecraft:writable_book_content":{pages:["page one"]}}},Page:0}'),
    ((99, Y, -42), 'jigsaw{name:"minecraft:start",target:"minecraft:empty",pool:"minecraft:empty",final_state:"minecraft:stone",joint:"rollable"}'),
    ((101, Y, -42), 'conduit[waterlogged=false]'),
    ((103, Y, -42), 'barrel{CustomName:"Barrel"}'),
]

VANILLA_SETUP = [
    f"give {NAME} minecraft:diamond_sword[minecraft:enchantments={{\"minecraft:sharpness\":3}},minecraft:custom_name=\"Excalibur\"] 1",
    f"give {NAME} minecraft:stone 32",
    f"give {NAME} minecraft:potion[minecraft:potion_contents={{potion:\"minecraft:swiftness\"}}] 1",
    f"item replace entity {NAME} armor.head with minecraft:diamond_helmet",
    f"item replace entity {NAME} weapon.offhand with minecraft:shield",
    f"xp add {NAME} 5 levels",
    f"spawnpoint {NAME} 90 70 -20",
    f"gamemode creative {NAME}",
    f"tp {NAME} {VANILLA_POS[0]} {VANILLA_POS[1]} {VANILLA_POS[2]} {VANILLA_POS[3]} {VANILLA_POS[4]}",
]


def offline_uuid(name):
    h = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    h[6] = (h[6] & 0x0F) | 0x30
    h[8] = (h[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(h))


# ---- NBT --------------------------------------------------------------------------------

class Nbt:
    """Reads NBT into (type, value) pairs; compounds become dicts, so order does not matter."""

    def __init__(self, data):
        self.b = Buf(data)

    def string(self):
        return self.b.take(struct.unpack(">H", self.b.take(2))[0]).decode("utf-8", "replace")

    def payload(self, t):
        b = self.b
        if t == 1:
            return ("byte", struct.unpack(">b", b.take(1))[0])
        if t == 2:
            return ("short", b.i16())
        if t == 3:
            return ("int", b.i32())
        if t == 4:
            return ("long", b.i64())
        if t == 5:
            return ("float", b.f32())
        if t == 6:
            return ("double", b.f64())
        if t in (7, 11, 12):
            n = b.i32()
            fmt, size = {7: ("b", 1), 11: ("i", 4), 12: ("q", 8)}[t]
            return ({7: "bytes", 11: "ints", 12: "longs"}[t], tuple(struct.unpack(f">{n}{fmt}", b.take(n * size))))
        if t == 8:
            return ("string", self.string())
        if t == 9:
            et = b.u8()
            return ("list", tuple(self.payload(et) for _ in range(b.i32())))
        if t == 10:
            out = {}
            while (tt := b.u8()) != 0:
                name = self.string()
                out[name] = self.payload(tt)
            return ("compound", out)
        raise ValueError(f"NBT tag type {t}")

    def network(self):
        t = self.b.u8()
        return None if t == 0 else self.payload(t)

    def named(self):
        t = self.b.u8()
        self.string()
        return self.payload(t)


def nbt_file(path):
    data = Path(path).read_bytes()
    if data[:2] == b"\x1f\x8b":
        data = gzip.decompress(data)
    return Nbt(data).named()


def get(tag, *path):
    for k in path:
        if tag is None or tag[0] != "compound":
            return None
        tag = tag[1].get(k)
    return tag


def val(tag):
    return None if tag is None else tag[1]


def chunk_block_entities(body):
    """Block entities of a Chunk Data body (after the packet id): {(x, y, z): (type, tag)}."""
    b = Buf(body)
    cx, cz = b.i32(), b.i32()
    for _ in range(b.varint()):  # heightmaps
        b.varint()
        b.take(8 * b.varint())
    b.take(b.varint())  # section data
    out = {}
    for _ in range(b.varint()):
        xz = b.u8()
        y = b.i16()
        kind = b.varint()
        nbt = Nbt(b.d[b.i:])
        tag = nbt.network()
        b.i += nbt.b.i
        out[(cx * 16 + (xz >> 4), y, cz * 16 + (xz & 15))] = (kind, tag)
    return out


# ---- client -----------------------------------------------------------------------------

def pid(state, direction, name):
    return PACKETS[state][direction]["minecraft:" + name]["protocol_id"]


class Client:
    """A scripted offline-mode client that records what the server tells it."""

    def __init__(self, host, port, name, view=6):
        self.c = Conn(host, port)
        self.c.s.settimeout(30)
        self.name = name
        self.game_mode = None
        self.pos = None
        self.teleports = []
        self.held_slot = None
        self.inventory = None
        self.chunks = {}
        self.chunk_counter = 0
        self.set_time = None
        self.block_entity_data = []
        self.chats = []
        self.closed = False
        self._login(host, port, view)

    def send(self, state, name, body=b""):
        self.c.send(pid(state, "serverbound", name), body)

    def _login(self, host, port, view):
        c = self.c
        c.send(0, varint(777) + string(host) + struct.pack(">H", port) + varint(2))
        self.send("login", "hello", string(self.name) + offline_uuid(self.name).bytes)
        while True:
            i, b = c.recv()
            if i == pid("login", "clientbound", "login_compression"):
                c.threshold = b.varint()
            elif i == pid("login", "clientbound", "login_finished"):
                self.uuid = uuid.UUID(bytes=b.take(16))
                break
            elif i == pid("login", "clientbound", "login_disconnect"):
                raise RuntimeError("login refused: " + b.string())
        self.send("login", "login_acknowledged")
        info = string("en_us") + bytes([view]) + varint(0) + b"\x01" + bytes([0x7F]) + varint(1) + b"\x00\x01" + varint(0)
        self.send("configuration", "client_information", info)
        cb = lambda n: pid("configuration", "clientbound", n)  # noqa: E731
        while True:
            i, b = c.recv()
            if i == cb("select_known_packs"):
                packs = [(b.string(), b.string(), b.string()) for _ in range(b.varint())]
                body = varint(len(packs)) + b"".join(string(a) + string(x) + string(y) for a, x, y in packs)
                self.send("configuration", "select_known_packs", body)
            elif i == cb("keep_alive"):
                self.send("configuration", "keep_alive", b.take(8))
            elif i == cb("ping"):
                self.send("configuration", "pong", b.take(4))
            elif i == cb("disconnect"):
                raise RuntimeError("disconnected during configuration")
            elif i == cb("finish_configuration"):
                self.send("configuration", "finish_configuration")
                break

    def pump(self, seconds, until=None):
        """Handles packets for up to `seconds` (or until `until()` holds)."""
        cb = lambda n: pid("play", "clientbound", n)  # noqa: E731
        end = time.time() + seconds
        while time.time() < end and not (until and until()):
            self.c.s.settimeout(max(0.05, min(0.5, end - time.time())))
            try:
                i, b = self.c.recv()
            except socket.timeout:
                continue
            except (ConnectionError, OSError):
                self.closed = True
                return
            if i == cb("login"):
                b.i32()
                b.bool()
                for _ in range(b.varint()):
                    b.string()
                b.varint(), b.varint(), b.varint()
                b.bool(), b.bool(), b.bool()
                b.varint()
                b.string()
                b.i64()
                self.game_mode = b.u8()
            elif i == cb("player_position"):
                tid = b.varint()
                x, y, z = b.f64(), b.f64(), b.f64()
                b.f64(), b.f64(), b.f64()
                yaw, pitch = b.f32(), b.f32()
                self.pos = (x, y, z, yaw, pitch)
                self.teleports.append(self.pos)
                self.send("play", "accept_teleportation", varint(tid) + struct.pack(">dddff", x, y, z, yaw, pitch))
            elif i == cb("game_event"):
                if b.u8() == 3:
                    self.game_mode = int(b.f32())
            elif i == cb("set_held_slot"):
                self.held_slot = b.varint()
            elif i == cb("container_set_content"):
                if b.varint() == 0:
                    b.varint()
                    items = []
                    for _ in range(b.varint()):
                        count = b.varint()
                        if count <= 0:
                            items.append(None)
                            continue
                        item = b.varint()
                        added, removed = b.varint(), b.varint()
                        items.append((item, count))
                        if added or removed:
                            break  # component data is type-specific; stop here
                    self.inventory = items
            elif i == cb("set_time"):
                game_time = b.i64()
                clocks = []
                for _ in range(b.varint()):
                    clock = b.varint()
                    t, shift = 0, 0
                    while True:
                        byte = b.u8()
                        t |= (byte & 0x7F) << shift
                        shift += 7
                        if not byte & 0x80:
                            break
                    clocks.append((clock, t))
                    b.f32(), b.f32()
                if self.set_time is None:
                    self.set_time = (game_time, clocks)
            elif i == cb("level_chunk_with_light"):
                body = b.d[b.i:]
                self.chunks[struct.unpack(">ii", body[:8])] = body
                self.chunk_counter += 1
            elif i == cb("chunk_batch_finished"):
                self.send("play", "chunk_batch_received", struct.pack(">f", 64.0))
            elif i == cb("block_entity_data"):
                self.block_entity_data.append(b.d[b.i:])
            elif i == cb("keep_alive"):
                self.send("play", "keep_alive", b.take(8))
            elif i == cb("ping"):
                self.send("play", "pong", b.take(4))
            elif i in (cb("system_chat"), cb("disguised_chat")):
                self.chats.append(b.d[b.i:])
            elif i == cb("disconnect"):
                self.closed = True
                return

    def loaded(self):
        self.send("play", "player_loaded")

    def command(self, cmd):
        self.send("play", "chat_command", string(cmd))

    def move(self, x, y, z, yaw, pitch):
        self.send("play", "move_player_pos_rot", struct.pack(">dddffB", x, y, z, yaw, pitch, 1))

    def creative(self, slot, name, count=1):
        if name is None:
            self.send("play", "set_creative_mode_slot", struct.pack(">h", slot) + varint(0))
        else:
            self.send("play", "set_creative_mode_slot", creative_slot(slot, item_id(name), count))

    def carry(self, slot):
        self.send("play", "set_carried_item", struct.pack(">h", slot))

    def use_on(self, pos, face, seq):
        body = varint(0) + position(*pos) + varint(face) + struct.pack(">fff", 0.5, 1.0, 0.5) + b"\x00\x00" + varint(seq)
        self.send("play", "use_item_on", body)

    def dig(self, pos, seq):
        self.send("play", "player_action", varint(0) + position(*pos) + bytes([1]) + varint(seq))

    def close(self):
        try:
            self.c.s.close()
        except OSError:
            pass


# ---- servers ----------------------------------------------------------------------------

def port_free(port):
    with socket.socket() as s:
        return s.connect_ex(("127.0.0.1", port)) != 0


class Server:
    def __init__(self, cmd, cwd, env=None, log=None):
        self.lines = []
        self.log = open(log, "w", encoding="utf-8") if log else None
        flags = 0x00000200 if os.name == "nt" else 0  # own process group: no stray Ctrl+C
        self.p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                                  bufsize=1, creationflags=flags)
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            line = re.sub(r"\x1b\[[0-9;]*m", "", line.rstrip())
            self.lines.append(line)
            if self.log:
                self.log.write(line + "\n")
                self.log.flush()

    def wait_for(self, pattern, timeout, start=0):
        end = time.time() + timeout
        while time.time() < end:
            for line in self.lines[start:]:
                if re.search(pattern, line):
                    return line
            if self.p.poll() is not None:
                return None
            time.sleep(0.1)
        return None

    def cmd(self, c):
        self.p.stdin.write(c + "\n")
        self.p.stdin.flush()

    def query(self, c, pattern, timeout=15):
        n = len(self.lines)
        self.cmd(c)
        return self.wait_for(pattern, timeout, n)

    def stop(self, timeout=120):
        if self.p.poll() is None:
            try:
                self.cmd("stop")
                self.p.wait(timeout=timeout)
            except (OSError, subprocess.TimeoutExpired):
                self.p.kill()
                self.p.wait()


def vanilla(world_parent, port, log):
    props = world_parent / "server.properties"
    text = props.read_text(encoding="utf-8")
    for key, value in [("server-port", str(port)), ("online-mode", "false"), ("white-list", "false"),
                       ("enforce-secure-profile", "false"), ("spawn-protection", "0"), ("view-distance", "6"),
                       ("simulation-distance", "4"), ("max-tick-time", "-1"), ("enable-rcon", "false")]:
        text, n = re.subn(rf"(?m)^{key}=.*$", f"{key}={value}", text)
        if not n:
            text += f"{key}={value}\n"
    props.write_text(text, encoding="utf-8")
    for f in (world_parent / "world" / "session.lock",):
        f.unlink(missing_ok=True)
    s = Server(["java", "-Xmx2G", "-jar", str(WORK / "server.jar"), "--nogui"], world_parent, log=log)
    if not s.wait_for(r"Done \(", 300):
        s.stop()
        sys.exit("vanilla server did not start:\n" + "\n".join(s.lines[-30:]))
    return s


# ---- checks -----------------------------------------------------------------------------

RESULTS = []


def check(name, ok, detail=""):
    RESULTS.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  | {detail}" if detail else ""), flush=True)
    return ok


def close(a, b, eps=1e-6):
    return a is not None and b is not None and all(abs(x - y) <= eps for x, y in zip(a, b))


def block_data(server, pos):
    x, y, z = pos
    return server.query(f"data get block {x} {y} {z}", r"has the following block data|is not a block entity|not loaded")


def entity_data(server, path):
    return server.query(f"data get entity {NAME} {path}", r"has the following entity data|Found no elements|No entity")


def decode_with_vanilla(codec, body, out):
    out.write_bytes(body)
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "vanilla_decode.py"), codec, str(out)],
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.returncode == 0, (r.stdout.strip().splitlines() or [r.stderr.strip()])[-1]


def build_fixture(out, port):
    fixture = out / "fixture"
    if fixture.exists():
        shutil.rmtree(fixture)
    shutil.copytree(WORK / "vanilla-world", fixture, ignore=shutil.ignore_patterns("session.lock", "logs"))
    s = vanilla(fixture, port, out / "vanilla-fixture.log")
    try:
        cx, cz = CHUNK
        s.query(f"forceload add {cx * 16} {cz * 16}", r"[Mm]arked chunk|already")
        time.sleep(2)
        s.query(f"fill {cx * 16} {PLATFORM_Y} {cz * 16} {cx * 16 + 15} {PLATFORM_Y} {cz * 16 + 15} minecraft:stone",
                r"Successfully filled|No blocks were filled|not loaded")
        for (x, y, z), block in FIXTURE_BLOCKS:
            line = s.query(f"setblock {x} {y} {z} minecraft:{block}",
                           r"Changed the block|Could not set|Unknown|Incorrect|Expected|Invalid|not loaded")
            if not line or "Changed the block" not in line:
                sys.exit(f"vanilla refused setblock {x} {y} {z} {block}: {line}")
        bad = [l for l in s.lines if "Serialization errors" in l or "Failed to decode" in l]
        if bad:
            sys.exit("vanilla could not read fixture block entity data:\n" + "\n".join(bad))
        c = Client("127.0.0.1", port, NAME)
        c.pump(5, lambda: c.pos is not None)
        c.loaded()
        for cmd in VANILLA_SETUP:
            s.cmd(cmd)
        time.sleep(1)
        c.carry(2)
        before = c.chunk_counter
        c.pump(15, lambda: CHUNK in c.chunks and c.chunk_counter > before and close(c.pos[:3], VANILLA_POS[:3]))
        c.pump(3)
        if CHUNK not in c.chunks:
            sys.exit("vanilla never sent the fixture chunk")
        (out / "vanilla_chunk.bin").write_bytes(c.chunks[CHUNK])
        c.close()
        time.sleep(2)
        s.query("save-all flush", r"Saved the game", 120)
    finally:
        s.stop()
    player = fixture / "world" / "players" / "data" / f"{offline_uuid(NAME)}.dat"
    if not player.exists():
        sys.exit(f"vanilla saved no player file at {player}")
    return fixture


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "persist-check"))
    ap.add_argument("--kiln-port", type=int, default=25585)
    ap.add_argument("--vanilla-port", type=int, default=25591)
    ap.add_argument("--fresh", action="store_true", help="rebuild the vanilla fixture")
    ap.add_argument("--skip-build", action="store_true")
    a = ap.parse_args()
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    for port in (a.kiln_port, a.vanilla_port):
        if not port_free(port):
            sys.exit(f"port {port} is in use")

    if not a.skip_build and subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "kiln-server"], cwd=ROOT).returncode:
        sys.exit("build failed")
    fixture = out / "fixture"
    if a.fresh or not (out / "vanilla_chunk.bin").exists() or not fixture.exists():
        print("== building the vanilla fixture", flush=True)
        fixture = build_fixture(out, a.vanilla_port)
    vanilla_player = nbt_file(fixture / "world" / "players" / "data" / f"{offline_uuid(NAME)}.dat")
    vanilla_level = nbt_file(fixture / "world" / "level.dat")

    # ---- Kiln on the vanilla world ----
    print("== Kiln on the vanilla-saved world", flush=True)
    world = out / "kiln-world"
    if world.exists():
        shutil.rmtree(world)
    shutil.copytree(fixture, world)
    exe = out / ("kiln-persist" + (".exe" if os.name == "nt" else ""))
    shutil.copy2(ROOT / "target" / "release" / ("kiln.exe" if os.name == "nt" else "kiln"), exe)
    env = dict(os.environ, KILN_PORT=str(a.kiln_port), KILN_WORLD=str(world / "world"), KILN_OPS=NAME, RUST_LOG="info")
    k = Server([str(exe)], ROOT, env=env, log=out / "kiln.log")
    if not k.wait_for("listening on", 30):
        k.stop()
        sys.exit("Kiln did not start:\n" + "\n".join(k.lines[-20:]))
    try:
        c = Client("127.0.0.1", a.kiln_port, NAME)
        c.pump(10, lambda: c.pos is not None and c.inventory is not None and CHUNK in c.chunks)
        c.loaded()
        want = tuple(val(get(vanilla_player, "Pos")))
        want = tuple(v[1] for v in want)
        rot = tuple(v[1] for v in val(get(vanilla_player, "Rotation")))
        check("Kiln places the player at the vanilla-saved position", close(c.pos[:3], want), f"{c.pos} vs {want}")
        check("Kiln restores the vanilla-saved rotation", close(c.pos[3:], rot, 1e-4), f"{c.pos[3:]} vs {rot}")
        check("Kiln restores the vanilla-saved game mode", c.game_mode == val(get(vanilla_player, "playerGameType")),
              f"{c.game_mode}")
        check("Kiln restores the selected slot", c.held_slot == val(get(vanilla_player, "SelectedItemSlot")), f"{c.held_slot}")
        inv = c.inventory or []
        expect = {36: ("diamond_sword", 1), 37: ("stone", 32), 38: ("potion", 1), 5: ("diamond_helmet", 1), 45: ("shield", 1)}
        got = {s: inv[s] for s in expect if s < len(inv)}
        check("Kiln sends the vanilla-saved inventory", all(got.get(s) == (item_id(n), cnt) for s, (n, cnt) in expect.items()),
              f"{got}")
        vt = val(get(vanilla_level, "Data", "Time"))
        check("Kiln continues the saved game time", c.set_time and vt <= c.set_time[0] <= vt + 200, f"{c.set_time} vs Time {vt}")

        # Block entities in the chunk packet, against vanilla's own packet.
        vanilla_bes = chunk_block_entities((out / "vanilla_chunk.bin").read_bytes())
        kiln_bes = chunk_block_entities(c.chunks[CHUNK])
        missing = sorted(set(vanilla_bes) - set(kiln_bes))
        extra = sorted(set(kiln_bes) - set(vanilla_bes))
        differ = [p for p in vanilla_bes if p in kiln_bes and vanilla_bes[p] != kiln_bes[p]]
        check(f"Kiln's chunk carries vanilla's {len(vanilla_bes)} block entities", not missing and not extra,
              f"missing {missing} extra {extra}")
        check("block entity types and update tags equal vanilla's", not differ,
              "; ".join(f"{p}: vanilla {vanilla_bes[p]} kiln {kiln_bes[p]}" for p in differ[:3]))
        kiln_chunk = c.chunks[CHUNK]

        # Changes on Kiln.
        seq = 1
        c.creative(40, "oak_log", 13)
        c.creative(37, "stone", 20)
        c.creative(41, "chest")
        c.carry(5)
        c.use_on((PLACED_CHEST[0], PLATFORM_Y, PLACED_CHEST[2]), 1, seq)
        seq += 1
        c.creative(42, "oak_sign")
        c.carry(6)
        c.use_on((PLACED_SIGN[0], PLATFORM_Y, PLACED_SIGN[2]), 1, seq)
        seq += 1
        c.dig(LOADED_CHEST, seq)
        c.pump(1)
        c.creative(41, None)
        c.creative(42, None)
        c.carry(4)
        tps = len(c.teleports)
        c.command(f"tp @s {KILN_TP[0]} {KILN_TP[1]} {KILN_TP[2]} {KILN_TP[3]} {KILN_TP[4]}")
        c.pump(5, lambda: len(c.teleports) > tps)
        x, y, z, yaw, pitch = KILN_TP
        for i in range(1, 6):
            c.move(x + KILN_WALK * i / 5, y, z, yaw, pitch)
            c.pump(0.1)
        c.command("time set 1234")
        c.command("gamemode survival")
        c.pump(2)
        c.close()
        time.sleep(1)
        k.cmd("stop")
        k.p.wait(timeout=60)
    finally:
        k.stop()
    check("Kiln sends Block Entity Data for the placed sign", len(c.block_entity_data) >= 1, f"{len(c.block_entity_data)} packets")
    if c.block_entity_data:
        ok, msg = decode_with_vanilla("net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket",
                                      c.block_entity_data[0], out / "kiln_block_entity_data.bin")
        check("vanilla codec decodes Kiln's Block Entity Data", ok, msg)
    kiln_player_file = world / "world" / "players" / "data" / f"{offline_uuid(NAME)}.dat"
    kiln_player = nbt_file(kiln_player_file)
    kiln_level = nbt_file(world / "world" / "level.dat")
    kiln_time = val(get(kiln_level, "Data", "Time"))
    clocks = nbt_file(world / "world" / "data" / "minecraft" / "world_clocks.dat")
    kiln_day = val(get(clocks, "data", "minecraft:overworld", "total_ticks"))
    check("level.dat keeps fields Kiln does not own", val(get(kiln_level, "Data", "LevelName")) == "world"
          and get(kiln_level, "Data", "DataPacks") is not None, "")
    ok, msg = decode_with_vanilla("net.minecraft.network.protocol.game.ClientboundLevelChunkWithLightPacket",
                                  kiln_chunk, out / "kiln_chunk.bin")
    check("vanilla codec decodes Kiln's chunk with block entities", ok, msg)
    check("Kiln saved the time of day", kiln_day is not None and 1234 <= kiln_day < 1234 + 600, f"{kiln_day}")
    check("player file keeps tags Kiln does not model", val(get(kiln_player, "XpLevel")) == 5
          and get(kiln_player, "abilities") is not None and get(kiln_player, "recipeBook") is not None,
          f"XpLevel {val(get(kiln_player, 'XpLevel'))}")

    # ---- vanilla on the world Kiln saved ----
    print("== vanilla on the Kiln-saved world", flush=True)
    after = out / "after-world"
    if after.exists():
        shutil.rmtree(after)
    shutil.copytree(world, after)
    s = vanilla(after, a.vanilla_port, out / "vanilla-after.log")
    try:
        cx, cz = CHUNK
        s.cmd(f"forceload add {cx * 16} {cz * 16}")
        time.sleep(2)
        errors = [l for l in s.lines if re.search(r"Couldn't load chunk|Failed to load|Exception|corrupt|mismatched", l)]
        check("vanilla loads the Kiln-saved world without errors", not errors, "; ".join(errors[:3]))
        for (x, yy, z), block in FIXTURE_BLOCKS:
            if (x, yy, z) == LOADED_CHEST:
                continue
            name = block.split("{")[0].split("[")[0]
            line = s.query(f"execute if block {x} {yy} {z} minecraft:{name}", r"Test passed|Test failed")
            check(f"block {name} at {x},{yy},{z} intact", line and "Test passed" in line, line or "no answer")
        def has(pos, *needles):
            line = block_data(s, pos) or ""
            return all(n in line for n in needles), line
        for pos, needles in [((97, Y, -46), ('"Hello"', '"Kiln"', "is_waxed: 1b")),
                             ((99, Y, -46), ("stripe_top", "Lore line")),
                             ((101, Y, -46), ("Skully",)),
                             ((103, Y, -46), ("minecraft:pig", "minecraft:cow", "RequiredPlayerRange: 1s")),
                             ((105, Y, -46), ("minecraft:beef",)),
                             ((107, Y, -46), ("angler_pottery_sherd",)),
                             ((103, Y, -44), ("minecraft:emerald",)),
                             ((107, Y, -44), ("kiln_test",)),
                             ((97, Y, -42), ("page one",))]:
            ok, line = has(pos, *needles)
            check(f"block entity data at {pos} intact", ok, line[-160:])
        line = s.query(f"execute if block {LOADED_CHEST[0]} {LOADED_CHEST[1]} {LOADED_CHEST[2]} minecraft:air", r"Test passed|Test failed")
        check("the chest broken on Kiln is gone", line and "Test passed" in line, line or "no answer")
        ok, line = has(LOADED_CHEST)
        check("its block entity is gone", "is not a block entity" in line, line[-160:])
        for pos, name in [(PLACED_CHEST, "chest"), (PLACED_SIGN, "oak_sign")]:
            q = s.query(f"execute if block {pos[0]} {pos[1]} {pos[2]} minecraft:{name}", r"Test passed|Test failed")
            ok, line = has(pos)
            check(f"{name} placed on Kiln exists with a block entity", q and "Test passed" in q and "has the following block data" in line,
                  line[-160:])
        gt = s.query("time query gametime", r"game time is \d+")
        dt = s.query("time query time", r"Clock minecraft:overworld is at \d+|Unknown|Incorrect")
        g = int(re.search(r"game time is (\d+)", gt).group(1)) if gt else None
        said = dt.split("]: ")[-1] if dt else ""
        d = int(re.findall(r"\d+", said)[-1]) if re.search(r"\d", said) else None
        check("vanilla continues Kiln's game time", g is not None and kiln_time <= g <= kiln_time + 400, f"{g} vs Kiln {kiln_time}")
        check("vanilla continues Kiln's time of day", d is not None and kiln_day <= d <= kiln_day + 400, f"{said} vs Kiln {kiln_day}")

        c = Client("127.0.0.1", a.vanilla_port, NAME)
        c.pump(10, lambda: c.pos is not None and c.held_slot is not None and c.game_mode is not None)
        expect_pos = (KILN_TP[0] + KILN_WALK, KILN_TP[1], KILN_TP[2])
        check("vanilla places the player where Kiln saved them", close(c.pos[:3], expect_pos), f"{c.pos} vs {expect_pos}")
        check("vanilla restores Kiln's rotation", close(c.pos[3:], KILN_TP[3:], 1e-4), f"{c.pos[3:]}")
        check("vanilla restores Kiln's game mode (survival)", c.game_mode == 0, f"{c.game_mode}")
        check("vanilla restores Kiln's selected slot", c.held_slot == 4, f"{c.held_slot}")
        inv = entity_data(s, "Inventory") or ""
        check("sword keeps its components (enchantment, name)", "sharpness" in inv and "Excalibur" in inv, inv[-200:])
        check("potion keeps its contents", "swiftness" in inv, "")
        plain = re.findall(r"\{[^{}]*\}", inv)
        check("count change made on Kiln kept", any("count: 20" in i and 'id: "minecraft:stone"' in i for i in plain), "")
        check("item added on Kiln present", "minecraft:oak_log" in inv and "count: 13" in inv, "")
        check("items removed on Kiln absent", "minecraft:chest" not in inv and "oak_sign" not in inv, "")
        eq = entity_data(s, "equipment") or ""
        check("equipment (helmet, offhand) kept", "diamond_helmet" in eq and "shield" in eq, eq[-160:])
        xp = entity_data(s, "XpLevel") or ""
        check("XP level (not modeled by Kiln) kept", xp.rstrip().endswith(": 5"), xp[-80:])
        sp = entity_data(s, "respawn") or ""
        check("spawn point (loaded and saved by Kiln) kept", "90, 70, -20" in sp, sp[-120:])
        c.close()
    finally:
        s.stop()

    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
