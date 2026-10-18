from lock import *
for p in [(91,75,-30),(-96,75,-1),(-30,75,91)]:
    for t in "KV":
        print(t, p, ask(t, f"execute in minecraft:the_end if block {p[0]} {p[1]} {p[2]} minecraft:end_gateway", wait=0.8).strip()[-60:])
