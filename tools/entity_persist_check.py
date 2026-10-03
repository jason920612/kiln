"""Vanilla acceptance test for Kiln's entity chunks (`entities/r.x.z.mca`).

1. Fixture: a copy of the reference world where the vanilla 26.3 server summons entities on a
   stone platform (items, an experience orb, an arrow stuck in the floor, a floating falling
   block, primed TNT, a snowball, a pig, a zombie in armor, and an armor stand, which Kiln does
   not simulate) and saves them.
2. Kiln on that world: a player joins next to them; Kiln must spawn the simulated entities
   for the client with vanilla's UUIDs (and not the others). The player drops emeralds; Kiln
   saves on `stop`. The entity chunk Kiln wrote must keep every entity: the unsimulated ones
   unchanged, the simulated ones with their UUID and state, plus the dropped emeralds.
3. Vanilla on the world Kiln saved: every entity is there (`data get entity <uuid>`), with the
   state checked for a few, and the log shows no entity loading errors.

usage: python tools/entity_persist_check.py [--out DIR] [--kiln-port 25586] [--vanilla-port 25592]
                                            [--skip-build]
"""

import argparse
import os
import re
import shutil
import struct
import subprocess
import sys
import time
import uuid
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
if not os.environ.get("KILN_WORK") and not (ROOT / "work").exists():
    # A git worktree under .claude/worktrees shares the main checkout's work/.
    for parent in ROOT.parents:
        if (parent / "work" / "server.jar").exists():
            os.environ["KILN_WORK"] = str(parent / "work")
            break
sys.path.insert(0, str(ROOT / "tools"))
sys.dont_write_bytecode = True
import persist_check as pc  # noqa: E402
from persist_check import RESULTS, WORK, Client, Nbt, Server, check, get, offline_uuid, port_free, val, vanilla  # noqa: E402
from smoke_client import PACKETS, Buf, creative_slot, item_id, position, varint  # noqa: E402

NAME = "EntityBot"
CHUNK = (8, -3)  # x 128..143, z -48..-33
PLATFORM_Y = 199
Y = 200
PLAYER = (136.5, 200.0, -38.5, 0.0, 0.0)  # more than 8 blocks from the orb
CUSTOM_DIAMOND = 'Item:{id:"minecraft:diamond",count:5},PickupDelay:32767s,Age:-32768s,CustomName:"Shiny",Tags:["kiln"]'

# (label, summon arguments); all in CHUNK.
SUMMONS = [
    ("item", f"item 130.5 {Y} -46.5 {{{CUSTOM_DIAMOND}}}"),
    ("aging item", f'item 131.5 {Y} -46.5 {{Item:{{id:"minecraft:gold_ingot",count:2}},PickupDelay:32767s,Age:100s}}'),
    ("orb", f"experience_orb 132.5 {Y} -46.5 {{Value:7s,Count:3}}"),
    ("arrow", f"arrow 133.5 {Y + 1} -46.5 {{Motion:[0.0,-1.0,0.0],life:-20000s,pickup:1b,damage:3.5d}}"),
    ("falling block", f'falling_block 134.5 {Y + 2} -46.5 {{BlockState:"minecraft:red_sand",NoGravity:1b,Time:-1000000}}'),
    ("tnt", f"tnt 135.5 {Y} -46.5 {{fuse:30000s}}"),
    ("snowball", f"snowball 136.5 {Y + 2} -46.5 {{NoGravity:1b}}"),
    ("pig", f'pig 138.5 {Y} -46.5 {{NoAI:1b,CustomName:"Porky",Health:7f,Tags:["kiln"]}}'),
    ("zombie", f'zombie 140.5 {Y} -46.5 {{NoAI:1b,PersistenceRequired:1b,Health:15f,IsBaby:1b,'
               f'equipment:{{head:{{id:"minecraft:iron_helmet",count:1}}}},Tags:["kiln"]}}'),
    ("armor stand", f'armor_stand 139.5 {Y} -46.5 {{CustomName:"Stand",ShowArms:1b}}'),
    # The cargo minecarts (Kiln simulates them; the loot table one stays unrolled).
    ("chest minecart", f'chest_minecart 129.5 {Y} -44.5 {{Items:[{{Slot:0b,id:"minecraft:diamond",count:5}},'
                       f'{{Slot:26b,id:"minecraft:apple",count:3}}],Tags:["kiln"]}}'),
    ("hopper minecart", f'hopper_minecart 130.5 {Y} -44.5 {{Items:[{{Slot:1b,id:"minecraft:coal",count:9}}],Enabled:0b}}'),
    ("furnace minecart", f'furnace_minecart 131.5 {Y} -44.5 {{Fuel:1234s,PushX:0.5d,PushZ:0.25d}}'),
    ("tnt minecart", f'tnt_minecart 132.5 {Y} -44.5 {{fuse:30000,explosion_power:6.0f}}'),
    ("loot minecart", f'chest_minecart 133.5 {Y} -44.5 {{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:99L}}'),
    # Chest boats and rafts, and a donkey with a chest and a saddle.
    ("chest boat", f'oak_chest_boat 134.5 {Y} -44.5 {{Items:[{{Slot:2b,id:"minecraft:diamond",count:6}},'
                   f'{{Slot:20b,id:"minecraft:apple",count:3}}],Tags:["kiln"]}}'),
    ("loot chest raft", f'bamboo_chest_raft 135.5 {Y} -44.5 {{LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:7L}}'),
    ("donkey", f'donkey 136.5 {Y} -44.5 {{NoAI:1b,Tame:1b,PersistenceRequired:1b,ChestedHorse:1b,'
               f'Items:[{{Slot:0b,id:"minecraft:emerald",count:5}},{{Slot:14b,id:"minecraft:stick",count:9}}],'
               f'equipment:{{saddle:{{id:"minecraft:saddle",count:1}}}},Tags:["kiln"]}}'),
    # wp30: a llama (strength 4: 12 slots) and a trader llama, with chest, items, coat and carpet.
    ("llama", f'llama 137.5 {Y} -44.5 {{NoAI:1b,Tame:1b,PersistenceRequired:1b,Strength:4,Variant:2,ChestedHorse:1b,'
              f'Items:[{{Slot:0b,id:"minecraft:emerald",count:5}},{{Slot:11b,id:"minecraft:stick",count:9}}],'
              f'equipment:{{body:{{id:"minecraft:red_carpet",count:1}}}},Tags:["kiln"]}}'),
    ("trader llama", f'trader_llama 138.5 {Y} -44.5 {{NoAI:1b,Tame:1b,PersistenceRequired:1b,Strength:2,Variant:3,DespawnDelay:30000,'
                     f'ChestedHorse:1b,Items:[{{Slot:3b,id:"minecraft:gold_ingot",count:7}}],'
                     f'equipment:{{body:{{id:"minecraft:blue_carpet",count:1}}}},Tags:["kiln"]}}'),
    # wp34: a riding stack (a cow carrying a husk that carries a chicken) saved by its root.
    ("riding stack", f'cow 141.5 {Y} -44.5 {{NoAI:1b,PersistenceRequired:1b,Tags:["kiln","stack"],'
                     f'Passengers:[{{id:"minecraft:husk",NoAI:1b,PersistenceRequired:1b,Tags:["kiln","pillion"],'
                     f'Passengers:[{{id:"minecraft:chicken",NoAI:1b,PersistenceRequired:1b,Tags:["kiln","topmost"]}}]}}]}}'),
]
SIMULATED = {"minecraft:cow", "minecraft:chicken","minecraft:item", "minecraft:experience_orb", "minecraft:arrow", "minecraft:falling_block",
             "minecraft:tnt", "minecraft:snowball", "minecraft:pig", "minecraft:zombie", "minecraft:chest_minecart",
             "minecraft:hopper_minecart", "minecraft:furnace_minecart", "minecraft:tnt_minecart", "minecraft:oak_chest_boat",
             "minecraft:bamboo_chest_raft", "minecraft:donkey", "minecraft:llama", "minecraft:trader_llama"}


def entity_types():
    import json
    reg = json.loads((WORK / "generated" / "reports" / "registries.json").read_text())
    return {v["protocol_id"]: k for k, v in reg["minecraft:entity_type"]["entries"].items()}


def region_chunk(world, chunk):
    """The NBT of an entity chunk as saved, or None."""
    cx, cz = chunk
    path = world / "dimensions" / "minecraft" / "overworld" / "entities" / f"r.{cx >> 5}.{cz >> 5}.mca"
    if not path.exists():
        return None
    data = path.read_bytes()
    i = ((cz & 31) << 5) | (cx & 31)
    loc = struct.unpack(">I", data[i * 4:i * 4 + 4])[0]
    if loc == 0:
        return None
    off = (loc >> 8) * 4096
    length, kind = struct.unpack(">IB", data[off:off + 5])
    body = data[off + 5:off + 4 + length]
    if kind != 2:
        raise ValueError(f"compression {kind}")
    return Nbt(zlib.decompress(body)).named()


def uuid_of(tag):
    ints = val(get(tag, "UUID"))
    if not ints:
        return None
    return uuid.UUID(bytes=struct.pack(">iiii", *ints))


def entities(world):
    root = region_chunk(world, CHUNK)
    if root is None:
        return {}
    out = {}
    for e in val(get(root, "Entities")) or ():
        out[uuid_of(e)] = e
    return out


def rider_chain(e):
    """The riders of `e` (its `Passengers`, and theirs), depth first."""
    for p in val(get(e, "Passengers")) or ():
        yield p
        yield from rider_chain(p)


def typ(e):
    return val(get(e, "id"))


class EntityClient(Client):
    """Also records Add Entity packets: (network id, UUID, type name)."""

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self.spawned = []
        add = pc.pid("play", "clientbound", "add_entity")
        types = entity_types()
        recv = self.c.recv

        def spy():
            i, b = recv()
            if i == add:
                s = Buf(b.d)
                s.i = b.i
                eid = s.varint()
                u = uuid.UUID(bytes=s.take(16))
                self.spawned.append((eid, u, types.get(s.varint())))
            return i, b
        self.c.recv = spy

    def drop_all(self, seq):
        self.send("play", "player_action", varint(4) + position(0, 0, 0) + bytes([0]) + varint(seq))


def build_fixture(out, port):
    fixture = out / "fixture"
    if fixture.exists():
        shutil.rmtree(fixture)
    shutil.copytree(WORK / "vanilla-world", fixture, ignore=shutil.ignore_patterns("session.lock", "logs"))
    s = vanilla(fixture, port, out / "vanilla-fixture.log")
    try:
        cx, cz = CHUNK
        s.query(f"forceload add {cx * 16} {cz * 16}", r"[Mm]arked chunk|already")
        time.sleep(2)
        s.query(f"kill @e[type=!player,x={cx * 16},y=0,z={cz * 16},dx=16,dy=400,dz=16]", r"Killed|No entity was found")
        s.query(f"fill {cx * 16} {PLATFORM_Y} {cz * 16} {cx * 16 + 15} {PLATFORM_Y} {cz * 16 + 15} minecraft:stone",
                r"Successfully filled|No blocks were filled|not loaded")
        s.query(f"fill {cx * 16} {Y} {cz * 16} {cx * 16 + 15} {Y + 6} {cz * 16 + 15} minecraft:air",
                r"Successfully filled|No blocks were filled|not loaded")
        for label, args in SUMMONS:
            line = s.query(f"summon {args}", r"Summoned new|Unable|Unknown|Incorrect|Expected|Invalid|not loaded")
            if not line or "Summoned new" not in line:
                sys.exit(f"vanilla refused to summon the {label}: {line}")
        c = Client("127.0.0.1", port, NAME)
        c.pump(5, lambda: c.pos is not None)
        c.loaded()
        s.cmd(f"gamemode creative {NAME}")
        s.cmd(f"tp {NAME} {PLAYER[0]} {PLAYER[1]} {PLAYER[2]} {PLAYER[3]} {PLAYER[4]}")
        c.pump(4)
        c.close()
        time.sleep(2)
        s.query("save-all flush", r"Saved the game", 120)
    finally:
        s.stop()
    return fixture


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(WORK / "entity-persist-check"))
    ap.add_argument("--kiln-port", type=int, default=25586)
    ap.add_argument("--vanilla-port", type=int, default=25592)
    ap.add_argument("--skip-build", action="store_true")
    a = ap.parse_args()
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    for port in (a.kiln_port, a.vanilla_port):
        if not port_free(port):
            sys.exit(f"port {port} is in use")
    if not a.skip_build and subprocess.run(["cargo", "build", "--release", "--quiet", "-p", "kiln-server"], cwd=ROOT).returncode:
        sys.exit("build failed")

    print("== vanilla summons and saves the fixture entities", flush=True)
    fixture = build_fixture(out, a.vanilla_port)
    before = entities(fixture / "world")
    kinds = sorted(typ(e) for e in before.values())
    # Mobs that spawned naturally (bats in the caves below) stay in: more unsimulated entities.
    if len(before) < len(SUMMONS):
        sys.exit(f"vanilla saved {len(before)} entities, expected at least {len(SUMMONS)}: {kinds}")
    print(f"   vanilla saved {kinds}", flush=True)
    arrow = next(e for e in before.values() if typ(e) == "minecraft:arrow")
    check("vanilla's arrow is stuck in the floor (fixture sanity)", val(get(arrow, "inGround")) == 1, "")

    # ---- Kiln on the vanilla world ----
    print("== Kiln on the vanilla-saved world", flush=True)
    world = out / "kiln-world"
    if world.exists():
        shutil.rmtree(world)
    shutil.copytree(fixture, world)
    exe = out / ("kiln-entity-persist" + (".exe" if os.name == "nt" else ""))
    shutil.copy2(ROOT / "target" / "release" / ("kiln.exe" if os.name == "nt" else "kiln"), exe)
    env = dict(os.environ, KILN_PORT=str(a.kiln_port), KILN_WORLD=str(world / "world"), KILN_OPS=NAME, RUST_LOG="info")
    k = Server([str(exe)], ROOT, env=env, log=out / "kiln.log")
    if not k.wait_for("listening on", 30):
        k.stop()
        sys.exit("Kiln did not start:\n" + "\n".join(k.lines[-20:]))
    try:
        c = EntityClient("127.0.0.1", a.kiln_port, NAME)
        want = {u for u, e in before.items() if typ(e) in SIMULATED}
        stack = next(e for e in before.values() if typ(e) == "minecraft:cow")
        riders = list(rider_chain(stack))
        check("vanilla saved the stack inside its root (fixture sanity)", [typ(r) for r in riders] == ["minecraft:husk", "minecraft:chicken"], f"{riders}")
        want |= {uuid_of(r) for r in riders}
        c.pump(15, lambda: c.pos is not None and want <= {u for _, u, _ in c.spawned})
        c.loaded()
        c.pump(2)
        seen = {u: t for _, u, t in c.spawned}
        missing = [typ(before[u]) if u in before else "rider" for u in want if u not in seen]
        check("Kiln spawns the vanilla-saved simulated entities for the client", not missing, f"missing {missing}")
        wrong = [(t, typ(before[u])) for u, t in seen.items() if u in before and t != typ(before[u])]
        check("with vanilla's UUIDs and types", not wrong, f"{wrong}")
        kept = [typ(before[u]) for u in seen if u in before and typ(before[u]) not in SIMULATED]
        check("and not the entities it keeps unsimulated", not kept, f"{kept}")
        c.creative(36, "emerald", 4)
        c.carry(0)
        c.pump(0.5)
        c.drop_all(1)
        c.pump(3)
        c.close()
        time.sleep(1)
        k.cmd("stop")
        k.p.wait(timeout=60)
    finally:
        k.stop()
    warnings = [l for l in k.lines if "kept as saved" in l or "entity chunk" in l]
    check("Kiln reads the vanilla entity chunk without warnings", not warnings, "; ".join(warnings[:3]))

    saved = entities(world / "world")
    lost = [typ(e) for u, e in before.items() if u not in saved]
    check("Kiln's entity chunk keeps every vanilla entity (by UUID)", not lost, f"lost {lost}")
    for u, e in before.items():
        if typ(e) not in SIMULATED:
            check(f"{typ(e)} (unsimulated) written back unchanged", saved.get(u) == e,
                  "" if saved.get(u) == e else f"{saved.get(u)}")
    by_type = {}
    for e in saved.values():
        by_type.setdefault(typ(e), []).append(e)

    def one(t, pred=lambda e: True):
        return next((e for e in by_type.get(t, []) if pred(e)), None)

    diamond = one("minecraft:item", lambda e: val(get(e, "Item", "id")) == "minecraft:diamond")
    check("item keeps its stack, infinite age and pickup delay, custom name and tags",
          diamond is not None and val(get(diamond, "Item", "count")) == 5 and val(get(diamond, "Age")) == -32768
          and val(get(diamond, "PickupDelay")) == 32767 and get(diamond, "CustomName") is not None
          and get(diamond, "Tags") is not None, f"{diamond}")
    gold = one("minecraft:item", lambda e: val(get(e, "Item", "id")) == "minecraft:gold_ingot")
    gold_before = next(e for e in before.values() if val(get(e, "Item", "id")) == "minecraft:gold_ingot")
    check("item age carries on from vanilla's", gold is not None and val(get(gold, "Age")) >= val(get(gold_before, "Age")),
          f"{val(get(gold_before, 'Age'))} -> {val(get(gold, 'Age')) if gold else None}")
    orb = one("minecraft:experience_orb")
    check("orb keeps value and count", orb is not None and val(get(orb, "Value")) == 7 and val(get(orb, "Count")) == 3, f"{orb}")
    arr = one("minecraft:arrow")
    check("arrow stays in the ground with its damage and pickup rule",
          arr is not None and val(get(arr, "inGround")) == 1 and val(get(arr, "damage")) == 3.5 and val(get(arr, "pickup")) == 1
          and get(arr, "inBlockState") is not None, f"{arr}")
    fb = one("minecraft:falling_block")
    check("falling block keeps its block and NoGravity",
          fb is not None and get(fb, "BlockState") == ("string", "minecraft:red_sand") and val(get(fb, "NoGravity")) == 1, f"{fb}")
    tnt = one("minecraft:tnt")
    check("TNT keeps burning its fuse", tnt is not None and 20000 < val(get(tnt, "fuse")) < 30000, f"{tnt}")
    pig = one("minecraft:pig")
    check("pig keeps its health, NoAI, custom name and tags",
          pig is not None and val(get(pig, "Health")) == 7.0 and val(get(pig, "NoAI")) == 1
          and get(pig, "CustomName") is not None and get(pig, "Tags") is not None, f"{pig}")
    zombie = one("minecraft:zombie")
    check("zombie keeps its health, baby flag, persistence and helmet",
          zombie is not None and val(get(zombie, "Health")) == 15.0 and val(get(zombie, "IsBaby")) == 1
          and val(get(zombie, "PersistenceRequired")) == 1
          and val(get(zombie, "equipment", "head", "id")) == "minecraft:iron_helmet", f"{zombie}")
    def slots(e):
        return {val(get(i, "Slot")): (val(get(i, "id")), val(get(i, "count"))) for i in val(get(e, "Items")) or ()}

    chest = one("minecraft:chest_minecart", lambda e: get(e, "Items") is not None)
    check("chest minecart keeps its slots and tags",
          chest is not None and slots(chest) == {0: ("minecraft:diamond", 5), 26: ("minecraft:apple", 3)}
          and get(chest, "Tags") is not None, f"{chest}")
    hopper = one("minecraft:hopper_minecart")
    check("hopper minecart keeps its slots and stays switched off",
          hopper is not None and slots(hopper) == {1: ("minecraft:coal", 9)} and val(get(hopper, "Enabled")) == 0, f"{hopper}")
    furnace = one("minecraft:furnace_minecart")
    check("furnace minecart burns its fuel and keeps its push",
          furnace is not None and 900 < val(get(furnace, "Fuel")) <= 1234 and get(furnace, "PushX") is not None
          and get(furnace, "PushZ") is not None, f"{furnace}")
    tnt_cart = one("minecraft:tnt_minecart")
    check("TNT minecart burns its fuse and keeps its power",
          tnt_cart is not None and 20000 < val(get(tnt_cart, "fuse")) < 30000 and val(get(tnt_cart, "explosion_power")) == 6.0, f"{tnt_cart}")
    loot = one("minecraft:chest_minecart", lambda e: get(e, "LootTable") is not None)
    check("loot table minecart keeps its unrolled table and seed",
          loot is not None and val(get(loot, "LootTable")) == "minecraft:chests/simple_dungeon" and val(get(loot, "LootTableSeed")) == 99, f"{loot}")
    boat = one("minecraft:oak_chest_boat")
    check("chest boat keeps its slots and tags",
          boat is not None and slots(boat) == {2: ("minecraft:diamond", 6), 20: ("minecraft:apple", 3)} and get(boat, "Tags") is not None, f"{boat}")
    raft = one("minecraft:bamboo_chest_raft")
    check("chest raft keeps its unrolled loot table and seed",
          raft is not None and val(get(raft, "LootTable")) == "minecraft:chests/simple_dungeon" and val(get(raft, "LootTableSeed")) == 7, f"{raft}")
    donkey = one("minecraft:donkey")
    check("donkey keeps its chest, slots, saddle and tags",
          donkey is not None and val(get(donkey, "ChestedHorse")) == 1 and slots(donkey) == {0: ("minecraft:emerald", 5), 14: ("minecraft:stick", 9)}
          and val(get(donkey, "equipment", "saddle", "id")) == "minecraft:saddle" and get(donkey, "Tags") is not None, f"{donkey}")
    llama = one("minecraft:llama")
    check("llama keeps its strength, coat, chest, slots, carpet and tags",
          llama is not None and val(get(llama, "Strength")) == 4 and val(get(llama, "Variant")) == 2 and val(get(llama, "ChestedHorse")) == 1
          and slots(llama) == {0: ("minecraft:emerald", 5), 11: ("minecraft:stick", 9)}
          and val(get(llama, "equipment", "body", "id")) == "minecraft:red_carpet" and get(llama, "Tags") is not None, f"{llama}")
    trader = one("minecraft:trader_llama")
    check("trader llama keeps its strength, coat, chest, slots, carpet and despawn delay",
          trader is not None and val(get(trader, "Strength")) == 2 and val(get(trader, "Variant")) == 3 and val(get(trader, "ChestedHorse")) == 1
          and slots(trader) == {3: ("minecraft:gold_ingot", 7)} and val(get(trader, "equipment", "body", "id")) == "minecraft:blue_carpet"
          and val(get(trader, "DespawnDelay")) == 30000, f"{trader}")
    cow = one("minecraft:cow", lambda e: get(e, "Passengers") is not None)
    cow_riders = list(rider_chain(cow)) if cow else []
    check("riding stack: the cow keeps its husk and the husk its chicken, with vanilla's UUIDs and tags",
          cow is not None and [typ(r) for r in cow_riders] == ["minecraft:husk", "minecraft:chicken"]
          and [uuid_of(r) for r in cow_riders] == [uuid_of(r) for r in rider_chain(stack)]
          and [val(get(r, "Tags")) for r in cow_riders] == [val(get(r, "Tags")) for r in rider_chain(stack)], f"{cow}")
    check("riders are not entities of the chunk of their own", len([e for e in saved.values() if typ(e) in ("minecraft:husk", "minecraft:chicken")]) == 0,
          f"{[typ(e) for e in saved.values()]}")
    check("a rider's position is its vehicle's x and z",
          cow is not None and all(val(get(r, "Pos"))[0] == val(get(cow, "Pos"))[0] and val(get(r, "Pos"))[2] == val(get(cow, "Pos"))[2] for r in cow_riders), "")
    snow = one("minecraft:snowball")
    check("snowball kept", snow is not None and val(get(snow, "NoGravity")) == 1, f"{snow}")
    emerald = one("minecraft:item", lambda e: val(get(e, "Item", "id")) == "minecraft:emerald")
    thrower = None
    if emerald is not None and get(emerald, "Thrower") is not None:
        thrower = uuid.UUID(bytes=struct.pack(">iiii", *val(get(emerald, "Thrower"))))
    check("emeralds dropped on Kiln saved with their thrower", emerald is not None and val(get(emerald, "Item", "count")) == 4
          and thrower == offline_uuid(NAME), f"{emerald}")
    root = region_chunk(world / "world", CHUNK)
    check("entity chunk has vanilla's layout (DataVersion 5023, Position)",
          val(get(root, "DataVersion")) == 5023 and val(get(root, "Position")) == CHUNK, "")

    # ---- vanilla on the world Kiln saved ----
    print("== vanilla on the Kiln-saved world", flush=True)
    after = out / "after-world"
    if after.exists():
        shutil.rmtree(after)
    shutil.copytree(world, after)
    s = vanilla(after, a.vanilla_port, out / "vanilla-after.log")
    try:
        cx, cz = CHUNK
        s.query(f"forceload add {cx * 16} {cz * 16}", r"[Mm]arked chunk|already")
        time.sleep(4)
        errors = [l for l in s.lines if re.search(r"Failed to load|Exception|corrupt|entity chunk|Skipping|UUID of added entity", l)]
        check("vanilla loads Kiln's entity chunk without errors", not errors, "; ".join(errors[:3]))
        for u, e in saved.items():
            line = s.query(f"data get entity {u}", r"has the following entity data|No entity was found|Found no elements")
            check(f"vanilla has the {typ(e)} {u}", line and "has the following entity data" in line, (line or "no answer")[-120:])
        for r in cow_riders:
            line = s.query(f"data get entity {uuid_of(r)}", r"has the following entity data|No entity was found|Found no elements")
            check(f"vanilla has the rider {typ(r)} {uuid_of(r)}", line and "has the following entity data" in line, (line or "no answer")[-120:])
        line = s.query('execute as @e[type=chicken,tag=topmost] on vehicle if entity @s[type=husk,tag=pillion] on vehicle if entity @s[type=cow,tag=stack]',
                       r"Test passed|Test failed")
        check("vanilla loads the stack riding: the chicken on the husk on the cow", line and "Test passed" in line, line or "")
        line = s.query(f"data get entity {emerald_uuid(emerald)} Item" if emerald else "list", r"has the following entity data|No entity")
        check("vanilla sees Kiln's emeralds", line and "minecraft:emerald" in line and "count: 4" in line, (line or "")[-120:])
        line = s.query('execute if entity @e[type=pig,name=Porky,tag=kiln]', r"Test passed|Test failed")
        check("vanilla's pig came back with its name and tag", line and "Test passed" in line, line or "")
        line = s.query('execute if entity @e[type=zombie,tag=kiln,nbt={IsBaby:1b,Health:15f}]', r"Test passed|Test failed")
        check("vanilla's zombie came back as a hurt baby", line and "Test passed" in line, line or "")
        line = s.query('execute if entity @e[type=item,tag=kiln,nbt={Age:-32768s}]', r"Test passed|Test failed")
        check("vanilla reads the item's age and tag", line and "Test passed" in line, line or "")
        line = s.query('execute if entity @e[type=llama,tag=kiln,nbt={Strength:4,Variant:2,ChestedHorse:1b,equipment:{body:{id:"minecraft:red_carpet"}}}]', r"Test passed|Test failed")
        check("vanilla's llama came back with its strength, coat, chest and carpet", line and "Test passed" in line, line or "")
        line = s.query('execute if entity @e[type=trader_llama,tag=kiln,nbt={Strength:2,Variant:3,DespawnDelay:30000}]', r"Test passed|Test failed")
        check("vanilla's trader llama came back with its strength, coat and despawn delay", line and "Test passed" in line, line or "")
    finally:
        s.stop()

    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    sys.exit(1 if failed else 0)


def emerald_uuid(e):
    return uuid_of(e)


if __name__ == "__main__":
    main()
