"""Protocol lobby features seen by the real 26.3 client, one server run per scene:

1. code of conduct: the server has a `codeofconduct/en_us.txt`; the client shows it while
   configuring (work/protocol-view-conduct.png).
2. resource pack: a required server pack with a prompt, served from a local HTTP server; the
   client shows the prompt while configuring (work/protocol-view-pack.png).
3. dialog: no configuration tasks; once joined, `dialog show` opens an inline notice dialog
   (work/protocol-view-dialog.png).

Screenshots are of the game window only (PrintWindow). Nothing clicks: each scene shows the
first screen the server asks for.

usage: python tools/protocol_view.py [--port 25584] [--http-port 25594] [--scene NAME]
"""

import argparse
import http.server
import io
import os
import shutil
import subprocess
import sys
import threading
import time
import zipfile

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402

NAME = "KilnView"
CONDUCT = """Welcome to the Kiln test server!

1. Be kind to other players.
2. No griefing outside the arena.
3. Have fun building."""
DIALOG = ('dialog show KilnView {type:"minecraft:notice",title:{text:"Kiln lobby",color:"gold"},'
          'body:{type:"minecraft:plain_message",contents:"Dialogs from the server work."},'
          'action:{label:"Got it"}}')


def tiny_pack():
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("pack.mcmeta", '{"pack":{"description":"Kiln test pack","min_format":1,"max_format":999}}')
    return buf.getvalue()


def serve_pack(port):
    data = tiny_pack()

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header("Content-Type", "application/zip")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def start_server(port, env_extra):
    e2e.stop_server(port)
    env = dict(os.environ, KILN_PORT=str(port), RUST_LOG="info", KILN_OPS=NAME, **env_extra)
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


LAUNCHED = [0.0]
HOLD = [8]


def client_log():
    """The client's log of the current launch (empty until it rewrites latest.log)."""
    path = e2e.WORK / "client" / "logs" / "latest.log"
    if not path.exists() or path.stat().st_mtime < LAUNCHED[0]:
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def screenshot(pid, shot):
    """Retries while the window is still being created."""
    for _ in range(30):
        try:
            e2e.screenshot(pid, shot)
            return
        except subprocess.CalledProcessError:
            time.sleep(2)
    sys.exit("no game window to capture")


def launch_until(port, ready, timeout=180):
    """Launches the client (retrying JVM start crashes) until `ready()` holds; returns its pid."""
    for attempt in range(3):
        LAUNCHED[0] = time.time()
        try:
            pid = e2e.launch(port, NAME)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + timeout
        while time.time() < deadline and e2e.alive(pid):
            if ready():
                return pid
            time.sleep(1)
        if e2e.alive(pid):
            return pid
        print(f"client exited early (attempt {attempt + 1})")
    return None


def scene(port, env, shot, ready, after=None):
    e2e.SERVER_LOG = e2e.WORK / f"server-{port}.log"
    server = start_server(port, env)
    pid = launch_until(port, ready)
    if pid is None:
        e2e.stop_server(port)
        sys.exit("the client did not start")
    if after:
        after(server)
    time.sleep(HOLD[0])
    screenshot(pid, shot)
    print("screenshot:", shot)
    for line in e2e.server_log().splitlines():
        if "resource pack" in line or "Disconnecting" in line or "panicked" in line:
            print("server:", line)
    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
    e2e.stop_server(port)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25584)
    ap.add_argument("--http-port", type=int, default=25594)
    ap.add_argument("--scene", choices=["conduct", "pack", "dialog"])
    ap.add_argument("--hold", type=int, default=8, help="seconds on the screen before the screenshot")
    a = ap.parse_args()
    HOLD[0] = a.hold
    connecting = lambda: "Connecting to" in client_log()  # noqa: E731
    if a.scene in (None, "conduct"):
        folder = e2e.WORK / "m3-protocol" / "codeofconduct"
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "en_us.txt").write_text(CONDUCT, encoding="utf-8")
        scene(a.port, {"KILN_CODE_OF_CONDUCT": str(folder)}, e2e.WORK / "protocol-view-conduct.png", connecting)
    if a.scene in (None, "pack"):
        httpd = serve_pack(a.http_port)
        env = {
            "KILN_RESOURCE_PACK": f"http://127.0.0.1:{a.http_port}/kiln-pack.zip",
            "KILN_REQUIRE_RESOURCE_PACK": "true",
            "KILN_RESOURCE_PACK_PROMPT": '{"text":"The Kiln lobby needs its resource pack.","color":"gold"}',
        }
        scene(a.port, env, e2e.WORK / "protocol-view-pack.png", connecting)
        httpd.shutdown()
    if a.scene in (None, "dialog"):
        joined = lambda: f"{NAME} finished loading terrain" in e2e.server_log()  # noqa: E731

        def show(server):
            time.sleep(3)
            server.stdin.write(DIALOG + "\n")
            server.stdin.flush()

        scene(a.port, {}, e2e.WORK / "protocol-view-dialog.png", joined, show)


if __name__ == "__main__":
    main()
