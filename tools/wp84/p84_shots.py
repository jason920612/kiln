"""wp84: a scene for the screenshot (par.sh): a harnessed happy ghast, a ghastling and a decorated pot beside the player, dusk. The K run takes the picture of the Xvfb display (both windows)."""
from p72 import *
import ctl
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
if T == "V": sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("gamerule do_daylight_cycle false", show=False); sv("time set 12600", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=14)
kill_all(); kit(ME)
sv(f"gamemode creative {ME}", show=False)
sv("setblock 18 200 24 minecraft:decorated_pot[facing=south]", show=False)
sv('data merge block 18 200 24 {item:{id:"minecraft:amethyst_shard",count:20}}', show=False)
sv('summon minecraft:happy_ghast 24.5 203 27.5 {NoAI:1b,PersistenceRequired:1b,Rotation:[200f,0f],equipment:{body:{id:"minecraft:orange_harness",count:1}}}', show=False)
sv('summon minecraft:happy_ghast 19.5 201.5 25.5 {NoAI:1b,PersistenceRequired:1b,Age:-24000}', show=False)
sv(f"tp {ME} 20.5 200 19.5", show=False)
time.sleep(4)
a.look(0, -8)
time.sleep(4)
if T == "K":
    print(ctl.shot("scene_ghast"))
time.sleep(3)
kill_all()
