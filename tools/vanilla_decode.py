"""Run tools/VanillaDecode.java against the unobfuscated server jar and its libraries.

usage: python tools/vanilla_decode.py <codec-class> <body.bin>
"""

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"


def classpath():
    jars = sorted((WORK / "versions").glob("*/server-*.jar"))
    jars += sorted((WORK / "libraries").rglob("*.jar"))
    return os.pathsep.join(str(j) for j in jars)


if __name__ == "__main__":
    codec, body = sys.argv[1], Path(sys.argv[2]).resolve()
    cmd = ["java", "-cp", classpath(), str(ROOT / "tools" / "VanillaDecode.java"), codec, str(body)]
    sys.exit(subprocess.call(cmd, cwd=WORK))
