"""wp85: what vanilla's registry loader accepts of the malformed-entry pack tools/wp85/packs/kiln85_bad, as tools/wp85/schema_accepted.txt.

usage: python3 tools/wp85/schema85.py [--write]   (on the VM, from the checkout; port 25596)
Starts a vanilla server with the pack; its loader refuses to start and names every entry it cannot decode ("Errors in element"). The
entries of the pack that it does not name are the accepted ones. Without --write the result is compared with schema_accepted.txt.
"""
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "packs", "kiln85_bad", "data", "kiln85")
HOME = os.path.expanduser("~")


def entries():
    out = []
    for dirpath, _, files in os.walk(ROOT):
        for f in sorted(files):
            rel = os.path.relpath(os.path.join(dirpath, f), ROOT).replace(os.sep, "/")[:-5]
            registry, name = rel.rsplit("/", 1)
            out.append((registry, "kiln85:" + name))
    return sorted(out)


def rejected_by_vanilla():
    env = dict(os.environ, NAME="bad", SEED="1", PACKS="kiln85_bad")
    subprocess.run(["bash", os.path.join(HERE, "sv85.sh"), "V", "fresh"], env=env, check=True, stdout=subprocess.DEVNULL)
    log = os.path.join(HOME, "wp85", "bad", "server.log")
    try:
        for _ in range(240):
            text = open(log, encoding="utf-8", errors="replace").read()
            if "Failed to load datapacks" in text or "Done (" in text:
                break
            time.sleep(0.5)
        names = set(re.findall(r">> Errors in element (\S+?):\s*$", text, re.M))
    finally:
        subprocess.run(["bash", os.path.join(HERE, "sv85.sh"), "V", "stop"], env=env, check=False)
    return names


def main():
    rejected = rejected_by_vanilla()
    lines = [
        "# Entries of tools/wp85/packs/kiln85_bad that vanilla 26.3 loads or refuses (its registry loader names the ones it cannot decode).",
        "# Made by tools/wp85/schema85.py --write on a vanilla server; crates/kiln-sim's registry_data test holds Kiln to it.",
    ]
    for registry, name in entries():
        lines.append(f"{'rejected' if name in rejected else 'accepted'} {registry} {name}")
    text = "\n".join(lines) + "\n"
    path = os.path.join(HERE, "schema_accepted.txt")
    if "--write" in sys.argv:
        open(path, "w", newline="\n").write(text)
        print("wrote", path)
    else:
        same = open(path).read() == text
        print("same as schema_accepted.txt" if same else "DIFFERENT from schema_accepted.txt:\n" + text)
        sys.exit(0 if same else 1)


if __name__ == "__main__":
    main()
