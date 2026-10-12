"""Compile tools/wp58/ClientCtl.java against the installed 26.3 client into work/wp58/clientctl.jar."""
import json, os, subprocess, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import launch_client as lc  # noqa: E402

root = Path(__file__).resolve().parent.parent.parent
v = json.loads((lc.MC / "versions" / "26.3" / "26.3.json").read_text())
cp = []
for lib in v["libraries"]:
    art = lib.get("downloads", {}).get("artifact")
    if art and lc.allowed(lib.get("rules")):
        cp.append(str(lc.MC / "libraries" / art["path"]))
cp.append(str(lc.MC / "versions" / "26.3" / "26.3.jar"))
out = root / "work" / "wp58" / "agent"
out.mkdir(parents=True, exist_ok=True)
subprocess.run(["javac", "-proc:none", "-d", str(out), "-cp", os.pathsep.join(cp),
                str(root / "tools" / "wp58" / "ClientCtl.java")], check=True)
(out / "MANIFEST.MF").write_text("Premain-Class: wp58.ClientCtl\n")
jar = root / "work" / "wp58" / "clientctl.jar"
subprocess.run(["jar", "cfm", str(jar), str(out / "MANIFEST.MF"), "-C", str(out), "wp58"], check=True)
print("built", jar)
