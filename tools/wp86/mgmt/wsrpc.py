#!/usr/bin/env python3
"""A small WebSocket + JSON-RPC client for the management server (wp86), written over sockets.

    from wsrpc import Ws
    ws = Ws("127.0.0.1", 25666, secret="x"*40)        # tls=True for wss (certificate not verified)
    ws.send_json({"jsonrpc": "2.0", "id": 1, "method": "minecraft:allowlist"})
    print(ws.recv_text())                              # the raw text of the next message

CLI: wsrpc.py HOST PORT SECRET [--tls] [--wait SECONDS] REQUEST_JSON...
     sends every request, prints the raw replies and then every message for --wait seconds.
"""
import base64, os, socket, ssl, struct, sys, time, hashlib


class HandshakeError(Exception):
    def __init__(self, status, headers, body):
        super().__init__(f"{status}")
        self.status, self.headers, self.body = status, headers, body


class Ws:
    def __init__(self, host, port, secret=None, tls=False, headers=None, protocol=None, bearer=True, timeout=10, raw_handshake=None):
        self.sock = socket.create_connection((host, port), timeout=timeout)
        if tls:
            ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
            ctx.check_hostname = False
            ctx.verify_mode = ssl.CERT_NONE
            self.sock = ctx.wrap_socket(self.sock, server_hostname=host)
        self.buf = b""
        key = base64.b64encode(os.urandom(16)).decode()
        h = [f"GET / HTTP/1.1", f"Host: {host}:{port}", "Upgrade: websocket", "Connection: Upgrade",
             f"Sec-WebSocket-Key: {key}", "Sec-WebSocket-Version: 13"]
        if secret is not None and bearer:
            h.append(f"Authorization: Bearer {secret}")
        if protocol:
            h.append(f"Sec-WebSocket-Protocol: {protocol}")
        for k, v in (headers or {}).items():
            h.append(f"{k}: {v}")
        self.sock.sendall(("\r\n".join(h) + "\r\n\r\n").encode())
        head = self._read_until(b"\r\n\r\n")
        lines = head.decode("latin1").split("\r\n")
        self.status_line = lines[0]
        self.resp_headers = [tuple(l.split(": ", 1)) for l in lines[1:] if ": " in l]
        status = int(lines[0].split()[1])
        if status != 101:
            n = 0
            for k, v in self.resp_headers:
                if k.lower() == "content-length":
                    n = int(v)
            body = self._read_n(n) if n else b""
            raise HandshakeError(lines[0], self.resp_headers, body)

    def _read_until(self, marker):
        while marker not in self.buf:
            d = self.sock.recv(4096)
            if not d:
                raise EOFError("closed")
            self.buf += d
        i = self.buf.index(marker) + len(marker)
        out, self.buf = self.buf[:i], self.buf[i:]
        return out[: -len(marker)]

    def _read_n(self, n):
        while len(self.buf) < n:
            d = self.sock.recv(4096)
            if not d:
                raise EOFError("closed")
            self.buf += d
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send_frame(self, opcode, payload, fin=True):
        b0 = (0x80 if fin else 0) | opcode
        n = len(payload)
        mask = os.urandom(4)
        if n < 126:
            hdr = struct.pack("!BB", b0, 0x80 | n)
        elif n < 65536:
            hdr = struct.pack("!BBH", b0, 0x80 | 126, n)
        else:
            hdr = struct.pack("!BBQ", b0, 0x80 | 127, n)
        masked = bytes(c ^ mask[i % 4] for i, c in enumerate(payload))
        self.sock.sendall(hdr + mask + masked)

    def send_text(self, s):
        self.send_frame(1, s.encode())

    def send_json(self, o):
        import json
        self.send_text(json.dumps(o, separators=(",", ":")))

    def recv_frame(self, timeout=5):
        """(opcode, payload) of the next frame; None on timeout."""
        self.sock.settimeout(timeout)
        try:
            h = self._read_n(2)
            op, ln = h[0] & 0x0F, h[1] & 0x7F
            if ln == 126:
                ln = struct.unpack("!H", self._read_n(2))[0]
            elif ln == 127:
                ln = struct.unpack("!Q", self._read_n(8))[0]
            return op, self._read_n(ln)
        except (socket.timeout, TimeoutError):
            return None

    def recv_text(self, timeout=5):
        """The raw text of the next text message (pings are answered); None on timeout, EOFError when closed."""
        end = time.time() + timeout
        while True:
            left = end - time.time()
            if left <= 0:
                return None
            f = self.recv_frame(left)
            if f is None:
                return None
            op, p = f
            if op == 1:
                return p.decode()
            if op == 9:
                self.send_frame(10, p)
            if op == 8:
                raise EOFError(f"close frame {p!r}")

    def drain(self, seconds):
        out = []
        end = time.time() + seconds
        while time.time() < end:
            try:
                t = self.recv_text(max(0.05, end - time.time()))
            except EOFError:
                out.append("<closed>")
                break
            if t is None:
                break
            out.append(t)
        return out

    def call(self, method, params=None, id=1, timeout=10):
        """Sends a request and returns the raw text of the reply with that id (notifications seen on the way are kept in self.notes)."""
        import json
        req = {"jsonrpc": "2.0", "id": id, "method": method}
        if params is not None:
            req["params"] = params
        self.send_json(req)
        return self.wait_reply(id, timeout)

    def wait_reply(self, id, timeout=10):
        import json
        if not hasattr(self, "notes"):
            self.notes = []
        end = time.time() + timeout
        while time.time() < end:
            t = self.recv_text(max(0.05, end - time.time()))
            if t is None:
                break
            try:
                o = json.loads(t)
            except ValueError:
                return t
            if isinstance(o, dict) and o.get("id") == id and ("result" in o or "error" in o):
                return t
            self.notes.append(t)
        return None

    def close(self):
        try:
            self.send_frame(8, b"")
        except OSError:
            pass
        self.sock.close()


def main():
    host, port, secret = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    args = sys.argv[4:]
    tls = "--tls" in args
    wait = 1.0
    if "--wait" in args:
        wait = float(args[args.index("--wait") + 1])
    reqs = [a for i, a in enumerate(args) if a not in ("--tls", "--wait") and not (i > 0 and args[i - 1] == "--wait")]
    ws = Ws(host, port, secret, tls=tls)
    for r in reqs:
        ws.send_text(r)
        print("->", r)
        for t in ws.drain(0.5):
            print("<-", t)
    for t in ws.drain(wait):
        print("<-", t)
    ws.close()


if __name__ == "__main__":
    main()
