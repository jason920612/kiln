"""Server list ping like the client does it (handshake intent 1, status request, ping request).

usage: python3 slp.py <host> <port> [--protocol N] [--raw] [--twice] [--legacy]
Prints the status JSON exactly as received (--raw: the response packet's bytes in hex too),
the pong's payload check, and what the server does after (closes / answers a second request).
--legacy sends the 1.6 style `FE 01 FA ...` ping and prints the kick reply.
"""
import socket, struct, sys, time, json


def varint(n):
    out = b""
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out += bytes([b | 0x80])
        else:
            return out + bytes([b])


def read_varint(sock):
    n = 0
    shift = 0
    while True:
        b = sock.recv(1)
        if not b:
            raise EOFError("closed")
        n |= (b[0] & 0x7F) << shift
        if not b[0] & 0x80:
            return n
        shift += 7


def read_packet(sock):
    n = read_varint(sock)
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError("closed")
        data += chunk
    return data


def packet(pid, body=b""):
    p = varint(pid) + body
    return varint(len(p)) + p


def mcstring(s):
    b = s.encode()
    return varint(len(b)) + b


def main():
    args = sys.argv[1:]
    host, port = args[0], int(args[1])
    protocol = int(args[args.index("--protocol") + 1]) if "--protocol" in args else 777
    s = socket.create_connection((host, port), timeout=10)
    if "--legacy" in args:
        # 1.6: FE 01 FA, "MC|PingHost", length, protocol 78, host, port
        h = host.encode("utf-16-be")
        data = b"\xfe\x01\xfa" + struct.pack(">H", 11) + "MC|PingHost".encode("utf-16-be")
        data += struct.pack(">HBH", 7 + len(h), 78, len(host)) + h + struct.pack(">i", port)
        s.sendall(data)
        buf = b""
        try:
            while True:
                c = s.recv(4096)
                if not c:
                    break
                buf += c
        except socket.timeout:
            pass
        print("legacy reply bytes:", buf.hex())
        if buf[:1] == b"\xff":
            print("kick:", buf[3:].decode("utf-16-be", "replace").replace("\x00", "\0"))
        return
    s.sendall(packet(0, varint(protocol) + mcstring(host) + struct.pack(">H", port) + varint(1)))
    s.sendall(packet(0))
    try:
        resp = read_packet(s)
    except EOFError:
        print("closed without a status response")
        return
    pid = resp[0]
    # status response: id 0, string
    i = 1
    n = 0
    shift = 0
    while True:
        b = resp[i]
        i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            break
        shift += 7
    text = resp[i:i + n].decode()
    print("packet id", pid, "json length", n)
    if "--raw" in args:
        print(resp.hex())
    print(text)
    if "--twice" in args:
        s.sendall(packet(0))
        try:
            r2 = read_packet(s)
            print("second request answered, id", r2[0])
        except (EOFError, ConnectionError, socket.timeout) as e:
            print("second request: connection closed", type(e).__name__)
        return
    t = int(time.time() * 1000)
    s.sendall(packet(1, struct.pack(">q", t)))
    try:
        pong = read_packet(s)
        print("pong id", pong[0], "echo ok" if struct.unpack(">q", pong[1:9])[0] == t else "echo WRONG")
    except EOFError:
        print("closed without pong")
    try:
        print("after pong:", "data" if s.recv(1) else "closed")
    except (socket.timeout, ConnectionError):
        print("after pong: still open")


main()
