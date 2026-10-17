from lock import *
for t in "KV":
    print(t, "items", kill_count(t, "@e[type=item]"), "xp", kill_count(t, "@e[type=experience_orb]"))
