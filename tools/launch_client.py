"""Launch the locally installed vanilla client (official launcher's .minecraft) in offline mode
and auto-connect to a server. Uses its own game directory (work/client) so the user's
options, saves and server list are untouched; libraries and assets are only read.

usage: python tools/launch_client.py [host:port] [username] [version]
"""

import json
import os
import subprocess
import sys
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = Path(os.environ.get("KILN_WORK") or ROOT / "work").resolve()
MC = Path(os.environ["APPDATA"]) / ".minecraft"


def allowed(rules):
    ok = not rules  # no rules: allowed
    for r in rules or []:
        os_ = r.get("os", {})
        match = os_.get("name", "windows") == "windows" and os_.get("arch", "x86_64") in ("x86_64", "x64")
        if match:
            ok = r["action"] == "allow"
    return ok


def main():
    server = sys.argv[1] if len(sys.argv) > 1 else "localhost:25570"
    name = sys.argv[2] if len(sys.argv) > 2 else "KilnTest"
    version = sys.argv[3] if len(sys.argv) > 3 else "26.3"

    vdir = MC / "versions" / version
    v = json.loads((vdir / f"{version}.json").read_text())
    cp = []
    for lib in v["libraries"]:
        if not allowed(lib.get("rules")):
            continue
        art = lib.get("downloads", {}).get("artifact")
        if art:
            path = MC / "libraries" / art["path"]
            if not path.exists():
                sys.exit(f"missing library {path}; start {version} once from the launcher")
            cp.append(str(path))
    cp.append(str(vdir / f"{version}.jar"))

    game = WORK / "client"
    natives = game / "natives"
    for d in ("java", "jna", "lwjgl", "netty"):
        (natives / d).mkdir(parents=True, exist_ok=True)
    options = game / "options.txt"
    if not options.exists():
        options.write_text(
            "\n".join(
                [
                    "onboardAccessibility:false",
                    "skipMultiplayerWarning:true",
                    "joinedFirstServer:true",
                    "pauseOnLostFocus:false",
                    "renderDistance:8",
                    "soundCategory_master:0.0",
                    "fullscreen:false",
                    "tutorialStep:none",
                ]
            )
            + "\n"
        )

    offline_uuid = uuid.UUID(bytes=bytes(16))  # replaced by the server's offline UUID on login
    args = [
        "java",
        "-Xmx2G",
        "--enable-native-access=ALL-UNNAMED",
        f"-Djava.library.path={natives / 'java'}",
        f"-Djna.tmpdir={natives / 'jna'}",
        f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives / 'lwjgl'}",
        f"-Dio.netty.native.workdir={natives / 'netty'}",
        "-cp",
        os.pathsep.join(cp),
        v["mainClass"],
        "--username", name,
        "--version", version,
        "--gameDir", str(game),
        "--assetsDir", str(MC / "assets"),
        "--assetIndex", v["assetIndex"]["id"],
        "--uuid", offline_uuid.hex,
        "--accessToken", "0",
        "--userType", "legacy",
        "--versionType", "release",
        "--width", "960",
        "--height", "540",
        "--quickPlayMultiplayer", server,
    ]
    # Launch through WMI so the client is not part of our process tree or job object
    # (it otherwise gets terminated along with the shell that started it). Arguments
    # go through a Java @argfile to keep the command line short; output goes to
    # the client's own logs/latest.log.
    argfile = game / "launch.args"
    quoted = ['"' + a.replace("\\", "\\\\") + '"' for a in args[1:]]
    argfile.write_text("\n".join(quoted) + "\n", encoding="utf-8")
    java = subprocess.run(["where", "java"], capture_output=True, text=True).stdout.splitlines()[0].strip()
    # OpenAL Soft's null backend: the test client needs no audio, and initializing the
    # system audio device intermittently killed the client during startup.
    bat = game / "launch.bat"
    bat.write_text(
        f'@echo off\r\nset ALSOFT_DRIVERS=null\r\n"{java}" @"{argfile}" > "{game / "client-stdout.log"}" 2>&1\r\n'
    )
    ps = (
        "$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments "
        f"@{{CommandLine='cmd.exe /c \"{bat}\"'; CurrentDirectory='{game}'}}; "
        "if ($r.ReturnValue -ne 0) { exit 1 }; "
        "for ($i = 0; $i -lt 50; $i++) { "
        "  $j = Get-CimInstance Win32_Process -Filter \"Name='java.exe'\" | "
        "       Where-Object { $_.CommandLine -like '*launch.args*' } | Select-Object -First 1; "
        "  if ($j) { $j.ProcessId; exit 0 }; Start-Sleep -Milliseconds 100 }; exit 1"
    )
    out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True)
    if out.returncode:
        sys.exit("failed to start client: " + out.stderr)
    print(f"client pid {out.stdout.strip()}; log: {game / 'logs' / 'latest.log'}")

if __name__ == "__main__":
    main()
