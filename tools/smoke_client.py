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
PACKETS = json.loads((ROOT / "work/generated/reports/packets.json").read_text())
# Raw packet bodies are saved here for tools/VanillaDecode.java.
DUMP_DIR = Path(os.environ.get("KILN_DUMP_DIR", ROOT / "work" / "dumps"))
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
    deadline = time.time() + 5
    sent_chat = False
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
        elif i == cb("system_chat"):
            chats.append(b.d[b.i :])
        elif i == cb("keep_alive"):
            c.send(sb("keep_alive"), b.take(8))
    print(f"play: login={got_login} position={got_pos} wait_event={got_wait} chunks={len(chunks)} system_chat={len(chats)}")
    view = 8  # sent in client_information above
    assert got_login and got_pos and got_wait, "join incomplete"
    assert len(chunks) == (2 * view + 1) ** 2, f"expected {(2 * view + 1) ** 2} chunks"
    assert any(b"hello from smoke test" in m for m in chats), "chat was not echoed"
    print("OK")


if __name__ == "__main__":
    host = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 25565
    name = sys.argv[3] if len(sys.argv) > 3 else "SmokeBot"
    status(host, port)
    join(host, port, name)
