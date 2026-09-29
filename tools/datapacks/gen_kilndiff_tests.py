"""Writes the game test definitions and structures of the `kilndiff` data pack (the pack
`tools/command_diff.py` puts in both servers' worlds): `test_instance`, `test_environment` and
`structure/*.nbt` under `tools/datapacks/kilndiff/data/kilndiff`.

usage: python tools/datapacks/gen_kilndiff_tests.py
"""

import gzip
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent / "kilndiff" / "data" / "kilndiff"

END, BYTE, SHORT, INT, LONG, FLOAT, DOUBLE, BYTE_ARRAY, STRING, LIST, COMPOUND, INT_ARRAY = range(12)


def name(s: str) -> bytes:
    b = s.encode("utf-8")
    return struct.pack(">H", len(b)) + b


def payload(kind: int, value) -> bytes:
    if kind == INT:
        return struct.pack(">i", value)
    if kind == STRING:
        return name(value)
    if kind == LIST:
        item_kind, items = value
        return bytes([item_kind]) + struct.pack(">i", len(items)) + b"".join(payload(item_kind, v) for v in items)
    if kind == COMPOUND:
        out = b""
        for key, (k, v) in value.items():
            out += bytes([k]) + name(key) + payload(k, v)
        return out + bytes([END])
    raise ValueError(kind)


def structure(size, blocks):
    """`blocks`: (pos, block id, properties, nbt or None)."""
    palette, entries = [], []
    for pos, block, props, nbt in blocks:
        key = (block, tuple(sorted(props.items())))
        if key not in palette:
            palette.append(key)
        entry = {"pos": (LIST, (INT, list(pos))), "state": (INT, palette.index(key))}
        if nbt is not None:
            entry["nbt"] = (COMPOUND, nbt)
        entries.append(entry)
    pal = []
    for block, props in palette:
        p = {"id": (STRING, block)}
        if props:
            p["properties"] = (COMPOUND, {k: (STRING, v) for k, v in props})
        pal.append(p)
    root = {
        "size": (LIST, (INT, list(size))),
        "palette": (LIST, (COMPOUND, pal)),
        "blocks": (LIST, (COMPOUND, entries)),
        "entities": (LIST, (COMPOUND, [])),
        "DataVersion": (INT, 5023),
    }
    return gzip.compress(bytes([COMPOUND]) + name("") + payload(COMPOUND, root), mtime=0)


def test_block(mode):
    return ("minecraft:test_block", {"mode": mode})


def block(pos, kind, nbt=None):
    return (pos, kind[0], kind[1], nbt)


def stone(pos):
    return (pos, "minecraft:stone", {}, None)


STRUCTURES = {
    # The start block powers the accept block next to it.
    "accept": ((3, 2, 1), [stone((0, 0, 0)), stone((1, 0, 0)), stone((2, 0, 0)), block((0, 1, 0), test_block("start")), block((1, 1, 0), test_block("accept"))]),
    # ... and the fail block, with its message, next to it too.
    "fail": (
        (3, 2, 1),
        [stone((0, 0, 0)), block((0, 1, 0), test_block("start")), block((1, 1, 0), test_block("fail"), {"message": (STRING, "boom")}), block((2, 1, 0), test_block("accept"))],
    ),
    # Nothing is next to the start block: the test times out.
    "lonely": ((3, 2, 1), [block((0, 1, 0), test_block("start")), block((2, 1, 0), test_block("accept"))]),
    # No start block.
    "nostart": ((2, 2, 1), [block((0, 1, 0), test_block("accept"))]),
    # A structure that is not square, to see rotations.
    "wide": ((4, 2, 2), [stone((0, 0, 0)), stone((3, 0, 1)), block((0, 1, 0), test_block("start")), block((0, 1, 1), test_block("accept"))]),
}

TESTS = {
    "pass_fn": {"type": "minecraft:function", "function": "minecraft:always_pass", "environment": "minecraft:default", "structure": "minecraft:empty", "max_ticks": 1, "setup_ticks": 1},
    "optional_fn": {"type": "minecraft:function", "function": "minecraft:always_pass", "environment": "minecraft:default", "structure": "minecraft:empty", "max_ticks": 1, "required": False},
    "accept": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:accept", "max_ticks": 20},
    "fail": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:fail", "max_ticks": 20},
    "timeout": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:lonely", "max_ticks": 5},
    "timeout_optional": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:lonely", "max_ticks": 3, "required": False},
    "nostart": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:nostart", "max_ticks": 5},
    "rotated": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:wide", "max_ticks": 10, "rotation": "clockwise_90"},
    "padded": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:accept", "max_ticks": 10, "padding": 2, "sky_access": True},
    "with_rules": {"type": "minecraft:block_based", "environment": "kilndiff:rules", "structure": "kilndiff:accept", "max_ticks": 10},
    "with_inline_env": {
        "type": "minecraft:block_based",
        "environment": {"type": "minecraft:game_rules", "rules": {"minecraft:random_tick_speed": 7}},
        "structure": "kilndiff:accept",
        "max_ticks": 10,
    },
    "flaky": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:accept", "max_ticks": 10, "max_attempts": 3, "required_successes": 1},
    "missing_structure": {"type": "minecraft:block_based", "environment": "minecraft:default", "structure": "kilndiff:not_there", "max_ticks": 10},
    "slow/one": {"type": "minecraft:function", "function": "minecraft:always_pass", "environment": "minecraft:default", "structure": "minecraft:empty", "max_ticks": 1, "setup_ticks": 2},
    "slow/two": {"type": "minecraft:function", "function": "minecraft:always_pass", "environment": "kilndiff:rules", "structure": "minecraft:empty", "max_ticks": 1, "setup_ticks": 2},
}

ENVIRONMENTS = {
    "rules": {"type": "minecraft:game_rules", "rules": {"minecraft:max_command_sequence_length": 4321}},
}


def main():
    for rel, (size, blocks) in STRUCTURES.items():
        path = ROOT / "structure" / f"{rel}.nbt"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(structure(size, blocks))
    for kind, table in (("test_instance", TESTS), ("test_environment", ENVIRONMENTS)):
        for rel, definition in table.items():
            path = ROOT / kind / f"{rel}.json"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(definition, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
