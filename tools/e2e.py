"""End-to-end check: unit tests, release build, restart the server, smoke client,
vanilla-codec decode of captured packets and, with --client, a real 26.3 client join.

usage: python tools/e2e.py [--client] [--keep-client] [--port 25570]
"""

import argparse
import glob
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"
SERVER_LOG = WORK / "server.log"  # replaced per port in main()
EXE = ROOT / "target" / "release" / ("kiln.exe" if os.name == "nt" else "kiln")

# Packets captured by the smoke client -> vanilla codec that must decode them exactly.
DECODE = {
    "level_chunk_with_light.bin": "net.minecraft.network.protocol.game.ClientboundLevelChunkWithLightPacket",
    "block_update.bin": "net.minecraft.network.protocol.game.ClientboundBlockUpdatePacket",
}


def run(cmd, **kw):
    print("$", " ".join(map(str, cmd)), flush=True)
    return subprocess.run(cmd, cwd=ROOT, **kw)


def kill_pid(pid):
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
    else:
        subprocess.run(["kill", "-9", str(pid)], capture_output=True)


def stop_server(port):
    """Stops only the server this script started on `port` (other worktrees may run their own)."""
    pidfile = WORK / f"server-{port}.pid"
    if pidfile.exists():
        kill_pid(pidfile.read_text().strip())
        pidfile.unlink()
        time.sleep(0.5)


def start_server(port, world=None):
    stop_server(port)
    env = dict(os.environ, KILN_PORT=str(port), RUST_LOG=os.environ.get("RUST_LOG", "info"))
    if world:
        env["KILN_WORLD"] = str(Path(world).resolve())
    log = open(SERVER_LOG, "w", encoding="utf-8")
    flags = 0x00000008 | 0x00000200 | 0x01000000 if os.name == "nt" else 0
    # Run a copy so rebuilding (here or in another worktree) never hits a locked exe.
    exe = WORK / f"kiln-{port}{EXE.suffix}"
    import shutil
    shutil.copy2(EXE, exe)
    p = subprocess.Popen([str(exe)], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, creationflags=flags)
    (WORK / f"server-{port}.pid").write_text(str(p.pid))
    for _ in range(50):
        if "listening on" in SERVER_LOG.read_text(encoding="utf-8", errors="replace"):
            return
        time.sleep(0.1)
    sys.exit("server did not start:\n" + SERVER_LOG.read_text(encoding="utf-8", errors="replace"))


def server_log():
    return re.sub(r"\x1b\[[0-9;]*m", "", SERVER_LOG.read_text(encoding="utf-8", errors="replace"))


def screenshot(pid, out):
    ps = rf"""
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class PW {{ [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
 public struct RECT {{ public int L, T, R, B; }} }}
"@
$h = (Get-Process -Id {pid}).MainWindowHandle
$r = New-Object PW+RECT; [PW]::GetClientRect($h, [ref]$r) | Out-Null
$bmp = New-Object System.Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
$g = [System.Drawing.Graphics]::FromImage($bmp); $hdc = $g.GetHdc()
[PW]::PrintWindow($h, $hdc, 3) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose(); $bmp.Save("{out}"); $bmp.Dispose()
"""
    subprocess.run(["powershell", "-NoProfile", "-Command", ps], check=True)


def real_client(port, name, keep):
    out = subprocess.run(
        [sys.executable, str(ROOT / "tools" / "launch_client.py"), f"localhost:{port}", name],
        capture_output=True, text=True, check=True,
    ).stdout
    pid = int(re.search(r"client pid (\d+)", out).group(1))
    deadline = time.time() + 180
    ok = False
    while time.time() < deadline:
        log = server_log()
        if f"{name} finished loading terrain" in log:
            ok = True
            break
        if re.search(rf"{name} left", log):
            break
        time.sleep(1)
    time.sleep(3)
    shot = WORK / "client.png"
    try:
        screenshot(pid, shot)
        print("screenshot:", shot)
    except Exception as e:  # noqa: BLE001
        print("screenshot failed:", e)
    client_log = (WORK / "client" / "logs" / "latest.log").read_text(encoding="utf-8", errors="replace")
    for line in client_log.splitlines():
        if re.search(r"disconnect|Failed to decode|Exception in|\[CHAT\]", line) and "Realms" not in line:
            print("client:", line)
    if not keep:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
    return ok


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--client", action="store_true")
    ap.add_argument("--keep-client", action="store_true")
    ap.add_argument("--port", type=int, default=25570)
    ap.add_argument("--skip-tests", action="store_true")
    ap.add_argument("--stop", action="store_true", help="stop the server afterwards")
    ap.add_argument("--world", help="load this vanilla world save (sets KILN_WORLD)")
    a = ap.parse_args()

    if not a.skip_tests and run(["cargo", "test", "--workspace", "--quiet"]).returncode:
        sys.exit("unit tests failed")
    global SERVER_LOG
    SERVER_LOG = WORK / f"server-{a.port}.log"
    if run(["cargo", "build", "--release", "--quiet", "-p", "kiln-server"]).returncode:
        sys.exit("build failed")
    start_server(a.port, a.world)
    dumps = WORK / f"dumps-{a.port}"
    env = dict(os.environ, KILN_DUMP_DIR=str(dumps))
    if a.world:
        env["KILN_SMOKE_NO_BUILD"] = "1"  # the build test assumes the flat world
    if run([sys.executable, "tools/smoke_client.py", "127.0.0.1", str(a.port), "SmokeBot"], env=env).returncode:
        print(server_log())
        sys.exit("smoke client failed")
    for f, codec in DECODE.items():
        path = dumps / f
        if not path.exists() and a.world:
            continue
        if run([sys.executable, "tools/vanilla_decode.py", codec, str(path)], capture_output=True).returncode:
            sys.exit(f"vanilla codec rejected {f}")
        print(f"vanilla decode OK: {f}")
    if a.client:
        if not real_client(a.port, "KilnTest", a.keep_client):
            print(server_log())
            sys.exit("real client did not finish loading")
        print("real client joined")
    if a.stop:
        stop_server(a.port)
    print("E2E OK")


if __name__ == "__main__":
    main()
