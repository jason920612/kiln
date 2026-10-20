#!/usr/bin/env python3
"""Sends handshake variants to a management server and prints the raw answer to each (wp86).

    auth_probe.py PORT SECRET [--tls] [--origin ORIGIN]

The secret and the origin go into the requests; the answers are printed with the websocket accept key
shortened, so the output of two servers can be compared with diff.
"""
import socket, ssl, sys, time

port, secret = int(sys.argv[1]), sys.argv[2]
tls = "--tls" in sys.argv
origin = sys.argv[sys.argv.index("--origin") + 1] if "--origin" in sys.argv else "https://good.example"
KEY = "dGhlIHNhbXBsZSBub25jZQ=="


def request(head, body=b"", read=1.0):
    s = socket.create_connection(("127.0.0.1", port), timeout=5)
    if tls:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        s = ctx.wrap_socket(s, server_hostname="localhost")
    try:
        s.sendall(head.encode("latin1") + body)
        out = b""
        end = time.time() + read
        s.settimeout(0.3)
        while time.time() < end:
            try:
                d = s.recv(4096)
            except socket.timeout:
                continue
            except (ConnectionError, ssl.SSLError):
                out += b"<reset>"
                break
            if not d:
                out += b"<eof>"
                break
            out += d
        return out.decode("latin1")
    finally:
        s.close()


def ws(extra="", method="GET", path="/", version="HTTP/1.1", upgrade=True, wsversion="13", key=True):
    lines = [f"{method} {path} {version}", "Host: x"]
    if upgrade:
        lines += ["Upgrade: websocket", "Connection: Upgrade"]
    if key:
        lines.append(f"Sec-WebSocket-Key: {KEY}")
    if wsversion:
        lines.append(f"Sec-WebSocket-Version: {wsversion}")
    return "\r\n".join(lines) + "\r\n" + extra + "\r\n"


cases = [
    ("bearer ok", ws(f"Authorization: Bearer {secret}\r\n")),
    ("bearer wrong", ws("Authorization: Bearer nope\r\n")),
    ("bearer empty", ws("Authorization: Bearer\r\n")),
    ("bearer lowercase", ws(f"Authorization: bearer {secret}\r\n")),
    ("bearer extra spaces", ws(f"Authorization: Bearer    {secret}  \r\n")),
    ("bearerx", ws(f"Authorization: Bearer{secret}\r\n")),
    ("basic", ws(f"Authorization: Basic {secret}\r\n")),
    ("none", ws()),
    ("proto ok + origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin: {origin}\r\n")),
    ("proto ok no space + origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1,{secret}\r\nOrigin: {origin}\r\n")),
    ("proto ok no origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\n")),
    ("proto ok bad origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin: https://evil.example\r\n")),
    ("proto ok empty origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin:\r\n")),
    ("proto wrong secret + origin", ws(f"Sec-WebSocket-Protocol: minecraft-v1, nope\r\nOrigin: {origin}\r\n")),
    ("proto other first", ws(f"Sec-WebSocket-Protocol: other, minecraft-v1, {secret}\r\nOrigin: {origin}\r\n")),
    ("proto only v1", ws(f"Sec-WebSocket-Protocol: minecraft-v1\r\nOrigin: {origin}\r\n")),
    ("both, bearer wrong", ws(f"Authorization: Bearer nope\r\nSec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin: {origin}\r\n")),
    ("both, bearer ok", ws(f"Authorization: Bearer {secret}\r\nSec-WebSocket-Protocol: minecraft-v1, nope\r\n")),
    ("post", ws(f"Authorization: Bearer {secret}\r\n", method="POST")),
    ("path /foo", ws(f"Authorization: Bearer {secret}\r\n", path="/foo")),
    ("path /?x=1", ws(f"Authorization: Bearer {secret}\r\n", path="/?x=1")),
    ("no upgrade headers", ws(f"Authorization: Bearer {secret}\r\n", upgrade=False)),
    ("ws version 8", ws(f"Authorization: Bearer {secret}\r\n", wsversion="8")),
    ("no ws version", ws(f"Authorization: Bearer {secret}\r\n", wsversion="")),
    ("no key", ws(f"Authorization: Bearer {secret}\r\n", key=False)),
    ("http 1.0", ws(f"Authorization: Bearer {secret}\r\n", version="HTTP/1.0")),
    ("plain get, bearer ok", f"GET / HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("plain get, no auth", "GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
    ("head", f"HEAD / HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("junk", "hello there\r\n\r\n"),
    ("lf only", f"GET / HTTP/1.1\nHost: x\nAuthorization: Bearer nope\n\n"),
]
for name, head in cases:
    print(f"## {name}")
    ans = request(head)
    ans = "\r\n".join(
        "sec-websocket-accept: <key>" if l.lower().startswith("sec-websocket-accept") else l
        for l in ans.split("\r\n")
    )
    print(ans.replace("\r\n", "\\r\\n\n"))
    print()

# ---- handshake details (second battery) ----
more = [
    ("v absent, upgrade+connection+origin", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nOrigin: http://x\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v absent, upgrade+connection, key", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v absent, upgrade only", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v absent, connection only", f"GET / HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v 99", ws(f"Authorization: Bearer {secret}\r\n", wsversion="99")),
    ("v empty", ws(f"Authorization: Bearer {secret}\r\n", wsversion=" ")),
    ("v 7", ws(f"Authorization: Bearer {secret}\r\n", wsversion="7")),
    ("v 0", ws(f"Authorization: Bearer {secret}\r\n", wsversion="0")),
    ("v13 upgrade foo", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: foo\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 upgrade WebSocket caps", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: WebSocket\r\nConnection: UPGRADE\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 connection keep-alive, upgrade", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 connection keep-alive only", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: keep-alive\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 upgrade missing", f"GET / HTTP/1.1\r\nHost: x\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 key empty", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key:\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 key short", f"GET / HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: abc\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer {secret}\r\n\r\n"),
    ("v13 extensions", ws(f"Authorization: Bearer {secret}\r\nSec-WebSocket-Extensions: permessage-deflate\r\n")),
    ("v13 other protocol only", ws(f"Authorization: Bearer {secret}\r\nSec-WebSocket-Protocol: chat\r\n")),
    ("with body", f"GET / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nAuthorization: Bearer nope\r\n\r\nhello"),
    ("long header", f"GET / HTTP/1.1\r\nHost: x\r\nX: {'a' * 9000}\r\n\r\n"),
    ("many headers", "GET / HTTP/1.1\r\nHost: x\r\n" + "".join(f"H{i}: v\r\n" for i in range(300)) + "\r\n"),
    ("absolute uri", f"GET http://x/ HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {secret}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"),
    ("expect 100", ws(f"Authorization: Bearer {secret}\r\nExpect: 100-continue\r\n")),
    ("OPTIONS no auth", "OPTIONS / HTTP/1.1\r\nHost: x\r\n\r\n"),
    ("auth twice", ws(f"Authorization: Bearer nope\r\nAuthorization: Bearer {secret}\r\n")),
    ("origin case", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin: HTTPS://GOOD.EXAMPLE\r\n")),
    ("origin trailing slash", ws(f"Sec-WebSocket-Protocol: minecraft-v1, {secret}\r\nOrigin: {origin}/\r\n")),
]
for name, head in more:
    print(f"## {name}")
    ans = request(head)
    ans = "\r\n".join(
        "sec-websocket-accept: <key>" if l.lower().startswith("sec-websocket-accept") else l
        for l in ans.split("\r\n")
    )
    print(ans.replace("\r\n", "\r\n\n"))
    print()
