"""wp66: a tiny Anvil reader (zlib chunks): blocks of a section as palette indexes + the palette, no block changes needed to read a world."""
import struct, zlib, math
from pathlib import Path


def _read(buf, pos, t):
    if t == 1: return struct.unpack_from(">b", buf, pos)[0], pos + 1
    if t == 2: return struct.unpack_from(">h", buf, pos)[0], pos + 2
    if t == 3: return struct.unpack_from(">i", buf, pos)[0], pos + 4
    if t == 4: return struct.unpack_from(">q", buf, pos)[0], pos + 8
    if t == 5: return struct.unpack_from(">f", buf, pos)[0], pos + 4
    if t == 6: return struct.unpack_from(">d", buf, pos)[0], pos + 8
    if t == 7:
        n = struct.unpack_from(">i", buf, pos)[0]; return buf[pos + 4:pos + 4 + n], pos + 4 + n
    if t == 8:
        n = struct.unpack_from(">H", buf, pos)[0]; return buf[pos + 2:pos + 2 + n].decode("utf-8", "replace"), pos + 2 + n
    if t == 9:
        et = buf[pos]; n = struct.unpack_from(">i", buf, pos + 1)[0]; pos += 5; out = []
        for _ in range(n):
            v, pos = _read(buf, pos, et); out.append(v)
        return out, pos
    if t == 10:
        d = {}
        while True:
            tt = buf[pos]; pos += 1
            if tt == 0: return d, pos
            n = struct.unpack_from(">H", buf, pos)[0]; name = buf[pos + 2:pos + 2 + n].decode(); pos += 2 + n
            d[name], pos = _read(buf, pos, tt)
    if t == 11:
        n = struct.unpack_from(">i", buf, pos)[0]; return list(struct.unpack_from(f">{n}i", buf, pos + 4)), pos + 4 + 4 * n
    if t == 12:
        n = struct.unpack_from(">i", buf, pos)[0]; return list(struct.unpack_from(f">{n}q", buf, pos + 4)), pos + 4 + 8 * n
    raise ValueError(t)


def parse_nbt(buf):
    assert buf[0] == 10
    n = struct.unpack_from(">H", buf, 1)[0]
    return _read(buf, 3 + n, 10)[0]


def chunk_nbt(world_dir, cx, cz, dim_dir=""):
    rx, rz = cx >> 5, cz >> 5
    p = Path(world_dir) / dim_dir / "region" / f"r.{rx}.{rz}.mca"
    if not p.exists():
        p = Path(world_dir) / "dimensions" / "minecraft" / "overworld" / "region" / f"r.{rx}.{rz}.mca"
    if not p.exists():
        return None
    with open(p, "rb") as f:
        f.seek(4 * ((cx & 31) + (cz & 31) * 32))
        e = struct.unpack(">I", f.read(4))[0]
        off, cnt = e >> 8, e & 255
        if off == 0:
            return None
        f.seek(off * 4096)
        length, ct = struct.unpack(">IB", f.read(5))
        data = f.read(length - 1)
    if ct == 2:
        data = zlib.decompress(data)
    elif ct == 3:
        pass
    else:
        raise ValueError(f"compression {ct}")
    return parse_nbt(data)


def _entry(e):
    if isinstance(e, str):
        return (e, {})
    if "" in e:   # 26.x writes a property-less state as a bare string, which the list-of-compounds reading shows as {"": name}
        return (e[""], {})
    return (e.get("id") or e.get("Name"), e.get("properties") or e.get("Properties") or {})


def section_states(chunk, sy):
    """(palette list of (name, props), [4096 palette indexes in y*256+z*16+x order]) of section index sy, or None."""
    secs = chunk.get("sections", [])
    miny = chunk.get("yPos", -4)
    for k, s in enumerate(secs):
        if s.get("Y", miny + k) == sy:
            bs = s["block_states"]
            pal = [_entry(e) for e in bs["palette"]]
            if len(pal) == 1:
                return pal, [0] * 4096
            bits = max(4, math.ceil(math.log2(len(pal))))
            per = 64 // bits
            data = bs["data"]
            mask = (1 << bits) - 1
            idx = []
            for i in range(4096):
                w = data[i // per] & 0xFFFFFFFFFFFFFFFF
                idx.append((w >> ((i % per) * bits)) & mask)
            return pal, idx
    return None
