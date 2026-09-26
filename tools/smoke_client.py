"""Protocol smoke test: status ping, then login -> configuration -> play against a Kiln server.

Parses the chunk packet completely so framing and layout mistakes show up as errors.
It shares our reading of the spec, so it cannot catch symmetric misunderstandings;
the real 26.3 client is the final check.

usage: python tools/smoke_client.py [host] [port] [name]
"""

import json
import os
import socket
import struct
import sys
import time
import uuid
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
PACKETS = json.loads((WORK / "generated/reports/packets.json").read_text())
# Raw packet bodies are saved here for tools/VanillaDecode.java.
DUMP_DIR = Path(os.environ.get("KILN_DUMP_DIR", WORK / "dumps"))
DUMP_DIR.mkdir(parents=True, exist_ok=True)


def pid(state, direction, name):
    return PACKETS[state][direction]["minecraft:" + name]["protocol_id"]


def varint(v):
    v &= 0xFFFFFFFF
    out = bytearray()
    while True:
        if v & ~0x7F == 0:
            out.append(v)
            return bytes(out)
        out.append((v & 0x7F) | 0x80)
        v >>= 7


def string(s):
    b = s.encode()
    return varint(len(b)) + b


class Buf:
    def __init__(self, data):
        self.d = data
        self.i = 0

    def take(self, n):
        if self.i + n > len(self.d):
            raise EOFError("read past end of packet")
        b = self.d[self.i : self.i + n]
        self.i += n
        return b

    def varint(self):
        v = 0
        for k in range(5):
            b = self.take(1)[0]
            v |= (b & 0x7F) << (7 * k)
            if not b & 0x80:
                return v - (1 << 32) if v & (1 << 31) else v
        raise ValueError("varint too long")

    def string(self):
        return self.take(self.varint()).decode()

    def u8(self):
        return self.take(1)[0]

    def i16(self):
        return struct.unpack(">h", self.take(2))[0]

    def i32(self):
        return struct.unpack(">i", self.take(4))[0]

    def i64(self):
        return struct.unpack(">q", self.take(8))[0]

    def f32(self):
        return struct.unpack(">f", self.take(4))[0]

    def f64(self):
        return struct.unpack(">d", self.take(8))[0]

    def bool(self):
        return self.u8() != 0

    def done(self):
        return self.i == len(self.d)


class Conn:
    def __init__(self, host, port):
        self.s = socket.create_connection((host, port), timeout=10)
        self.rbuf = b""
        self.threshold = None

    def send(self, packet_id, body=b""):
        data = varint(packet_id) + body
        if self.threshold is not None:
            if len(data) >= self.threshold:
                data = varint(len(data)) + zlib.compress(data)
            else:
                data = varint(0) + data
        self.s.sendall(varint(len(data)) + data)

    def _fill(self):
        chunk = self.s.recv(65536)
        if not chunk:
            raise ConnectionError("server closed the connection")
        self.rbuf += chunk

    def recv(self):
        while True:
            b = Buf(self.rbuf)
            try:
                length = b.varint()
                if len(self.rbuf) - b.i >= length:
                    frame = self.rbuf[b.i : b.i + length]
                    self.rbuf = self.rbuf[b.i + length :]
                    break
            except EOFError:
                pass
            self._fill()
        if self.threshold is not None:
            f = Buf(frame)
            data_len = f.varint()
            frame = frame[f.i :]
            if data_len:
                assert data_len >= self.threshold, "compressed packet below threshold"
                frame = zlib.decompress(frame)
                assert len(frame) == data_len, "declared length mismatch"
        p = Buf(frame)
        return p.varint(), p


def parse_chunk(b):
    x, z = b.i32(), b.i32()
    for _ in range(b.varint()):
        kind = b.varint()
        longs = b.varint()
        assert kind in (1, 4, 5), kind
        b.take(8 * longs)
    data = Buf(b.take(b.varint()))
    sections = 0
    while not data.done():
        data.i16(), data.i16()
        for entries, direct_bits in ((4096, 16), (64, 7)):
            bpe = data.u8()
            if bpe == 0:
                data.varint()
                continue
            if bpe <= (8 if entries == 4096 else 3):
                for _ in range(data.varint()):
                    data.varint()
            per_long = 64 // bpe
            data.take(8 * -(-entries // per_long))
        sections += 1
    assert sections == 24, f"expected 24 sections, got {sections}"
    assert b.varint() == 0, "block entities"
    masks = []
    for _ in range(4):  # BitSet as a byte array (26.3 ByteBufCodecs.BIT_SET)
        masks.append(int.from_bytes(b.take(b.varint()), "little"))
    sky, block = masks[0], masks[1]
    for mask in (sky, block):
        n = b.varint()
        assert n == bin(mask).count("1"), "light array count does not match mask"
        for _ in range(n):
            assert b.varint() == 2048
            b.take(2048)
    assert b.done(), "trailing bytes in chunk packet"
    return x, z


REPORTS = ROOT / "work" / "generated" / "reports"
ITEMS = json.loads((REPORTS / "registries.json").read_text())["minecraft:item"]["entries"]
BLOCKS = json.loads((REPORTS / "blocks.json").read_text())


def item_id(name):
    return ITEMS["minecraft:" + name]["protocol_id"]


def default_state(name):
    return next(s["id"] for s in BLOCKS["minecraft:" + name]["states"] if s.get("default"))


def position(x, y, z):
    return struct.pack(">q", ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF))


def creative_slot(slot, item, count=1):
    # Untrusted slot: count, item id, 0 added and 0 removed components.
    return struct.pack(">h", slot) + varint(count) + varint(item) + varint(0) + varint(0)


def use_item_on(pos, face, seq):
    return varint(0) + position(*pos) + varint(face) + struct.pack(">fff", 0.5, 1.0, 0.5) + b"\x00\x00" + varint(seq)


# A 3-high stone pillar in front of spawn (the player spawns at 8.5, -60, 8.5 facing +z),
# plus an oak log placed and then broken again. Ground surface is y = -61.
PILLAR = [(8, -60, 12), (8, -59, 12), (8, -58, 12)]
LOG = (10, -60, 12)
TORCH = (6, -60, 12)


def build(c, sb):
    c.send(sb("set_creative_mode_slot"), creative_slot(36, item_id("stone")))
    c.send(sb("set_carried_item"), struct.pack(">h", 0))
    seq = 1
    for x, y, z in PILLAR:
        c.send(sb("use_item_on"), use_item_on((x, y - 1, z), 1, seq))  # click the top of the block below
        seq += 1
    c.send(sb("set_creative_mode_slot"), creative_slot(36, item_id("oak_log")))
    c.send(sb("use_item_on"), use_item_on((LOG[0], LOG[1] - 1, LOG[2]), 1, seq))
    seq += 1
    # Start digging (creative: instant break).
    c.send(sb("player_action"), varint(0) + position(*LOG) + bytes([1]) + varint(seq))
    seq += 1
    c.send(sb("set_creative_mode_slot"), creative_slot(36, item_id("torch")))
    c.send(sb("use_item_on"), use_item_on((TORCH[0], TORCH[1] - 1, TORCH[2]), 1, seq))


def check_build(updates, acks):
    stone = default_state("stone")
    for p in PILLAR:
        assert (p, stone) in updates, f"no stone block update at {p}: {updates}"
    log_states = [s for (p, s) in updates if p == LOG]
    assert len(log_states) == 2 and log_states[1] == default_state("air"), f"log place/break: {log_states}"
    assert (TORCH, default_state("torch")) in updates, "torch not placed"
    assert acks == [1, 2, 3, 4, 5, 6], f"acks {acks}"
    print(f"build: {len(updates)} block updates, acks {acks}")


def status(host, port):
    c = Conn(host, port)
    c.send(0, varint(777) + string(host) + struct.pack(">H", port) + varint(1))
    c.send(pid("status", "serverbound", "status_request"))
    _, b = c.recv()
    info = json.loads(b.string())
    t = int(time.time() * 1000)
    c.send(pid("status", "serverbound", "ping_request"), struct.pack(">q", t))
    _, b = c.recv()
    assert b.i64() == t
    print("status:", info)


def join(host, port, name):
    c = Conn(host, port)
    c.send(0, varint(777) + string(host) + struct.pack(">H", port) + varint(2))
    c.send(pid("login", "serverbound", "hello"), string(name) + uuid.uuid4().bytes)
    while True:
        i, b = c.recv()
        if i == pid("login", "clientbound", "login_compression"):
            c.threshold = b.varint()
        elif i == pid("login", "clientbound", "login_finished"):
            u = uuid.UUID(bytes=b.take(16))
            print("login finished:", u, b.string())
            break
        else:
            raise RuntimeError(f"unexpected login packet {i}")
    c.send(pid("login", "serverbound", "login_acknowledged"))

    cb = lambda n: pid("configuration", "clientbound", n)
    sb = lambda n: pid("configuration", "serverbound", n)
    info = string("en_us") + bytes([8]) + varint(0) + b"\x01" + bytes([0x7F]) + varint(1) + b"\x00\x01" + varint(0)
    c.send(sb("client_information"), info)
    registries = tags = 0
    while True:
        i, b = c.recv()
        if i == cb("select_known_packs"):
            packs = [(b.string(), b.string(), b.string()) for _ in range(b.varint())]
            print("known packs offered:", packs)
            body = varint(len(packs)) + b"".join(string(a) + string(bb) + string(cc) for a, bb, cc in packs)
            c.send(sb("select_known_packs"), body)
        elif i == cb("registry_data"):
            reg = b.string()
            n = b.varint()
            for _ in range(n):
                b.string()
                assert not b.bool(), "unexpected NBT payload"
            assert b.done()
            registries += 1
        elif i == cb("update_tags"):
            for _ in range(b.varint()):
                b.string()
                for _ in range(b.varint()):
                    b.string()
                    for _ in range(b.varint()):
                        b.varint()
                    tags += 1
            assert b.done()
        elif i == cb("finish_configuration"):
            break
    print(f"configuration: {registries} registries, {tags} tags")
    c.send(sb("finish_configuration"))

    cb = lambda n: pid("play", "clientbound", n)
    sb = lambda n: pid("play", "serverbound", n)
    chunks = set()
    got_login = got_pos = got_wait = False
    chats = []
    block_updates = []
    light_updates = []
    acks = []
    deadline = time.time() + 5
    sent_chat = False
    built = False
    while time.time() < deadline:
        c.s.settimeout(max(0.1, deadline - time.time()))
        try:
            i, b = c.recv()
        except socket.timeout:
            break
        if i == cb("login"):
            eid = b.i32()
            got_login = True
            print("play login, entity id", eid)
        elif i == cb("player_position"):
            tid = b.varint()
            pos = (b.f64(), b.f64(), b.f64())
            got_pos = True
            print("teleport", tid, pos)
            c.send(sb("accept_teleportation"), varint(tid) + struct.pack(">dddff", *pos, 0.0, 0.0))
        elif i == cb("game_event"):
            if b.u8() == 13:
                got_wait = True
        elif i == cb("level_chunk_with_light"):
            if not chunks and DUMP_DIR:
                (DUMP_DIR / "level_chunk_with_light.bin").write_bytes(b.d[b.i :])
            chunks.add(parse_chunk(b))
        elif i == cb("chunk_batch_finished"):
            c.send(sb("chunk_batch_received"), struct.pack(">f", 64.0))
            if not sent_chat and len(chunks) > 50:
                body = string("hello from smoke test") + struct.pack(">qq", 0, 0) + b"\x00" + varint(0) + bytes(3) + b"\x00"
                c.send(sb("chat"), body)
                sent_chat = True
            if not built and got_pos and len(chunks) > 50 and not os.environ.get("KILN_SMOKE_NO_BUILD"):
                build(c, sb)
                built = True
        elif i == cb("system_chat"):
            chats.append(b.d[b.i :])
        elif i == cb("block_update"):
            if not block_updates and DUMP_DIR:
                (DUMP_DIR / "block_update.bin").write_bytes(b.d[b.i :])
            v = b.i64()
            pos = (v >> 38, (v << 52 & (2**64 - 1)) >> 52, (v << 26 & (2**64 - 1)) >> 38)
            pos = tuple(c - (1 << 26) if i != 1 and c >= 1 << 25 else c for i, c in enumerate(pos))
            pos = (pos[0], pos[1] - (1 << 12) if pos[1] >= 1 << 11 else pos[1], pos[2])
            block_updates.append((pos, b.varint()))
        elif i == cb("light_update"):
            if not light_updates and DUMP_DIR:
                (DUMP_DIR / "light_update.bin").write_bytes(b.d[b.i :])
            light_updates.append((b.varint(), b.varint()))
        elif i == cb("block_changed_ack"):
            acks.append(b.varint())
        elif i == cb("keep_alive"):
            c.send(sb("keep_alive"), b.take(8))
    print(f"play: login={got_login} position={got_pos} wait_event={got_wait} chunks={len(chunks)} system_chat={len(chats)}")
    view = 8  # sent in client_information above
    assert got_login and got_pos and got_wait, "join incomplete"
    assert len(chunks) == (2 * view + 1) ** 2, f"expected {(2 * view + 1) ** 2} chunks"
    assert any(b"hello from smoke test" in m for m in chats), "chat was not echoed"
    if not os.environ.get("KILN_SMOKE_NO_BUILD"):
        check_build(block_updates, acks)
        assert (0, 0) in light_updates, f"no light update for the torch's chunk: {light_updates}"
    print("OK")


if __name__ == "__main__":
    host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 25565
    name = sys.argv[3] if len(sys.argv) > 3 else "SmokeBot"
    status(host, port)
    join(host, port, name)
