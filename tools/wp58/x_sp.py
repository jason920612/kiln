from p68_ghast import *
setup_two()
sv("tp KilnTest 6.5 200 6.5", show=False); time.sleep(1.5)
F("before", pos(a), a.state(0)["health"])
for i in range(4):
    sprint_ticks(250)
    F("after sprint", i, pos(a), a.state(0)["health"], a.screen())
