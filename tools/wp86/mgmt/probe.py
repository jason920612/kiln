#!/usr/bin/env python3
"""Runs a script of management-server requests and prints a transcript (wp86).

    probe.py HOST PORT SECRET SCRIPT [--tls] [--console FIFO] [--no-obs]

Script lines:
    # text             a header line (printed)
    {json}             sent on the main connection, the reply printed
    !raw TEXT          TEXT sent as it is (a text frame), the replies printed
    !sleep SECONDS
    !console COMMAND   a command written to the server console fifo
    !obs               print what the observer connection (a second client) has received so far
    !reconnect         opens a new main connection
    !close             closes the main connection
Every reply and every notification printed is the raw text the server sent, so two servers' transcripts diff byte for byte.
The observer connection prints "[obs] ..." lines; messages carry no timing so transcripts stay comparable.
"""
import sys, time, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from wsrpc import Ws
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
import mcclient


def main():
    host, port, secret, script = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    tls = "--tls" in sys.argv
    console = sys.argv[sys.argv.index("--console") + 1] if "--console" in sys.argv else None
    use_obs = "--no-obs" not in sys.argv
    game_port = int(sys.argv[sys.argv.index("--game-port") + 1]) if "--game-port" in sys.argv else None
    games = {}
    main_ws = Ws(host, port, secret, tls=tls)
    obs = Ws(host, port, secret, tls=tls) if use_obs else None

    def show_obs(wait=0.4):
        if obs:
            for t in obs.drain(wait):
                print("[obs]", t)

    def show_main_pending(wait):
        for t in main_ws.drain(wait):
            print("<-", t)
            if t == "<closed>":
                raise EOFError

    for line in open(script, encoding="utf-8"):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        if line.startswith("#"):
            print(line)
            continue
        if line.startswith("!sleep "):
            time.sleep(float(line.split()[1]))
            continue
        if line.startswith("!console "):
            cmd = line[len("!console "):]
            print("console>", cmd)
            fd = os.open(console, os.O_WRONLY)
            os.write(fd, (cmd + "\n").encode())
            os.close(fd)
            time.sleep(0.7)
            show_main_pending(0.1)
            show_obs(0.3)
            continue
        if line.startswith("!join "):
            name = line.split()[1]
            c = mcclient.Client("127.0.0.1", game_port, name).login()
            c.until_play()
            games[name] = c
            print("game> joined", name)
            time.sleep(0.6)
            show_main_pending(0.1)
            show_obs(0.3)
            continue
        if line.startswith("!pkts "):
            name = line.split()[1]
            for pid, body in games[name].drain(0.8):
                if pid in (mcclient.CB_SYSTEM_CHAT, mcclient.CB_DISCONNECT, 33, 66):
                    print("game>", name, pid, body.hex())
                elif pid == -1:
                    print("game>", name, "closed")
            continue
        if line.startswith("!leave "):
            games.pop(line.split()[1]).close()
            time.sleep(0.6)
            show_main_pending(0.1)
            show_obs(0.3)
            continue
        if line == "!obs":
            show_obs(0.5)
            continue
        if line == "!reconnect":
            main_ws = Ws(host, port, secret, tls=tls)
            continue
        if line == "!close":
            main_ws.close()
            continue
        text = line[5:] if line.startswith("!raw ") else line
        print("->", text)
        try:
            main_ws.send_text(text)
            # a request with an id: wait for its reply (lookups can take a while), then for what trails it
            wanted = None
            try:
                o = json.loads(text)
                if isinstance(o, dict) and "method" in o and o.get("id") is not None and "result" not in o and "error" not in o:
                    wanted = o["id"]
            except ValueError:
                pass
            if wanted is not None:
                end = time.time() + 8
                while time.time() < end:
                    t = main_ws.recv_text(max(0.05, end - time.time()))
                    if t is None:
                        break
                    print("<-", t)
                    try:
                        r = json.loads(t)
                    except ValueError:
                        break
                    if isinstance(r, dict) and r.get("id") == wanted and ("result" in r or "error" in r):
                        break
            show_main_pending(0.3)
        except (EOFError, OSError) as e:
            if not isinstance(e, EOFError):
                print("<- <closed>")
            try:
                main_ws.sock.close()
            except OSError:
                pass
            main_ws = Ws(host, port, secret, tls=tls)
        show_obs(0.3)
    show_obs(0.5)


if __name__ == "__main__":
    main()
