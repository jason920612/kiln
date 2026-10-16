from p68_ghast import *
F("server ghasts:", sv("execute if entity @e[type=happy_ghast]", show=False)[-80:])
F(server_ghast()[-700:])
F([ (e["t"], round(e["x"],1), round(e["y"],1), round(e["z"],1)) for e in a.state(40)["entities"]])
F(pos(a), pos(b))
