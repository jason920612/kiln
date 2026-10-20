"""RCON and Query replies of a server, as hex, for comparing two servers byte by byte.

usage: python3 rcon_probe.py <host> <rcon port> <password> <query port>
Needs ops/players set up by the caller. Prints one line per exchange.
"""
import socket
import struct
import sys
import time

host, rport, password, qport = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4])


def pkt(i, t, body):
    b = body.encode()
    return struct.pack("<iii", len(b) + 10, i, t) + b + b"\0\0"


def read_pkts(s, timeout=3.0, want=1):
    s.settimeout(timeout)
    out = []
    buf = b""
    while len(out) < want:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            out.append(("closed",))
            break
        buf += chunk
        while len(buf) >= 4:
            n = struct.unpack("<i", buf[:4])[0]
            if len(buf) < 4 + n:
                break
            i, t = struct.unpack("<ii", buf[4:12])
            out.append((i, t, buf[12:4 + n - 2].decode("utf-8", "replace")))
            buf = buf[4 + n:]
    return out


def exchange(s, label, data, want=1):
    s.sendall(data)
    time.sleep(0.15)
    print(label, read_pkts(s, want=want))


s = socket.create_connection((host, rport), timeout=5)
exchange(s, "cmd before login", pkt(10, 2, "list"))
exchange(s, "bad password", pkt(11, 3, "wrong"))
exchange(s, "empty password", pkt(12, 3, ""))
exchange(s, "login", pkt(13, 3, password))
exchange(s, "list", pkt(14, 2, "list"))
exchange(s, "unknown command", pkt(15, 2, "nonsense"))
exchange(s, "slash prefix", pkt(16, 2, "/list"))
exchange(s, "say", pkt(17, 2, "say hello from rcon"))
exchange(s, "time query", pkt(18, 2, "time query daytime"))
exchange(s, "bad type", pkt(19, 7, "x"))
exchange(s, "empty command", pkt(20, 2, ""))
exchange(s, "give error", pkt(21, 2, "give Nobody stone"))
exchange(s, "difficulty", pkt(22, 2, "difficulty"))
exchange(s, "seed", pkt(23, 2, "seed"))
exchange(s, "gamerule", pkt(24, 2, "gamerule minecraft:do_daylight_cycle"))
exchange(s, "setblock", pkt(25, 2, "setblock 0 100 0 minecraft:stone"))
exchange(s, "fill huge", pkt(26, 2, "help"), want=1)
s.close()

# a short garbage packet, and a wrong length
for label, data in [("garbage 3 bytes", b"\x01\x02\x03"), ("wrong length", struct.pack("<iii", 99, 1, 3) + b"x\0\0")]:
    t = socket.create_connection((host, rport), timeout=5)
    t.sendall(data)
    time.sleep(0.2)
    print(label, read_pkts(t, timeout=1.0))
    t.close()

# Query
u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
u.settimeout(2.0)
sess = b"\x00\x00\x00\x01"
u.sendto(b"\xfe\xfd\x09" + sess, (host, qport))
ch = u.recvfrom(2048)[0]
print("challenge reply", ch[:5].hex(), "token", ch[5:-1].decode(), "last", ch[-1])
token = int(ch[5:-1])
basic = u.sendto(b"\xfe\xfd\x00" + sess + struct.pack(">i", token), (host, qport))
print("basic", u.recvfrom(2048)[0])
u.sendto(b"\xfe\xfd\x00" + sess + struct.pack(">i", token) + b"\0\0\0\0", (host, qport))
print("full", u.recvfrom(2048)[0])
u.sendto(b"\xfe\xfd\x00" + sess + struct.pack(">i", token + 1), (host, qport))
try:
    print("wrong token", u.recvfrom(2048)[0])
except socket.timeout:
    print("wrong token: no reply")
u.sendto(b"\xfe\xfd\x00" + b"\x00\x00\x00\x02" + struct.pack(">i", token), (host, qport))
try:
    print("other session", u.recvfrom(2048)[0])
except socket.timeout:
    print("other session: no reply (same address keeps its challenge)")
