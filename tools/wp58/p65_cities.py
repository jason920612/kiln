from p65_b2 import *
END = "minecraft:the_end"
found = {}
for ox in (-3000, -1500, 1500, 3000):
    for oz in (-3000, -1500, 1500, 3000):
        r = sv(f"execute in {END} positioned {ox} 64 {oz} run locate structure minecraft:end_city", wait=4.0, show=False)
        m = re.search(r"\[(-?\d+), ~, (-?\d+)\]", r)
        if m:
            found[(int(m.group(1)), int(m.group(2)))] = 1
print(sorted(found), flush=True)
