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

    game = ROOT / "work" / "client"
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
    log = open(game / "client-stdout.log", "w", encoding="utf-8", errors="replace")
    # Detach so the client outlives the shell (and its job object) that started it.
    flags = 0
    if os.name == "nt":
        flags = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP | 0x01000000  # BREAKAWAY_FROM_JOB
    p = subprocess.Popen(args, cwd=game, stdout=log, stderr=subprocess.STDOUT, creationflags=flags)
    print(f"client pid {p.pid}; log: {game / 'client-stdout.log'}")


if __name__ == "__main__":
    main()
