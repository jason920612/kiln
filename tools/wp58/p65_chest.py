"""wp65: open the fortress chests through the real client and list what the generated loot is (same seed on both servers -> same contents)."""
from p65_b2 import *
prep()
for nm in ("KilnTest", "Obs"):
    sv(f"execute in minecraft:the_nether run tp {nm} 278.5 80 195.5", show=False)
time.sleep(8)
for pos in [(274, 57, 218), (267, 57, 220), (275, 64, 198), (282, 72, 190), (232, 50, 210)]:
    spot = free_spot(a, pos[0], pos[1], pos[2], r=3, dy=2)
    if not spot:
        print(pos, "no spot"); continue
    sv(f"execute in minecraft:the_nether run tp KilnTest {spot[1]+0.5} {spot[2]} {spot[3]+0.5}", show=False)
    time.sleep(3.5)
    a.close()
    a.use_on(*pos)
    time.sleep(1.2)
    st = a.state(0)
    items = [(s["i"], s["id"].replace("minecraft:", ""), s["n"]) for s in st.get("slots", []) if s["i"] < 27 and s["n"] > 0]
    print("chest", pos, a.screen(), items, flush=True)
    a.close()
