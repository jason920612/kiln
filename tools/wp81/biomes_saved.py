"""wp81: the biomes stored in the chunks of two saved worlds (anvil), compared quart by quart: biomes_saved.py WORLD_A WORLD_B DIM_DIR CX0 CZ0 CX1 CZ1
(DIM_DIR: DIM-1 for the nether, '' for the overworld). Prints the number of quarts that differ and a few examples."""
import struct, sys, zlib, math, os


def nbt(b):
    p = [0]

    def rd(fmt):
        n = struct.calcsize(fmt)
        v = struct.unpack_from(fmt, b, p[0])
        p[0] += n
        return v[0] if len(v) == 1 else v

    def string():
        n = rd(">H")
        s = b[p[0]:p[0] + n].decode("utf-8", "replace")
        p[0] += n
        return s

    def payload(t):
        if t == 1: return rd(">b")
        if t == 2: return rd(">h")
        if t == 3: return rd(">i")
        if t == 4: return rd(">q")
        if t == 5: return rd(">f")
        if t == 6: return rd(">d")
        if t == 7:
            n = rd(">i"); v = b[p[0]:p[0] + n]; p[0] += n; return v
        if t == 8: return string()
        if t == 9:
            it = rd(">b"); n = rd(">i"); return [payload(it) for _ in range(n)]
        if t == 10:
            d = {}
            while True:
                tt = rd(">b")
                if tt == 0: return d
                k = string(); d[k] = payload(tt)
        if t == 11:
            n = rd(">i"); return list(struct.unpack_from(">%di" % n, b, _adv(p, 4 * n)))
        if t == 12:
            n = rd(">i"); return list(struct.unpack_from(">%dq" % n, b, _adv(p, 8 * n)))
        raise ValueError(t)

    t = rd(">b"); string()
    return payload(t)


def _adv(p, n):
    o = p[0]; p[0] += n; return o


def chunk(world, dim, cx, cz):
    f = os.path.join(world, dim, "region", "r.%d.%d.mca" % (cx >> 5, cz >> 5))
    if not os.path.exists(f): return None
    d = open(f, "rb").read()
    i = ((cx & 31) + (cz & 31) * 32) * 4
    off = (d[i] << 16 | d[i + 1] << 8 | d[i + 2]) * 4096
    if off == 0: return None
    ln = struct.unpack_from(">i", d, off)[0]
    comp = d[off + 4]
    raw = d[off + 5:off + 4 + ln]
    return nbt(zlib.decompress(raw) if comp == 2 else raw)


def quarts(c):
    out = {}
    for s in c.get("sections", []):
        y = s["Y"]
        b = s.get("biomes")
        if not b: continue
        pal = b["palette"]; data = b.get("data")
        if len(pal) == 1 or not data:
            idx = [0] * 64
        else:
            bits = max(1, math.ceil(math.log2(len(pal))))
            per = 64 // bits
            idx = []
            for k in range(64):
                w = data[k // per] & ((1 << 64) - 1)
                idx.append((w >> ((k % per) * bits)) & ((1 << bits) - 1))
        for k in range(64):
            out[(y, k)] = pal[idx[k]]
    return out


def main():
    a, bw, dim = sys.argv[1], sys.argv[2], sys.argv[3]
    cx0, cz0, cx1, cz1 = map(int, sys.argv[4:8])
    diff = tot = 0
    ex = []
    for cx in range(cx0, cx1 + 1):
        for cz in range(cz0, cz1 + 1):
            ca, cb = chunk(a, dim, cx, cz), chunk(bw, dim, cx, cz)
            if ca is None or cb is None:
                print("missing", cx, cz, ca is None, cb is None); continue
            qa, qb = quarts(ca), quarts(cb)
            for k in qa:
                tot += 1
                if qa[k] != qb.get(k):
                    diff += 1
                    if len(ex) < 12: ex.append((cx, cz, k, qa[k], qb.get(k)))
    print("quarts", tot, "different", diff)
    for e in ex: print(e)


def stored_biome(world, dim, x, y, z):
    c = chunk(world, dim, x >> 4, z >> 4)
    if c is None: return None
    q = quarts(c)
    return q.get((y >> 4, ((y & 15) >> 2) * 16 + ((z & 15) >> 2) * 4 + ((x & 15) >> 2)))
if __name__ == "__main__":
    main()
