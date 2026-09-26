"""Checks Kiln's entity packets and generated entity tables against the vanilla 26.3 jar.

1. Writes test vectors with `cargo run -p kiln-proto --example entity_vectors`.
2. Decodes every vector with tools/vanilla_decode.py (vanilla codec, no trailing bytes).
3. Decodes them again with tools/VanillaDump.java and compares every expected field value,
   and checks each packet id against the data generator's packets.json.
4. Compares kiln_data::entities (types, data fields, serializers) with the running game.

usage: python tools/entity_vectors.py [--out work/wp-entity/vectors]
"""

import argparse
import json
import subprocess
import sys
import zipfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.dont_write_bytecode = True  # no tools/__pycache__ from the import below
from vanilla_decode import ROOT, WORK, classpath  # noqa: E402


def run(cmd, **kw):
    print("$", " ".join(map(str, cmd)), flush=True)
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, **kw)


def java_dump(*args):
    cmd = ["java", "-cp", classpath(), str(ROOT / "tools" / "VanillaDump.java"), *map(str, args)]
    p = subprocess.run(cmd, cwd=WORK, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if p.returncode != 0:
        sys.exit(f"VanillaDump failed:\n{p.stdout}\n{p.stderr}")
    return p.stdout


def same(got, want, approx):
    try:
        g, w = float(got), float(want)
    except ValueError:
        return not approx and got == want
    return abs(g - w) <= 1e-3 if approx else g == w


def check_packets(out, manifest):
    failures = 0
    packet_ids = json.loads((WORK / "generated/reports/packets.json").read_text())["play"]["clientbound"]

    def decode(entry):
        name, cls = entry[0], entry[1]
        if "items" in entry[4:]:
            # Item stacks need default item components, which the static-registry decoder
            # lacks; VanillaDump binds empty ones and checks these below.
            return name, 0, "OK: decoded (skipped: item stacks, checked by VanillaDump)"
        p = subprocess.run(
            [sys.executable, str(ROOT / "tools/vanilla_decode.py"), cls, str(out / f"{name}.bin")],
            capture_output=True, text=True,
        )
        # The game's logger prefixes stdout lines after bootstrap.
        result = [l for l in p.stdout.splitlines() if "OK: decoded" in l or "FAIL" in l]
        return name, p.returncode, result[-1].split("[STDOUT]: ")[-1] if result else (p.stdout + p.stderr)[-300:]

    with ThreadPoolExecutor(4) as pool:
        for name, code, result in pool.map(decode, manifest):
            ok = code == 0 and result.startswith("OK: decoded")
            failures += not ok
            print(f"  vanilla_decode {name}: {result if ok else 'FAIL ' + result}")

    sections, current = {}, None
    for line in java_dump("packets", out).splitlines():
        if line.startswith("== "):
            current = sections.setdefault(line[3:], {"fields": {}, "notes": []})
        elif current is not None and line.startswith(("!! ", "## ")):
            current["notes"].append(line)
        elif current is not None and "=" in line:
            path, value = line.split("=", 1)
            current["fields"][path] = value

    for name, cls, packet_name, packet_id, *_ in manifest:
        problems = []
        want_id = packet_ids.get(packet_name, {}).get("protocol_id")
        if want_id != int(packet_id):
            problems.append(f"packet id {packet_id}, packets.json says {want_id} for {packet_name}")
        sec = sections.get(name)
        if sec is None:
            problems.append("not decoded")
        else:
            problems += [n for n in sec["notes"] if n.startswith("!!")]
            for line in (out / f"{name}.expect").read_text().splitlines():
                approx = "~" in line.split("=", 1)[0]
                path, want = line.split("~" if approx else "=", 1)
                got = sec["fields"].get(path)
                if got is None or not same(got, want, approx):
                    problems.append(f"{path}: got {got!r}, want {'~' if approx else ''}{want!r}")
        # Vanilla re-encoding the decoded packet must reproduce our bytes exactly.
        reencode = next((n for n in (sec or {}).get("notes", []) if n.startswith("## reencode")), "")
        if sec is not None and reencode != "## reencode same":
            problems.append(reencode[3:] or "not re-encoded")
        print(f"  fields {name}: {'ok' if not problems else 'FAIL'} ({len((out / f'{name}.expect').read_text().splitlines())} checked, {reencode[3:]})")
        for p in problems:
            print(f"    {p}")
        failures += bool(problems)
    return failures


def check_tables(out):
    jar = next((WORK / "versions").glob("*/server-*.jar"))
    with zipfile.ZipFile(jar) as z:
        # Every class that defines synched data, whether or not an entity type uses it.
        defining = {
            n[:-6].replace("/", ".")
            for n in z.namelist()
            if n.endswith(".class") and "/syncher/" not in n and b"SynchedEntityData" in z.read(n)
            and b"defineId" in z.read(n)
        }
    ours = set((out / "tables.txt").read_text().splitlines())
    our_classes = {line.split()[1] for line in ours if line.startswith("field ")}
    our_classes |= {line.split()[2] for line in ours if line.startswith("typeclass ")}
    class_list = out / "classes.txt"
    class_list.write_text("\n".join(sorted(defining | our_classes)))
    theirs = set(java_dump("entities", class_list).splitlines())

    failures = 0
    for kind in ("serializer", "type", "typeclass", "field"):
        a = {l for l in ours if l.startswith(kind + " ")}
        b = {l for l in theirs if l.startswith(kind + " ")}
        missing, extra = sorted(b - a), sorted(a - b)
        print(f"  {kind}: {len(a & b)} match, {len(missing)} missing from Kiln, {len(extra)} not in vanilla")
        for l in missing[:20]:
            print(f"    missing: {l}")
        for l in extra[:20]:
            print(f"    extra:   {l}")
        failures += bool(missing or extra)
    return failures


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "wp-entity" / "vectors"))
    args = ap.parse_args()
    out = Path(args.out).resolve()

    p = run(["cargo", "run", "-q", "-p", "kiln-proto", "--example", "entity_vectors", "--", str(out)])
    print(p.stdout.strip(), p.stderr.strip())
    if p.returncode != 0:
        sys.exit("example failed")
    manifest = [line.split() for line in (out / "manifest.txt").read_text().splitlines()]

    print("packets:")
    failures = check_packets(out, manifest)
    print("tables:")
    failures += check_tables(out)
    print("PASS" if failures == 0 else f"FAIL ({failures})")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
