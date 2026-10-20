"""A small protocol-level Minecraft 26.3 client (offline login) for wp86's side-by-side checks.

import mcclient; c = mcclient.Client("127.0.0.1", 25589, "Alice"); c.login(); c.until_play()
c.chat("hello"); pkts = c.drain(2.0)       # [(packet id, body bytes)], ids as in kiln-data's packets.rs
c.command("msg Bob hi"); c.close()

It answers keep-alives, pings, chunk batches and the configuration handshake by itself, and sends
chat the way an unsigned (no chat session) client does.
"""
import hashlib
import socket
import struct
import time
import uuid
import zlib

PROTOCOL = 777

# Play packet ids (kiln-data/src/gen/packets.rs).
CB_DISCONNECT = 32
CB_DISGUISED_CHAT = 33
CB_KEEP_ALIVE = 45
CB_LOGIN = 50
CB_PING = 62
CB_PLAYER_CHAT = 66
CB_PLAYER_INFO_UPDATE = 71
CB_SYSTEM_CHAT = 124
CB_CHUNK_BATCH_FINISHED = 11
SB_CHAT_ACK = 6
SB_CHAT_COMMAND = 7
SB_CHAT_COMMAND_SIGNED = 8
SB_CHAT = 9
SB_CHAT_SESSION_UPDATE = 10
SB_CHUNK_BATCH_RECEIVED = 11
SB_KEEP_ALIVE = 28
SB_CLIENT_INFORMATION = 0x0E
SB_PONG = 45


def varint(n):
    n &= 0xFFFFFFFF
    out = b""
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out += bytes([b | 0x80])
        else:
            return out + bytes([b])


def read_varint(buf, i=0):
    n = 0
    shift = 0
    while True:
        b = buf[i]
        i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            if n & 0x80000000:
                n -= 1 << 32
            return n, i
        shift += 7


def mcstring(s):
    b = s.encode()
    return varint(len(b)) + b


def read_string(buf, i):
    n, i = read_varint(buf, i)
    return buf[i:i + n].decode(), i + n


def offline_uuid(name):
    h = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    h[6] = (h[6] & 0x0F) | 0x30
    h[8] = (h[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(h))


class Client:
    def __init__(self, host, port, name, protocol=PROTOCOL):
        self.host, self.port, self.name, self.protocol = host, port, name, protocol
        self.sock = socket.create_connection((host, port), timeout=20)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.buf = b""
        self.threshold = -1
        self.uuid = offline_uuid(name)
        self.state = "handshake"
        self.log = []
        self.disconnect = None
        self.last_index = 0

    # ---- framing ----
    def send_raw(self, pid, body=b""):
        p = varint(pid) + body
        if self.threshold >= 0:
            if len(p) >= self.threshold:
                z = zlib.compress(p)
                p = varint(len(p)) + z
            else:
                p = varint(0) + p
        self.sock.sendall(varint(len(p)) + p)

    def recv_raw(self, timeout=10.0):
        """The next packet as (id, body), or None on timeout; raises EOFError when closed."""
        end = time.time() + timeout
        while True:
            try:
                n, i = read_varint(self.buf)
                if len(self.buf) - i >= n:
                    p = self.buf[i:i + n]
                    self.buf = self.buf[i + n:]
                    if self.threshold >= 0:
                        size, j = read_varint(p)
                        p = p[j:] if size == 0 else zlib.decompress(p[j:])
                    pid, j = read_varint(p)
                    return pid, p[j:]
            except IndexError:
                pass
            left = end - time.time()
            if left <= 0:
                return None
            self.sock.settimeout(left)
            try:
                chunk = self.sock.recv(65536)
            except socket.timeout:
                return None
            if not chunk:
                raise EOFError("closed")
            self.buf += chunk

    # ---- login and configuration ----
    def login(self, host=None):
        self.send_raw(0, varint(self.protocol) + mcstring(host or self.host) + struct.pack(">H", self.port) + varint(2))
        self.send_raw(0, mcstring(self.name) + self.uuid.bytes)
        while True:
            r = self.recv_raw()
            if r is None:
                raise TimeoutError("login")
            pid, body = r
            if pid == 0:  # login disconnect
                self.disconnect = body.decode("utf-8", "replace")
                raise ConnectionError("login disconnect: " + self.disconnect)
            if pid == 3:  # compression
                self.threshold, _ = read_varint(body)
            elif pid == 2:  # login finished
                self.send_raw(3)
                break
            elif pid == 1:
                raise ConnectionError("online mode server (encryption request)")
        self.state = "configuration"
        # client information (configuration): locale, view distance, chat mode, colours, skin, hand, filtering, listing, particles
        self.client_information(view=4)
        while self.state == "configuration":
            r = self.recv_raw()
            if r is None:
                raise TimeoutError("configuration")
            pid, body = r
            if pid == 2:
                self.disconnect = body.hex()
                raise ConnectionError("configuration disconnect")
            if pid == 15:  # known packs: none
                # the client has the vanilla core pack (Kiln disconnects clients without it)
                self.send_raw(7, varint(1) + mcstring("minecraft") + mcstring("core") + mcstring("26.3"))
            elif pid == 3:  # finish configuration
                self.send_raw(3)
                self.state = "play"
            elif pid == 4:
                self.send_raw(4, body)
            elif pid == 5:
                self.send_raw(5, body)
            elif pid == 9:  # resource pack push: accepted + loaded
                id16 = body[:16]
                self.send_raw(6, id16 + varint(3))
                self.send_raw(6, id16 + varint(0))
            elif pid == 20:  # code of conduct
                self.send_raw(9)
        return self

    def client_information(self, view=4, chat_mode=0, listing=True):
        """Client Information (configuration id 0, play id SB_CLIENT_INFORMATION)."""
        body = mcstring("en_us") + bytes([view]) + varint(chat_mode) + b"\x01" + bytes([0x7F]) + varint(1) + b"\x00"
        body += (b"\x01" if listing else b"\x00") + varint(0)
        self.send_raw(0 if self.state == "configuration" else SB_CLIENT_INFORMATION, body)

    # ---- play ----
    def handle(self, pid, body):
        """Answers what keeps the connection alive; returns True for packets the caller should see."""
        if pid == CB_KEEP_ALIVE:
            self.send_raw(SB_KEEP_ALIVE, body)
            return False
        if pid == CB_PING:
            self.send_raw(SB_PONG, body)
            return False
        if pid == CB_CHUNK_BATCH_FINISHED:
            self.send_raw(SB_CHUNK_BATCH_RECEIVED, struct.pack(">f", 20.0))
            return False
        if pid == CB_DISCONNECT:
            self.disconnect = body.hex()
        return True

    def until_play(self, timeout=30.0):
        """Reads until the Login packet (and a moment after), answering keep-alives."""
        end = time.time() + timeout
        seen = []
        while time.time() < end:
            r = self.recv_raw(1.0)
            if r is None:
                continue
            pid, body = r
            if self.handle(pid, body):
                seen.append((pid, body))
            if pid == CB_LOGIN:
                return seen
        raise TimeoutError("no Login packet")

    def drain(self, seconds):
        """Everything that arrives in the next `seconds`, minus the keep-alive traffic."""
        end = time.time() + seconds
        out = []
        while time.time() < end:
            try:
                r = self.recv_raw(max(0.05, end - time.time()))
            except EOFError:
                out.append((-1, b"closed"))
                break
            if r is None:
                continue
            if self.handle(*r):
                out.append(r)
        return out

    def chat(self, message, timestamp_ms=None, salt=0):
        ts = int(time.time() * 1000) if timestamp_ms is None else timestamp_ms
        body = mcstring(message) + struct.pack(">qq", ts, salt) + b"\x00" + varint(0) + b"\x00\x00\x00" + b"\x00"
        self.send_raw(SB_CHAT, body)

    def command(self, text):
        self.send_raw(SB_CHAT_COMMAND, mcstring(text))

    def command_signed(self, text, timestamp_ms=None, salt=0):
        # what a client without a chat session sends for a command with message arguments: no signatures
        ts = int(time.time() * 1000) if timestamp_ms is None else timestamp_ms
        body = mcstring(text) + struct.pack(">qq", ts, salt) + varint(0) + varint(0) + bytes(3) + bytes(1)
        self.send_raw(SB_CHAT_COMMAND_SIGNED, body)

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


def describe_chat(pid, body):
    """A PLAYER_CHAT or DISGUISED_CHAT packet as a readable summary (timestamps left out)."""
    if pid == CB_PLAYER_CHAT:
        i = 0
        gi, i = read_varint(body, i)
        sender = uuid.UUID(bytes=body[i:i + 16])
        i += 16
        idx, i = read_varint(body, i)
        has_sig = body[i]
        i += 1
        if has_sig:
            i += 256
        content, i = read_string(body, i)
        i += 16  # timestamp, salt
        n, i = read_varint(body, i)
        seen = []
        for _ in range(n):
            v, i = read_varint(body, i)
            if v == 0:
                i += 256
                seen.append("sig")
            else:
                seen.append(v - 1)
        return f"PLAYER_CHAT global={gi} sender={sender} index={idx} signed={bool(has_sig)} content={content!r} lastSeen={seen} rest={body[i:].hex()}"
    if pid == CB_DISGUISED_CHAT:
        return f"DISGUISED_CHAT {body.hex()}"
    if pid == CB_SYSTEM_CHAT:
        return f"SYSTEM_CHAT {body.hex()}"
    return f"packet {pid} {body[:40].hex()}"


# ---- movement (a client that stands still or hovers) ----
CB_PLAYER_POSITION = 73
SB_ACCEPT_TELEPORT = 0
SB_CLIENT_TICK_END = 13
SB_MOVE_POS = 30
SB_PLAYER_LOADED = 44


def _position(self, body):
    tid, i = read_varint(body, 0)
    x, y, z = struct.unpack(">ddd", body[i:i + 24])
    return tid, x, y, z


def hover(self, seconds, y_offset=0.0, on_ground=False, tick=0.05):
    """Joins the world like a client (confirms the teleport, reports loaded), then sends `seconds` of position reports
    at the spawn height + y_offset; returns [(t, packet id)] of what arrived (disconnect included)."""
    import time as _t
    pkts = self.until_play() if not getattr(self, "_in_play", False) else []
    self._in_play = True
    spawn = None
    end = _t.time() + 5
    while spawn is None and _t.time() < end:
        for pid, body in pkts + self.drain(0.2):
            if pid == CB_PLAYER_POSITION:
                spawn = _position(self, body)
        pkts = []
    if spawn is None:
        raise TimeoutError("no position")
    tid, x, y, z = spawn
    self.send_raw(SB_ACCEPT_TELEPORT, varint(tid))
    self.send_raw(SB_PLAYER_LOADED)
    self.send_raw(SB_CLIENT_TICK_END)
    out = []
    t0 = _t.time()
    n = 0
    while _t.time() - t0 < seconds:
        n += 1
        self.send_raw(SB_MOVE_POS, struct.pack(">dddB", x, y + y_offset, z, 1 if on_ground else 0))
        self.send_raw(SB_CLIENT_TICK_END)
        _t.sleep(tick)
        for pid, body in self.drain(0.0):
            if pid == CB_DISCONNECT:
                out.append((round(_t.time() - t0, 1), pid, body))
                return out
            if pid == -1:
                out.append((round(_t.time() - t0, 1), -1, b""))
                return out
    return out


Client.hover = hover
Client.position = _position
