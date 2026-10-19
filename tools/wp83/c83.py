"""wp83: helpers for the real-client checks (run on the VM next to ~/wp83/K; client agent on 25999)."""
import json
import os
import sys
import time

sys.path.insert(0, os.path.expanduser("~/wp83"))
from ctl import C  # noqa: E402  (tools/wp58/ctl.py copied to ~/wp83)

CONSOLE = {"K": os.path.expanduser("~/wp83/K/console"), "V": os.path.expanduser("~/wp83/V/console")}


def con(cmd, side="K", wait=0.4):
    """A console command on Kiln (K) or the vanilla server (V)."""
    with open(CONSOLE[side], "w") as f:
        f.write(cmd + "\n")
    time.sleep(wait)


def shot(path):
    os.system(f"import -display :99 -window root {path}")
