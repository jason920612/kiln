"""wp63: what the observer's client is sent when the actor joins (run on the VM; the actor's client is restarted).

  KILN_PLAY_DIR=wp63 PYTHONPATH=tools/wp58 python3 tools/wp58/p63_join.py   (TARGET=V for the vanilla server)
"""
import json, os, subprocess, sys, time
import p61
from p61 import *
from p63_meta import spylog, keep, NOISE

port = "25591" if os.environ.get("TARGET") == "V" else "25585"
o = C(26000)
o.cmd("spy on")
subprocess.run(["bash", str(HOME / "wt" / "wp63-wp61-gaps" / "tools" / "wp58" / "vm_client63.sh"), "stop"])
time.sleep(6)
spylog(o)
env = dict(os.environ, KILN_CLIENT_PORT=port)
subprocess.Popen(["setsid", "bash", str(HOME / "wt" / "wp63-wp61-gaps" / "tools" / "wp58" / "vm_client63.sh")], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
time.sleep(70)
log = keep(spylog(o), NOISE + ("RemoveEntities",))
print("\n".join(log))
(HOME / "wp63" / f"join_{os.environ.get('TARGET', 'K')}.json").write_text(json.dumps(log, indent=1))
