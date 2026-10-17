from lock import *
for c in ["time query day", "time query time", "time query minecraft:overworld", "time query minecraft:overworld time", "time of day"]:
    r = ask_both(c)
    print(c, "\n  K", r["K"].strip()[-110:].replace("\n"," | "), "\n  V", r["V"].strip()[-110:].replace("\n"," | "))
