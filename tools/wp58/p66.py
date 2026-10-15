"""wp66 helpers: one real client (the actor KilnTest, agent 25999) on the Kiln (TARGET=K, port 25588) or the vanilla (TARGET=V, port 25594)
server of wp66, console through the fifos of ~/wp66 and ~/wp66/vanilla. Runs on the VM: KILN_PLAY_DIR=wp66 PYTHONPATH=tools/wp58 python3 script.py"""
import json, math, os, re, sys, time
from pathlib import Path
os.environ.setdefault("KILN_PLAY_DIR", "wp66")
import p61
from p61 import *

p61.VDIR = HOME / "wp66" / "vanilla"
T = os.environ.get("TARGET", "K")
OUT = HOME / "wp66"
if T == "V":
    p61.LOG = p61.VDIR / "server.log"


def client(port=25999):
    return C(port)


def say(*a):
    print(*a, flush=True)


def vlog_lines(since):
    f = (p61.VDIR if T == "V" else OUT) / "server.log"
    with open(f, "rb") as fh:
        fh.seek(since)
        txt = fh.read().decode(errors="replace")
    return [ANSI.sub("", l) for l in txt.splitlines()]


def logpos():
    return ((p61.VDIR if T == "V" else OUT) / "server.log").stat().st_size


def mspt():
    f = (p61.VDIR if T == "V" else OUT) / "server.log"
    txt = ANSI.sub("", f.read_text(errors="replace"))
    ls = [l for l in txt.splitlines() if "mspt mean" in l]
    return re.sub(r"^\S+Z\s+\S+\s+\S+\s+", "", ls[-1])[:140] if ls else ""
