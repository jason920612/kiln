"""spawn-protection side by side: Boss (made operator) and Alice (not) join; Alice starts digging the block under her feet,
then Boss does the same far from nobody's business (he is an operator). Prints what each gets back.

usage: python3 spawn_probe.py <host> <port> <console fifo>
"""
import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcclient as m

host, port, fifo = sys.argv[1], int(sys.argv[2]), sys.argv[3]
CB_BLOCK_UPDATE = 8
CB_BLOCK_CHANGED_ACK = 4
SB_PLAYER_ACTION = 41


def pos_long(x, y, z):
    return ((x & 0x3FFFFFF) << 38) | ((z & 0x3FFFFFF) << 12) | (y & 0xFFF)


def join(name):
    c = m.Client(host, port, name).login()
    c.until_play()
    spawn = None
    for pid, body in c.drain(1.0):
        if pid == m.CB_PLAYER_POSITION:
            spawn = c.position(body)
    if spawn is None:
        raise SystemExit("no position for " + name)
    tid, x, y, z = spawn
    c.send_raw(m.SB_ACCEPT_TELEPORT, m.varint(tid) + struct.pack(">dddff", x, y, z, 0.0, 0.0))
    c.send_raw(m.SB_PLAYER_LOADED)
    c.send_raw(m.SB_CLIENT_TICK_END)
    c.drain(0.5)
    return c, (x, y, z)


boss, bpos = join("Boss")
alice, apos = join("Alice")
with open(fifo, "w") as f:
    f.write("op Boss\n")
time.sleep(1.0)
boss.drain(0.5)
alice.drain(0.5)
print("same spawn for both:", bpos[0] - apos[0] < 20)


def dig(c, name, pos):
    x, y, z = int(pos[0] // 1), int(pos[1] // 1) - 1, int(pos[2] // 1)
    c.send_raw(SB_PLAYER_ACTION, m.varint(0) + struct.pack(">Q", pos_long(x, y, z)) + bytes([1]) + m.varint(7))
    time.sleep(0.5)
    for pid, body in c.drain(0.8):
        if pid == CB_BLOCK_UPDATE:
            print(f"{name}: BLOCK_UPDATE (the block put back) {body.hex()[:20]}...")
        elif pid == CB_BLOCK_CHANGED_ACK:
            print(f"{name}: BLOCK_CHANGED_ACK {body.hex()}")
        elif pid == m.CB_SYSTEM_CHAT:
            print(f"{name}: SYSTEM_CHAT {body.hex()}")
        # (entity, chunk and sound packets are not part of the answer)


dig(alice, "Alice (not an operator)", apos)
dig(boss, "Boss (operator)", bpos)
