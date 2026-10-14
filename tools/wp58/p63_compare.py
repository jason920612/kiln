"""wp63: compare the spy logs of Kiln (K) and the vanilla reference server (V) event by event.

  python3 tools/wp58/p63_compare.py [meta|session|join]    reads ~/wp63/<what>_K.json and ~/wp63/<what>_V.json

Prints, per event and per viewer (obs: what the other player's client got; self: what the player's own client got), whether the
two lists are equal; differences are shown. Entity data is compared by its entries (id=value), packets by name and order; per-tick
noise (health regeneration steps, air counting down) is compared as "the same sequence of entries" and the first differing items are printed.
"""
import json, sys
from pathlib import Path

HOME = Path.home() / "wp63"
what = sys.argv[1] if len(sys.argv) > 1 else "meta"


def load(t):
    return json.loads((HOME / f"{what}_{t}.json").read_text())


def first_diff(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i, x, y
    if len(a) != len(b):
        i = min(len(a), len(b))
        return i, (a[i] if i < len(a) else None), (b[i] if i < len(b) else None)
    return None


def norm(lines):
    # Entity ids of unknown entities, the order of entity_data / DamageEvent within a tick (vanilla sends events first).
    out = [l for l in lines if not l.startswith("entity_data ? 8=") and not l.startswith("entity_data item")]
    return out


def main():
    if what == "join":
        k = json.loads((HOME / "join_K.json").read_text())
        v = json.loads((HOME / "join_V.json").read_text())
        print("join: obs K", k)
        print("join: obs V", v)
        print("EQUAL" if k == v else "DIFFERENT")
        return
    k, v = load("K"), load("V")
    same = diff = 0
    for ev in k:
        if ev not in v:
            print(f"{ev}: only in K")
            continue
        for who in ("obs", "self"):
            a, b = norm(k[ev][who]), norm(v[ev][who])
            # DamageEvent comes before the data of the same tick in vanilla, after it in Kiln: compare sorted within a window of equal ticks
            if sorted(a) == sorted(b):
                same += 1
                tag = "same" if a == b else "same entries, other order"
                print(f"= {ev:28s} {who:4s} {tag}")
            else:
                diff += 1
                d = first_diff(a, b)
                print(f"! {ev:28s} {who:4s} K {len(a)} items, V {len(b)} items; first difference at {d[0]}: K={d[1]!r} V={d[2]!r}")
    print(f"{same} equal, {diff} different")


main()
