"""Samples a process's threads every second: CPU% per thread-name prefix. usage: thr73.py PROCNAME SECONDS"""
import os, sys, time, subprocess
name, secs = sys.argv[1], float(sys.argv[2])
end = time.time() + secs
pid = None
while time.time() < end and not pid:
    try:
        pid = subprocess.check_output(["pgrep", "-x", name], text=True).split()[0]
    except Exception:
        time.sleep(0.5)
hz = os.sysconf("SC_CLK_TCK")
def sample():
    out = {}
    for t in os.listdir(f"/proc/{pid}/task"):
        try:
            comm = open(f"/proc/{pid}/task/{t}/comm").read().strip()
            f = open(f"/proc/{pid}/task/{t}/stat").read().rsplit(")", 1)[1].split()
            ticks = int(f[11]) + int(f[12])
        except Exception:
            continue
        key = comm.rstrip("0123456789").rstrip("-")
        out[key] = out.get(key, 0) + ticks
    return out
prev = sample(); tp = time.time()
while time.time() < end:
    time.sleep(1.0)
    try:
        cur = sample()
    except Exception:
        break
    tc = time.time()
    parts = {k: (cur[k] - prev.get(k, 0)) / hz / (tc - tp) for k in cur}
    busy = {k: round(v, 2) for k, v in sorted(parts.items(), key=lambda kv: -kv[1]) if v > 0.05}
    load = open("/proc/loadavg").read().split()[0]
    print(time.strftime("%H:%M:%S"), "load", load, busy, flush=True)
    prev, tp = cur, tc
