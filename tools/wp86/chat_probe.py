"""Two unsigned clients chat on a server; prints every chat packet each of them receives.

usage: python3 chat_probe.py <host> <port> [--op-fifo FIFO]   (ops Alice through the console fifo once she joined)
"""
import sys, time, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcclient as m

host, port = sys.argv[1], int(sys.argv[2])
fifo = sys.argv[sys.argv.index("--op-fifo") + 1] if "--op-fifo" in sys.argv else None

a = m.Client(host, port, "Alice").login()
a.until_play()
time.sleep(0.5)
b = m.Client(host, port, "Bob").login()
b.until_play()
a.drain(1.0)
b.drain(1.0)
if fifo:
    with open(fifo, "w") as f:
        f.write("op Alice\n")
    time.sleep(1.0)
    a.drain(0.5)
    b.drain(0.5)

def show(tag, pkts):
    for pid, body in pkts:
        if pid in (m.CB_PLAYER_CHAT, m.CB_DISGUISED_CHAT, m.CB_SYSTEM_CHAT, m.CB_DISCONNECT):
            print(tag, m.describe_chat(pid, body))
        elif pid == -1:
            print(tag, "closed")

steps = [
    ("chat hello", lambda: a.chat("hello world")),
    ("chat second", lambda: a.chat("second")),
    ("cmd say", lambda: a.command_signed("say hi there")),
    ("cmd me", lambda: a.command_signed("me waves")),
    ("cmd msg", lambda: a.command_signed("msg Bob secret")),
    ("cmd tellraw", lambda: a.command('tellraw @a "raw"')),
]
for name, fn in steps:
    print("==", name)
    fn()
    time.sleep(0.4)
    show("  A:", a.drain(0.6))
    show("  B:", b.drain(0.3))
a.close(); b.close()
