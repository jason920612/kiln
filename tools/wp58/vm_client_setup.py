"""Run on the VM: install the Linux 26.3 client files under ~/mc-client from the version json
(Mojang data, kept out of git: work/wp58/26.3.json, or the path in KILN_CLIENT_VERSION_JSON) and the asset index (34.json): jar, libraries (Linux rules),
asset objects. Everything comes from Mojang's own download URLs, the same ones the launcher uses."""
import hashlib, json, os, sys, urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

here = Path(__file__).resolve().parent
version_json = Path(os.environ.get("KILN_CLIENT_VERSION_JSON", here.parent.parent / "work" / "wp58" / "26.3.json"))
root = Path.home() / "mc-client"
v = json.loads(version_json.read_text())


def fetch(url, dest, sha1=None):
    dest = Path(dest)
    if dest.exists() and (sha1 is None or hashlib.sha1(dest.read_bytes()).hexdigest() == sha1):
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    for attempt in range(3):
        try:
            data = urllib.request.urlopen(url, timeout=60).read()
            if sha1 and hashlib.sha1(data).hexdigest() != sha1:
                raise ValueError("sha1 mismatch " + url)
            dest.write_bytes(data)
            return
        except Exception as e:  # noqa: BLE001
            if attempt == 2:
                print("FAILED", url, e)


def allowed(rules):
    ok = not rules
    for r in rules or []:
        os_ = r.get("os", {})
        if os_.get("name", "linux") == "linux" and os_.get("arch", "x86_64") in ("x86_64", "x64"):
            ok = r["action"] == "allow"
    return ok


jobs = []
c = v["downloads"]["client"]
jobs.append((c["url"], root / "versions/26.3/26.3.jar", c["sha1"]))
(root / "versions/26.3").mkdir(parents=True, exist_ok=True)
(root / "versions/26.3/26.3.json").write_text(version_json.read_text())
cp = []
for lib in v["libraries"]:
    if not allowed(lib.get("rules")):
        continue
    art = lib.get("downloads", {}).get("artifact")
    if art:
        jobs.append((art["url"], root / "libraries" / art["path"], art["sha1"]))
        cp.append(str(root / "libraries" / art["path"]))
cp.append(str(root / "versions/26.3/26.3.jar"))
(root / "classpath.txt").write_text(":".join(cp))
idx = v["assetIndex"]
(root / "assets/indexes").mkdir(parents=True, exist_ok=True)
fetch(idx["url"], root / "assets/indexes" / f"{idx['id']}.json", idx["sha1"])
index = json.loads((root / "assets/indexes" / f"{idx['id']}.json").read_text())
for name, o in index["objects"].items():
    h = o["hash"]
    jobs.append((f"https://resources.download.minecraft.net/{h[:2]}/{h}", root / "assets/objects" / h[:2] / h, h))
print(len(jobs), "files")
with ThreadPoolExecutor(16) as ex:
    list(ex.map(lambda j: fetch(*j), jobs))
print("done")
