"""Checks Kiln's clientbound packet encoders (HUD, scoreboard, teams, world effects, player
state, configuration/common packets) against the vanilla 26.3 jar.

1. Writes test vectors with `cargo run -p kiln-proto --example packet_vectors`.
2. Decodes all of them in one JVM with tools/VanillaDump.java, with the vanilla data pack's
   world registries loaded (dimension types, damage types, dialogs). A vector passes when
   vanilla consumes every byte, re-encoding the decoded packet reproduces the bytes exactly,
   every expected field value matches, and the packet id matches packets.json.
3. With --bless, copies the vectors into crates/kiln-proto/tests/testdata/clientbound.txt,
   the golden file `cargo test` compares the encoders against.

usage: python tools/packet_vectors.py [--out work/wp2-packets/vectors] [--bless]
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # no tools/__pycache__ from the imports below
from entity_vectors import java_dump, run, same  # noqa: E402
from vanilla_decode import ROOT, WORK  # noqa: E402

GOLDEN = ROOT / "crates/kiln-proto/tests/testdata/clientbound.txt"


def varint(v):
    out = bytearray()
    while True:
        if v & ~0x7F == 0:
            return bytes(out + bytes([v]))
        out.append(v & 0x7F | 0x80)
        v >>= 7


def parse_dump(text):
    sections, current = {}, None
    for line in text.splitlines():
        if line.startswith("== "):
            current = sections.setdefault(line[3:], {"fields": {}, "notes": []})
        elif current is not None and line.startswith(("!! ", "## ")):
            current["notes"].append(line)
        elif current is not None and "=" in line:
            path, value = line.split("=", 1)
            current["fields"][path] = value
    return sections


def check(out, manifest):
    report = json.loads((WORK / "generated/reports/packets.json").read_text(encoding="utf-8"))
    sections = parse_dump(java_dump("packets", out, "full"))
    failures = 0
    for name, _cls, key, packet_id, *_ in manifest:
        state, packet_name = key.split("/", 1)
        problems = []
        want_id = report.get(state, {}).get("clientbound", {}).get(packet_name, {}).get("protocol_id")
        if want_id != int(packet_id):
            problems.append(f"packet id {packet_id}, packets.json says {want_id} for {state} {packet_name}")
        sec = sections.get(name)
        expect = (out / f"{name}.expect").read_text(encoding="utf-8").splitlines()
        if sec is None:
            problems.append("not decoded")
        else:
            problems += [n for n in sec["notes"] if n.startswith("!!")]
            for line in expect:
                approx = "~" in line.split("=", 1)[0]
                path, want = line.split("~" if approx else "=", 1)
                got = sec["fields"].get(path)
                if got is None or not same(got, want, approx):
                    problems.append(f"{path}: got {got!r}, want {'~' if approx else ''}{want!r}")
        reencode = next((n for n in (sec or {}).get("notes", []) if n.startswith("## reencode")), "")
        if sec is not None and reencode != "## reencode same":
            problems.append(reencode[3:] or "not re-encoded")
        print(f"  {name}: {'ok' if not problems else 'FAIL'} ({len(expect)} fields, {reencode[3:] or '-'})")
        for p in problems:
            print(f"    {p}")
        failures += bool(problems)
    return failures


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "wp2-packets" / "vectors"))
    ap.add_argument("--bless", action="store_true", help="update the golden file when every vector passes")
    args = ap.parse_args()
    out = Path(args.out).resolve()

    p = run(["cargo", "run", "-q", "-p", "kiln-proto", "--example", "packet_vectors", "--", str(out)])
    print(p.stdout.strip(), p.stderr.strip())
    if p.returncode != 0:
        sys.exit("example failed")
    manifest = [line.split() for line in (out / "manifest.txt").read_text(encoding="utf-8").splitlines()]
    print(f"{len(manifest)} vectors:")
    failures = check(out, manifest)
    print("PASS" if failures == 0 else f"FAIL ({failures} of {len(manifest)})")
    if failures == 0 and args.bless:
        lines = [f"{name} {(varint(int(pid)) + (out / f'{name}.bin').read_bytes()).hex()}" for name, _, _, pid, *_ in manifest]
        GOLDEN.parent.mkdir(parents=True, exist_ok=True)
        GOLDEN.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        print(f"blessed {GOLDEN.relative_to(ROOT)}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
