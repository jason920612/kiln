"""wp82 item 3: block entity data and timing, Kiln against vanilla in lock step: place a block entity (frozen game), step ONE tick at a time and read the
interesting fields after every step. usage: be82.py [ticks]"""
from lab import *
N = int(sys.argv[1]) if len(sys.argv) > 1 else 5
setup(frozen=True)
Y = 150
X = 20
CASES = [
    ("brewing_stand", 'minecraft:brewing_stand{Items:[{Slot:0b,id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:water"}}},{Slot:3b,id:"minecraft:nether_wart",count:1},{Slot:4b,id:"minecraft:blaze_powder",count:1}]}', ["BrewTime", "Fuel", "total_brew_time"]),
    ("hopper", 'minecraft:hopper{Items:[{Slot:0b,id:"minecraft:stone",count:3}]}', ["TransferCooldown"]),
    ("hopper_empty", "minecraft:hopper", ["TransferCooldown"]),
    ("furnace", 'minecraft:furnace{Items:[{Slot:0b,id:"minecraft:raw_iron",count:2},{Slot:1b,id:"minecraft:coal",count:1}]}', ["lit_time_remaining", "cooking_time_spent"]),
    ("lectern_nopages", 'minecraft:lectern{Book:{id:"minecraft:writable_book",count:1},Page:2}', ["Page"]),
    ("lectern_pages", 'minecraft:lectern{Book:{id:"minecraft:writable_book",count:1,components:{"minecraft:writable_book_content":{pages:[{raw:"a"},{raw:"b"},{raw:"c"}]}}},Page:5}', ["Page"]),
    ("jukebox", 'minecraft:jukebox{RecordItem:{id:"minecraft:music_disc_13",count:1},ticks_since_song_started:5L,is_playing:1b}', ["ticks_since_song_started", "is_playing", "RecordItem"]),
    ("jukebox_extra", 'minecraft:jukebox{RecordItem:{id:"minecraft:music_disc_13",count:1},Foo:3b}', ["Foo", "ticks_since_song_started"]),
    ("chest_extra", 'minecraft:chest{Foo:3b,Items:[{Slot:0b,id:"minecraft:stone",count:3}]}', ["Foo", "Items"]),
]
def field(text, key):
    m = re.search(r"\b" + key + r": ([^,}\]]*(?:\{[^}]*\})?)", text)
    return m.group(1) if m else None
for name, setblock, keys in CASES:
    OB(f"fill {X - 3} {Y - 3} {X - 3} {X + 3} {Y + 3} {X + 3} minecraft:air")
    OB(f"setblock {X} {Y - 1} {X} minecraft:iron_block")
    time.sleep(0.4)
    OB(f"setblock {X} {Y} {X} {setblock}")
    rows = {"K": [], "V": []}
    for i in range(N):
        step(1)
        for t in "KV":
            txt = nbt(t, "block", f"{X} {Y} {X}")
            rows[t].append(tuple(field(txt, k) for k in keys))
    same = rows["K"] == rows["V"]
    F(f"{name:18s} {'SAME' if same else 'DIFF'} {keys}")
    if not same:
        for i in range(N):
            F(f"    step {i + 1}: K {rows['K'][i]}   V {rows['V'][i]}")
both_("tick unfreeze")
OB(f"fill {X - 3} {Y - 3} {X - 3} {X + 3} {Y + 3} {X + 3} minecraft:air")
