"""Writes the wp85 test packs under tools/wp85/packs (own small JSON and one own structure template; nothing here
is a copy of a Mojang file). Run: python3 tools/wp85/make_packs.py

  kiln85_desert     replaces the vanilla desert biome (own features, spawns, no carvers) and adds a feature
  kiln85_crystal    a new biome in the overworld's biome list (the dimension's own multi_noise list)
  kiln85_hills      own noise settings (sea level 40, own terrain) for the overworld
  kiln85_structure  a new structure: a template, a pool, a processor list, a structure set and a biome tag
  kiln85_dims       a dimension type of its own and two new dimensions (superflat, nether noise + checkerboard)
  kiln85_nether     replaces the nether's dimension type (shorter, brighter, beds work)
"""
import gzip
import json
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent / "packs"
FORMAT = 121


def write(pack, path, data):
    p = ROOT / pack / path
    p.parent.mkdir(parents=True, exist_ok=True)
    if isinstance(data, (bytes, bytearray)):
        p.write_bytes(data)
    else:
        p.write_bytes((json.dumps(data, indent=2) + chr(10)).encode("utf-8"))


def mcmeta(pack, text):
    write(pack, "pack.mcmeta", {"pack": {"description": text, "min_format": FORMAT, "max_format": FORMAT}})


def simple_block(pack, name, block, count, step_note=""):
    write(pack, f"data/kiln85/worldgen/feature/{name}.json", {
        "type": "minecraft:simple_block",
        "to_place": {"type": "minecraft:simple", "state": block},
    })
    write(pack, f"data/kiln85/worldgen/placed_feature/{name}.json", {
        "feature": f"kiln85:{name}",
        "placement": [
            {"type": "minecraft:count", "count": count},
            {"type": "minecraft:in_square"},
            {"type": "minecraft:heightmap", "heightmap": "WORLD_SURFACE_WG"},
            {"type": "minecraft:biome"},
        ],
    })


def spawns(**by_category):
    return {
        "minecraft:gameplay/natural_mob_spawns": {
            "argument": {"spawn_costs": {}, "spawns_by_category": by_category},
            "modifier": "overlay",
        }
    }


def entry(kind, weight, lo, hi=None):
    count = lo if hi is None else {"type": "minecraft:uniform", "min_inclusive": lo, "max_inclusive": hi}
    return {"type": kind, "count": count, "weight": weight}


def steps(**at):
    """11 decoration steps with the placed features given by step index."""
    out = [[] for _ in range(11)]
    for k, v in at.items():
        out[int(k[1:])] = v
    return out


# ---- kiln85_desert -------------------------------------------------------------------------------------------------
def desert():
    pack = "kiln85_desert"
    mcmeta(pack, "wp85: replaces the desert biome")
    simple_block(pack, "gold_patch", "minecraft:gold_block", 3)
    write(pack, "data/minecraft/worldgen/biome/desert.json", {
        "attributes": {**spawns(creature=[entry("minecraft:cow", 8, 2, 3)], monster=[entry("minecraft:zombie", 20, 2, 3)])},
        "carvers": [],
        "downfall": 0.0,
        "effects": {"water_color": "#3f76e4"},
        "features": steps(s9=["kiln85:gold_patch"]),
        "has_precipitation": False,
        "temperature": 2.0,
    })


# ---- kiln85_crystal ------------------------------------------------------------------------------------------------
def crystal():
    pack = "kiln85_crystal"
    mcmeta(pack, "wp85: a new biome in the overworld's list")
    simple_block(pack, "crystal_patch", "minecraft:amethyst_block", 6)
    write(pack, "data/kiln85/worldgen/biome/crystal_flats.json", {
        "attributes": {**spawns(creature=[entry("minecraft:sheep", 12, 2, 4)], monster=[entry("minecraft:spider", 50, 1, 2)])},
        "carvers": ["minecraft:cave"],
        "downfall": 0.8,
        "effects": {"water_color": "#4aa6d8"},
        "features": steps(s9=["kiln85:crystal_patch"]),
        "has_precipitation": True,
        "temperature": 0.4,
    })

    def space(biome, c, t):
        full = [-2.0, 2.0]
        return {
            "biome": biome,
            "parameters": {
                "temperature": t, "humidity": full, "continentalness": c, "erosion": full, "depth": full, "weirdness": full, "offset": 0.0,
            },
        }

    write(pack, "data/minecraft/dimension/overworld.json", {
        "type": "minecraft:overworld",
        "generator": {
            "type": "minecraft:noise",
            "settings": "minecraft:overworld",
            "biome_source": {
                "type": "minecraft:multi_noise",
                "biomes": [
                    space("minecraft:ocean", [-2.0, -0.25], [-2.0, 2.0]),
                    space("kiln85:crystal_flats", [-0.25, 2.0], [-2.0, 0.0]),
                    space("minecraft:plains", [-0.25, 2.0], [0.0, 0.5]),
                    space("minecraft:desert", [-0.25, 2.0], [0.5, 2.0]),
                ],
            },
        },
    })


# ---- kiln85_hills --------------------------------------------------------------------------------------------------
def hills():
    pack = "kiln85_hills"
    mcmeta(pack, "wp85: own noise settings")
    density = {
        "type": "minecraft:add",
        "left": {
            "type": "minecraft:squeeze",
            "input": {
                "type": "minecraft:interpolated", "cell_size_xz": 4, "cell_size_y": 8,
                "input": {
                    "type": "minecraft:add",
                    "left": {"type": "minecraft:gradient", "axis": "y", "from_coordinate": -64, "to_coordinate": 200, "from_value": 1.5, "to_value": -1.5},
                    "right": {
                        "type": "minecraft:mul",
                        "left": {"type": "minecraft:noise", "noise": "minecraft:jagged", "xz_scale": 0.5, "y_scale": 0.0},
                        "right": 0.4,
                    },
                },
            },
        },
        "right": {"type": "minecraft:beardifier"},
    }
    write(pack, "data/kiln85/worldgen/noise_settings/hills.json", {
        "default_block": "minecraft:stone",
        "default_fluid": {"id": "minecraft:water", "properties": {"level": "0"}},
        "disable_mob_generation": False,
        "legacy_random_source": False,
        "material_rule": "minecraft:overworld",
        "noise": {"min_y": -64, "height": 384},
        "noise_router": {
            "temperature": "minecraft:overworld/temperature",
            "vegetation": "minecraft:overworld/vegetation",
            "continents": "minecraft:overworld/continents",
            "erosion": "minecraft:overworld/erosion",
            "depth": "minecraft:overworld/depth",
            "ridges": "minecraft:overworld/ridges",
            "chunk_surface_level": 0.0,
            "final_density": density,
        },
        "sea_level": 40,
        "spawn_target": [],
    })
    write(pack, "data/minecraft/dimension/overworld.json", {
        "type": "minecraft:overworld",
        "generator": {
            "type": "minecraft:noise",
            "settings": "kiln85:hills",
            "biome_source": {"type": "minecraft:multi_noise", "preset": "minecraft:overworld"},
        },
    })


# ---- kiln85_structure ----------------------------------------------------------------------------------------------
def nbt_string(s):
    b = s.encode("utf-8")
    return struct.pack(">H", len(b)) + b


def hut_template():
    """A 5x4x5 cobblestone hut (planks floor and roof, an open doorway, a chest) as a structure template file."""
    palette = ["minecraft:cobblestone", "minecraft:oak_planks", "minecraft:chest"]
    blocks = []
    for x in range(5):
        for z in range(5):
            blocks.append((x, 0, z, 1))
            blocks.append((x, 3, z, 1))
            if x in (0, 4) or z in (0, 4):
                for y in (1, 2):
                    if not (x == 2 and z == 0):  # the doorway
                        blocks.append((x, y, z, 0))
    blocks.append((3, 1, 3, 2))

    def tag(kind, name, payload):
        return bytes([kind]) + nbt_string(name) + payload

    def int_(v):
        return struct.pack(">i", v)

    def list_(kind, items):
        return bytes([kind]) + struct.pack(">i", len(items)) + b"".join(items)

    def compound(parts):
        return b"".join(parts) + b"\x00"

    pal = list_(10, [compound([tag(8, "id", nbt_string(n))]) for n in palette])
    blks = []
    for x, y, z, s in blocks:
        parts = [tag(9, "pos", list_(3, [int_(x), int_(y), int_(z)])), tag(3, "state", int_(s))]
        if s == 2:
            parts.append(tag(10, "nbt", compound([tag(8, "id", nbt_string("minecraft:chest")), tag(8, "LootTable", nbt_string("minecraft:chests/simple_dungeon"))])))
        blks.append(compound(parts))
    root = compound([
        tag(3, "DataVersion", int_(5023)),
        tag(9, "size", list_(3, [int_(5), int_(4), int_(5)])),
        tag(9, "palette", pal),
        tag(9, "blocks", list_(10, blks)),
        tag(9, "entities", list_(10, [])),
    ])
    return gzip.compress(bytes([10]) + nbt_string("") + root, mtime=0)


def structure():
    pack = "kiln85_structure"
    mcmeta(pack, "wp85: a new structure")
    write(pack, "data/kiln85/structure/hut.nbt", hut_template())
    write(pack, "data/kiln85/worldgen/processor_list/mossy.json", {
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {"predicate_type": "minecraft:random_block_match", "block": "minecraft:cobblestone", "probability": 0.5},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": "minecraft:mossy_cobblestone",
            }],
        }]
    })
    write(pack, "data/kiln85/worldgen/template_pool/hut/start.json", {
        "elements": [{
            "element": {
                "element_type": "minecraft:single_pool_element",
                "location": "kiln85:hut",
                "processors": "kiln85:mossy",
                "projection": "rigid",
            },
            "weight": 1,
        }],
        "fallback": "minecraft:empty",
    })
    write(pack, "data/kiln85/worldgen/structure/hut.json", {
        "type": "minecraft:jigsaw",
        "biomes": "#kiln85:has_structure/hut",
        "max_distance_from_center": 30,
        "project_start_to_heightmap": "WORLD_SURFACE_WG",
        "size": 1,
        "spawn_overrides": {},
        "start_height": {"absolute": 0},
        "start_pool": "kiln85:hut/start",
        "step": "surface_structures",
        "terrain_adaptation": "beard_thin",
        "use_expansion_hack": False,
    })
    write(pack, "data/kiln85/worldgen/structure_set/huts.json", {
        "placement": {"type": "minecraft:random_spread", "salt": 85085, "separation": 2, "spacing": 6},
        "structures": [{"structure": "kiln85:hut", "weight": 1}],
    })
    write(pack, "data/kiln85/tags/worldgen/biome/has_structure/hut.json", {
        "values": ["minecraft:plains", "minecraft:desert", "minecraft:forest", "minecraft:meadow"],
    })


# ---- kiln85_dims ---------------------------------------------------------------------------------------------------
def dims():
    pack = "kiln85_dims"
    mcmeta(pack, "wp85: dimension types and dimensions")
    write(pack, "data/kiln85/dimension_type/flat_type.json", {
        "ambient_light": 0.1,
        "attributes": {
            "minecraft:gameplay/bed_rule": {"can_set_spawn": "always", "can_sleep": "always"},
            "minecraft:gameplay/respawn_anchor_works": True,
        },
        "coordinate_scale": 2.0,
        "default_clock": "minecraft:overworld",
        "has_ceiling": False,
        "has_ender_dragon_fight": False,
        "has_skylight": True,
        "height": 64,
        "infiniburn": "#minecraft:infiniburn_overworld",
        "logical_height": 64,
        "min_y": -16,
        "monster_spawn_block_light_limit": 0,
        "monster_spawn_light_level": 7,
        "timelines": "#minecraft:in_overworld",
    })
    write(pack, "data/kiln85/dimension_type/cavern_type.json", {
        "ambient_light": 0.1,
        "attributes": {
            "minecraft:gameplay/bed_rule": {"can_set_spawn": "never", "can_sleep": "never", "destroy_on_use": True},
            "minecraft:gameplay/can_start_raid": False,
            "minecraft:gameplay/fast_lava": True,
            "minecraft:gameplay/piglins_zombify": False,
            "minecraft:gameplay/respawn_anchor_works": True,
            "minecraft:gameplay/water_evaporates": True,
        },
        "coordinate_scale": 1.0,
        "has_ceiling": True,
        "has_ender_dragon_fight": False,
        "has_fixed_time": True,
        "has_skylight": False,
        "height": 256,
        "infiniburn": "#minecraft:infiniburn_nether",
        "logical_height": 128,
        "min_y": 0,
        "monster_spawn_block_light_limit": 15,
        "monster_spawn_light_level": 7,
        "timelines": "#minecraft:in_nether",
    })
    write(pack, "data/kiln85/dimension/flatland.json", {
        "type": "kiln85:flat_type",
        "generator": {
            "type": "minecraft:flat",
            "settings": {
                "biome": "minecraft:plains",
                "features": False,
                "lakes": False,
                "structure_overrides": [],
                "layers": [
                    {"block": "minecraft:bedrock", "height": 1},
                    {"block": "minecraft:stone", "height": 2},
                    {"block": "minecraft:water", "height": 1},
                    {"block": "minecraft:dirt", "height": 2},
                    {"block": "minecraft:grass_block", "height": 1},
                ],
            },
        },
    })
    write(pack, "data/kiln85/dimension/cavern.json", {
        "type": "kiln85:cavern_type",
        "generator": {
            "type": "minecraft:noise",
            "settings": "minecraft:nether",
            "biome_source": {
                "type": "minecraft:checkerboard",
                "scale": 2,
                "biomes": ["minecraft:crimson_forest", "minecraft:warped_forest", "minecraft:basalt_deltas", "minecraft:nether_wastes"],
            },
        },
    })


# ---- kiln85_nether -------------------------------------------------------------------------------------------------
def nether():
    pack = "kiln85_nether"
    mcmeta(pack, "wp85: replaces the nether's dimension type")
    write(pack, "data/minecraft/dimension_type/the_nether.json", {
        "ambient_light": 0.5,
        "attributes": {
            "minecraft:gameplay/bed_rule": {"can_set_spawn": "always", "can_sleep": "always"},
            "minecraft:gameplay/fast_lava": True,
            "minecraft:gameplay/piglins_zombify": False,
            "minecraft:gameplay/respawn_anchor_works": False,
            "minecraft:gameplay/water_evaporates": False,
        },
        "coordinate_scale": 4.0,
        "has_ceiling": True,
        "has_ender_dragon_fight": False,
        "has_fixed_time": True,
        "has_skylight": False,
        "height": 192,
        "infiniburn": "#minecraft:infiniburn_nether",
        "logical_height": 128,
        "min_y": 16,
        "monster_spawn_block_light_limit": 15,
        "monster_spawn_light_level": 7,
        "timelines": "#minecraft:in_nether",
    })


if __name__ == "__main__":
    for f in (desert, crystal, hills, structure, dims, nether):
        f()
    print("wrote", ROOT)
