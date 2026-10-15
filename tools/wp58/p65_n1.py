"""wp65 nether 1: both players go through the portal and A walks (digs) towards the fortress while the server's mspt / chunk counters are logged."""
from p65_b2 import *

def n1():
    prep()
    sv("tp KilnTest 805 200 805", show=False); sv("tp Obs 806 200 805", show=False)
    sv("give KilnTest minecraft:diamond_pickaxe[minecraft:enchantments={'minecraft:efficiency':5}] 1", show=False)
    sv("give Obs minecraft:diamond_pickaxe[minecraft:enchantments={'minecraft:efficiency':5}] 1", show=False)
    time.sleep(1)
    sv("tp KilnTest 808.5 200 808.5", show=False); sv("tp Obs 809.5 200 808.5", show=False)
    ta = wait_dim(a, "the_nether", 40); tb = wait_dim(b, "the_nether", 40)
    F("arrived", ta, tb, pos(a), pos(b))
    time.sleep(3)
    shot("nether_portal_arrive")
    t0 = time.time(); ch0 = chunks()
    log = []
    ok = go(a, 272, 240, timeout=240, log=log)
    F("A reached the fortress area:", ok, pos(a), "in", round(time.time() - t0), "s", log)
    F("mspt", mspt()); F("chunks", chunks())
    shot("nether_walk")

if __name__ == "__main__":
    n1()
