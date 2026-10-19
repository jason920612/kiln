"""What a server does with bad chat: too long, illegal characters, chat hidden / commands only, spam,
a bad acknowledgement, and a chat session it cannot verify. Prints what each client receives.

usage: python3 chat_errors.py <host> <port>
"""
import os
import struct
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mcclient as m

host, port = sys.argv[1], int(sys.argv[2])


def disconnect_text(c):
    """The reason of a play disconnect as hex (network NBT) plus any literal strings in it."""
    if c.disconnect is None:
        return None
    return c.disconnect


def show(tag, pkts):
    for pid, body in pkts:
        if pid in (m.CB_PLAYER_CHAT, m.CB_DISGUISED_CHAT, m.CB_SYSTEM_CHAT, m.CB_DISCONNECT):
            print(tag, m.describe_chat(pid, body))
        elif pid == -1:
            print(tag, "closed")


def fresh(name):
    c = m.Client(host, port, name).login()
    c.until_play()
    c.drain(0.5)
    return c


def case(title, name, action, wait=0.6):
    print("==", title)
    c = fresh(name)
    action(c)
    show("  ", c.drain(wait))
    if c.disconnect:
        print("   disconnect:", c.disconnect)
    c.close()
    time.sleep(0.3)


def long_message(c):
    c.chat("x" * 300)


def illegal(c):
    c.chat("hello § world")


def control(c):
    c.chat("hello \x07 world")


def hidden(c):
    c.client_information(chat_mode=2)
    time.sleep(0.2)
    c.chat("while hidden")


def commands_only(c):
    c.client_information(chat_mode=1)
    time.sleep(0.2)
    c.chat("commands only")
    time.sleep(0.2)
    c.command("list")


def hidden_command(c):
    c.client_information(chat_mode=2)
    time.sleep(0.2)
    c.command("list")


def spam(c):
    for i in range(14):
        c.chat(f"spam {i}")


def bad_ack(c):
    c.send_raw(m.SB_CHAT_ACK, m.varint(5))


def bad_update(c):
    # an update claiming bit 19 acknowledged although nothing was sent
    body = m.mcstring("hi") + struct.pack(">qq", int(time.time() * 1000), 0) + b"\x00" + m.varint(0) + bytes([0x00, 0x00, 0x08]) + bytes([0])
    c.send_raw(m.SB_CHAT, body)


def bad_checksum(c):
    body = m.mcstring("hi") + struct.pack(">qq", int(time.time() * 1000), 0) + b"\x00" + m.varint(0) + bytes(3) + bytes([7])
    c.send_raw(m.SB_CHAT, body)


def signature_without_session(c):
    # a signed message from a client that never sent a chat session
    body = m.mcstring("signed?") + struct.pack(">qq", int(time.time() * 1000), 1) + b"\x01" + bytes(256) + m.varint(0) + bytes(3) + bytes([0])
    c.send_raw(m.SB_CHAT, body)


def bad_session(c):
    # a chat session whose key is not signed by Mojang
    key = bytes(range(64))
    body = bytes(16) + struct.pack(">q", int(time.time() * 1000) + 3600_000) + m.varint(len(key)) + key + m.varint(8) + bytes(8)
    c.send_raw(m.SB_CHAT_SESSION_UPDATE, body)


def tell_nobody(c):
    c.command_signed("msg Nobody hi")


case("too long (300 chars)", "Longy", long_message, 1.0)
case("section sign", "Para", illegal, 1.0)
case("control character", "Ctrl", control, 1.0)
case("chat hidden, chat then command", "Hider", hidden)
case("commands only: chat and /list", "Cmdy", commands_only)
case("hidden: /list", "Hid2", hidden_command)
case("14 quick messages", "Spammer", spam, 1.0)
case("acknowledgement offset past the window", "Acker", bad_ack, 1.0)
case("acknowledged a message never sent", "Acker2", bad_update, 1.0)
case("wrong checksum", "Sumer", bad_checksum, 1.0)
case("signature without a chat session", "Sigger", signature_without_session, 1.0)
case("chat session with a bad key", "Keyer", bad_session, 1.0)
case("msg to nobody", "Lonely", tell_nobody, 1.0)
