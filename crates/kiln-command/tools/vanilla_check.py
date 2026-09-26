"""Run crates/kiln-command/tools/VanillaCheck.java against the unobfuscated server jar.

usage: python crates/kiln-command/tools/vanilla_check.py tree <commands.bin> [commands.json]
       python crates/kiln-command/tools/vanilla_check.py show <packet-class> <body.bin>
       python crates/kiln-command/tools/vanilla_check.py encode

Runs in work/wp-command so vanilla's logs stay out of the shared work/logs.
"""

import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent.parent
WORK = ROOT / "work"


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "show":
        args[2] = str(Path(args[2]).resolve())
    if args and args[0] == "tree":
        args[1] = str(Path(args[1]).resolve())
        report = Path(args[2]) if len(args) > 2 else WORK / "generated" / "reports" / "commands.json"
        args[2:] = [str(report.resolve())]
    cwd = WORK / "wp-command"
    cwd.mkdir(exist_ok=True)
    cmd = ["java", "-cp", classpath(), str(HERE / "VanillaCheck.java"), *args]
    sys.exit(subprocess.call(cmd, cwd=cwd))
