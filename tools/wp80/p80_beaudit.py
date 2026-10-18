"""wp80: `data get block` of block entities whose state changes while they tick, Kiln against vanilla in lock step (both frozen, stepped N ticks): does the command see the live
state? (Found with a beacon whose Levels read 0.) Prints SAME / DIFF with the two NBT texts."""
from lab80 import *
setup(frozen=False, rules=("command_block_output false", "spawn_mobs false"))
both_("difficulty normal")
revive()
Y = 150
CASES = [
    ("brewing_stand", 'minecraft:brewing_stand{Items:[{Slot:0b,id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:water"}}},{Slot:3b,id:"minecraft:nether_wart",count:1},{Slot:4b,id:"minecraft:blaze_powder",count:1}]}', 30),
    ("furnace", 'minecraft:furnace{Items:[{Slot:0b,id:"minecraft:raw_iron",count:2},{Slot:1b,id:"minecraft:coal",count:1}]}', 30),
    ("blast_furnace", 'minecraft:blast_furnace{Items:[{Slot:0b,id:"minecraft:raw_iron",count:2},{Slot:1b,id:"minecraft:coal",count:1}]}', 30),
    ("smoker", 'minecraft:smoker{Items:[{Slot:0b,id:"minecraft:beef",count:2},{Slot:1b,id:"minecraft:coal",count:1}]}', 30),
    ("campfire", 'minecraft:campfire{Items:[{Slot:0b,id:"minecraft:beef",count:1}]}', 30),
    ("hopper", 'minecraft:hopper{Items:[{Slot:0b,id:"minecraft:stone",count:3}]}', 30),
    ("mob_spawner", 'minecraft:spawner{SpawnData:{entity:{id:"minecraft:pig"}},RequiredPlayerRange:0s,MaxNearbyEntities:0s}', 30),
    ("repeating_command_block", 'minecraft:repeating_command_block{auto:1b,Command:"say hi"}', 30),
    ("conduit", "minecraft:conduit", 100),
    ("lectern", 'minecraft:lectern{Book:{id:"minecraft:writable_book",count:1},Page:2}', 10),
    ("daylight_detector", "minecraft:daylight_detector", 30),
    ("sculk_sensor", "minecraft:sculk_sensor", 30),
    ("end_gateway", "minecraft:end_gateway{Age:5L}", 30),
    ("trial_spawner", "minecraft:trial_spawner", 30),
    ("vault", "minecraft:vault", 30),
    ("crafter", "minecraft:crafter", 30),
    ("beehive", "minecraft:beehive", 30),
    ("bell", "minecraft:bell", 30),
    ("jukebox", 'minecraft:jukebox{RecordItem:{id:"minecraft:music_disc_13",count:1},ticks_since_song_started:5L,is_playing:1b}', 30),
    ("chest", 'minecraft:chest{Items:[{Slot:0b,id:"minecraft:stone",count:3}]}', 30),
    ("decorated_pot", "minecraft:decorated_pot", 10),
    ("beacon", "minecraft:beacon", 100),
]
both_("tick freeze")
X = 20
for name, setblock, ticks in CASES:
    OB(f"fill {X - 6} {Y - 6} {X - 6} {X + 6} {Y + 6} {X + 6} minecraft:air")
    OB(f"setblock {X} {Y - 1} {X} minecraft:iron_block")
    if name == "beacon":
        OB(f"fill {X - 1} {Y - 1} {X - 1} {X + 1} {Y - 1} {X + 1} minecraft:iron_block")
    OB(f"setblock {X} {Y} {X} {setblock}")
    step(ticks)
    k = nbt("K", "block", f"{X} {Y} {X}")
    v = nbt("V", "block", f"{X} {Y} {X}")
    norm = lambda t: re.sub(r"(UUID|Owner|Target|spawn_potentials|SpawnPotentials|LastOutput|last_spawn|next_mob_spawns_at|cooldown_ends_at|spawn_data|Delay|SpawnData)\b[^,}]*", r"\1", re.sub(r"^\d+, \d+, \d+, ", "", t))
    # compare as sets of "key: value" top-level pieces
    ks, vs = set(re.findall(r"(\w+): ([^,{}\[\]]*)", norm(k))), set(re.findall(r"(\w+): ([^,{}\[\]]*)", norm(v)))
    same = ks == vs
    print(f"{name:24s} {'SAME' if same else 'DIFF'}" + ("" if same else f"  onlyK {sorted(ks - vs)[:8]} onlyV {sorted(vs - ks)[:8]}"), flush=True)
both_("tick unfreeze")
OB(f"fill {X - 6} {Y - 6} {X - 6} {X + 6} {Y + 6} {X + 6} minecraft:air")
