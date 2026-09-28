"""WASM plugins seen by the real 26.3 client: the example plugins (spawn protection, chat
formatter, counter) built for wasm32-wasip2 and loaded with KILN_PLUGINS_DIR. The client
(not an operator) tries to break and place blocks at the spawn and is refused, chats and
sees its line formatted, then breaks blocks far from spawn and asks /broken. Then hot
reload: the chat formatter's manifest changes and `/kiln plugins reload chat-format` makes the
next line come out in the new format; the heartbeat plugin gets `/beat 200` (a task that
follows the player) and is reloaded as `v2` while the task is in flight: the ping arrives from
v2, exactly once. Input goes to the game window with PostMessage (no focus stealing);
screenshots of the game window only (PrintWindow) go to work/plugins-view-*.png, and the
client's chat log is checked.

usage: python tools/plugins_view.py [--port 25588] [--keep]
"""

import argparse
import io
import ctypes
import os
import re
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import e2e  # noqa: E402

NAME = "KilnView"
EXAMPLES = {"chat-format": "chat_format", "counter": "counter", "heartbeat": "heartbeat", "spawn-protection": "spawn_protection"}


def build_plugins():
    plugins = e2e.ROOT / "plugins"
    cmd = ["cargo", "build", "--release", "--target", "wasm32-wasip2", "--manifest-path", str(plugins / "Cargo.toml")]
    if subprocess.run(cmd, cwd=e2e.ROOT).returncode:
        sys.exit("building the plugins failed")
    out = e2e.WORK / "plugins-view"
    shutil.rmtree(out, ignore_errors=True)
    for crate, stem in EXAMPLES.items():
        dest = out / crate
        dest.mkdir(parents=True)
        shutil.copy2(plugins / "examples" / crate / "plugin.toml", dest / "plugin.toml")
        shutil.copy2(plugins / "target" / "wasm32-wasip2" / "release" / f"{stem}.wasm", dest / "plugin.wasm")
    return out


def start_server(port, plugins):
    e2e.stop_server(port)
    # No KILN_OPS: the client must not be an operator (spawn protection exempts operators).
    env = {k: v for k, v in os.environ.items() if k != "KILN_OPS"}
    env.update(KILN_PORT=str(port), RUST_LOG="info", KILN_PLUGINS_DIR=str(plugins))
    exe = e2e.WORK / f"kiln-{port}{e2e.EXE.suffix}"
    shutil.copy2(e2e.EXE, exe)
    log = open(e2e.SERVER_LOG, "w", encoding="utf-8")
    flags = 0x00000200 | 0x01000000 if os.name == "nt" else 0
    p = subprocess.Popen([str(exe)], cwd=e2e.ROOT, env=env, stdin=subprocess.PIPE, stdout=log,
                         stderr=subprocess.STDOUT, creationflags=flags, text=True)
    (e2e.WORK / f"server-{port}.pid").write_text(str(p.pid))
    for _ in range(200):
        if "listening on" in e2e.server_log():
            return p
        time.sleep(0.1)
    sys.exit("server did not start:\n" + e2e.server_log())


class Window:
    """Posts input to the game window (its client area), without taking the focus."""

    def __init__(self, pid):
        self.user32 = ctypes.windll.user32
        self.hwnd = None

        @ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)
        def cb(h, _):
            p = ctypes.c_ulong()
            self.user32.GetWindowThreadProcessId(h, ctypes.byref(p))
            if p.value == pid and self.user32.IsWindowVisible(h):
                self.hwnd = h
                return False
            return True

        self.user32.EnumWindows(cb, 0)
        if not self.hwnd:
            sys.exit("no game window")

    def post(self, msg, wparam, lparam):
        self.user32.PostMessageW(ctypes.c_void_p(self.hwnd), msg, wparam, lparam)

    def left_click(self):
        class RECT(ctypes.Structure):
            _fields_ = [("l", ctypes.c_long), ("t", ctypes.c_long), ("r", ctypes.c_long), ("b", ctypes.c_long)]
        r = RECT()
        self.user32.GetClientRect(ctypes.c_void_p(self.hwnd), ctypes.byref(r))
        lp = ((r.b // 2) << 16) | (r.r // 2)
        # The game only attacks with a grabbed mouse, which it grabs when it thinks it has the
        # focus: tell GLFW so (the real focus stays where it is).
        self.post(0x0007, 0, 0)  # WM_SETFOCUS
        time.sleep(0.2)
        self.post(0x0201, 0x0001, lp)  # WM_LBUTTONDOWN
        time.sleep(0.1)
        self.post(0x0202, 0, lp)  # WM_LBUTTONUP

    def key(self, vk, scan):
        self.post(0x0100, vk, (scan << 16) | 1)  # WM_KEYDOWN
        time.sleep(0.05)
        self.post(0x0101, vk, (scan << 16) | 0xC0000001)  # WM_KEYUP

    def type(self, text):
        for ch in text:
            self.post(0x0102, ord(ch), 1)  # WM_CHAR
            time.sleep(0.03)

    def chat(self, text):
        self.key(0x54, 0x14)  # T opens the chat
        time.sleep(0.6)
        self.type(text)
        time.sleep(0.3)
        self.key(0x0D, 0x1C)  # Enter
        time.sleep(0.8)


def main():
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace", line_buffering=True)
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=25588)
    ap.add_argument("--keep", action="store_true", help="leave the client and server running")
    a = ap.parse_args()
    if subprocess.run(["cargo", "build", "--release", "-p", "kiln-server"], cwd=e2e.ROOT).returncode:
        sys.exit("build failed")
    plugins = build_plugins()
    e2e.SERVER_LOG = e2e.WORK / f"server-{a.port}.log"
    server = start_server(a.port, plugins)

    def console(cmd, wait=0.3):
        server.stdin.write(cmd + "\n")
        server.stdin.flush()
        time.sleep(wait)

    pid = None
    for attempt in range(3):
        try:
            pid = e2e.launch(a.port, NAME)
        except subprocess.CalledProcessError as err:
            print(f"client launch failed (attempt {attempt + 1}): {err}")
            continue
        deadline = time.time() + 180
        while time.time() < deadline and f"{NAME} finished loading terrain" not in e2e.server_log():
            if not e2e.alive(pid):
                break
            time.sleep(1)
        if e2e.alive(pid) and f"{NAME} finished loading terrain" in e2e.server_log():
            break
        print(f"client exited before joining (attempt {attempt + 1})")
    else:
        e2e.stop_server(a.port)
        sys.exit("the client did not join")

    shots = []

    def shot(tag):
        out = e2e.WORK / f"plugins-view-{tag}.png"
        e2e.screenshot(pid, out)
        shots.append(out)
        print("screenshot:", out, flush=True)

    def chat_log():
        log = (e2e.WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
        return [l for l in log.splitlines() if "[CHAT]" in l]

    def until(pattern, action, tries=4, wait=1.5):
        """Repeats an input until the client's chat shows `pattern`: posted input is dropped
        while the client is busy (e.g. loading terrain after a teleport)."""
        for _ in range(tries):
            before = len(chat_log())
            action()
            time.sleep(wait)
            if any(re.search(pattern, l) for l in chat_log()[before:]):
                return True
        return False

    time.sleep(4)
    win = Window(pid)
    console("time set 6000")
    console("gamerule minecraft:spawn_mobs false")
    console(f"gamemode creative {NAME}")
    console(f"give {NAME} minecraft:stone 64")
    # Superflat spawn (8, -60, 8): stand south of it looking down at the grass ahead.
    console(f"tp {NAME} 8.5 -60 11.5 180 60", wait=3)

    checks = {}
    # 1. Breaking at the spawn: refused, the block comes back, the client hears why.
    # A left click posted to the window works only while the game believes it has the focus;
    # otherwise `/kiln break` sends the same packet the client would.
    clicked = until("This area is protected", win.left_click, tries=1)
    print("left click reached the game:", clicked)
    checks["break at spawn refused"] = clicked or until("This area is protected", lambda: console(f"kiln break {NAME} 8 -61 9"))
    # 2. Placing there (a right click on the grass, as the client would send it): refused.
    checks["place at spawn refused"] = until("This area is protected", lambda: console(f"kiln use {NAME} 8 -61 10"))
    # 3. Chat, formatted by the chat-format plugin.
    checks["formatted chat"] = until(r"\[KilnView\] » hello from a wasm plugin", lambda: win.chat("hello from a wasm plugin"))
    shot("spawn")

    # 4. Far from spawn the same clicks work; the counter counts the breaks.
    console(f"tp {NAME} 300.5 -60 300.5 180 60", wait=6)

    def dig():
        for i in range(3):
            win.left_click()
            time.sleep(0.6)
            console(f"kiln break {NAME} 300 -61 {299 - i}")
        time.sleep(0.5)
        win.chat("/broken")

    checks["counter counts breaks far from spawn"] = until(r"You broke [1-9]\d* blocks; everyone: [1-9]", dig)
    shot("far")

    # 5. Hot reload of the chat formatter with a changed manifest: the next line is in the
    # new format.
    manifest = plugins / "chat-format" / "plugin.toml"
    manifest.write_text(manifest.read_text(encoding="utf-8").replace('name_color = "gold"', 'name_color = "aqua"\nprefix = "(v2) "'),
                        encoding="utf-8")
    console("kiln plugins reload chat-format", wait=2)
    checks["reload: new chat format"] = until(r"\(v2\) \[KilnView\] » after the reload", lambda: win.chat("after the reload"))
    # 6. Hot reload with a task in flight: /beat 200 (10 s), reload the heartbeat as v2 at
    # once; the ping comes from v2, once.
    heartbeat = plugins / "heartbeat" / "plugin.toml"
    checks["heartbeat scheduled"] = until(r"\[v1\] scheduled in 200 ticks", lambda: win.chat("/beat 200"))
    heartbeat.write_text(heartbeat.read_text(encoding="utf-8").replace('label = "v1"', 'label = "v2"'), encoding="utf-8")
    console("kiln plugins reload heartbeat", wait=1)
    deadline = time.time() + 20
    while time.time() < deadline and not any("[v2] ping" in l for l in chat_log()):
        time.sleep(0.5)
    pings = [l for l in chat_log() if "] ping" in l]
    checks["reload: the task in flight ran once, in v2"] = len(pings) == 1 and "[v2] ping 1" in pings[0]
    time.sleep(1)
    shot("reload")
    for l in chat_log()[-10:]:
        print("client:", l)
    for l in e2e.server_log().splitlines():
        if re.search(r"plugin|panicked|ERROR|WARN|reload", l):
            print("server:", l)
    for k, v in checks.items():
        print(f"{'OK  ' if v else 'FAIL'} {k}")
    if not a.keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        e2e.stop_server(a.port)
    sys.exit(0 if all(checks.values()) else 1)


if __name__ == "__main__":
    main()
