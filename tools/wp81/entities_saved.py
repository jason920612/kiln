"""wp81: the entities stored in a world's entity region files of one dimension, by type and chunk: entities_saved.py WORLD DIM_PATH [TYPE]
(DIM_PATH like dimensions/minecraft/the_nether)."""
import os, sys, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from biomes_saved import nbt, struct, zlib


def chunks(world, dim):
    d = os.path.join(world, dim, "entities")
    if not os.path.isdir(d):
        return
    for f in sorted(os.listdir(d)):
        if not f.endswith(".mca"):
            continue
        rx, rz = map(int, f.split(".")[1:3])
        data = open(os.path.join(d, f), "rb").read()
        for i in range(1024):
            off = (data[i * 4] << 16 | data[i * 4 + 1] << 8 | data[i * 4 + 2]) * 4096
            if off == 0:
                continue
            ln = struct.unpack_from(">i", data, off)[0]
            comp = data[off + 4]
            raw = data[off + 5:off + 4 + ln]
            yield rx * 32 + (i & 31), rz * 32 + (i >> 5), nbt(zlib.decompress(raw) if comp == 2 else raw)


if __name__ == "__main__":
    world, dim = sys.argv[1], sys.argv[2]
    only = sys.argv[3] if len(sys.argv) > 3 else None
    by = collections.Counter()
    where = collections.defaultdict(list)
    for cx, cz, c in chunks(world, dim):
        for e in c.get("Entities", []):
            t = e.get("id", "?")
            by[t] += 1
            where[t].append((cx, cz))
    for t, n in by.most_common():
        print("%-32s %5d  e.g. chunks %s" % (t, n, where[t][:6]))
