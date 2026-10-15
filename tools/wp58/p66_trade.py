"""wp66: the trade offers (prices) of the profession villagers with and without the hero effect. usage: TARGET=K|V [ACTOR..] python3 p66_trade.py OUT.json"""
import sys
from p66_raid import *

if __name__ == "__main__":
    a = client(PORT)
    try:
        S("gamerule advance_time false", "time set 1000", f"effect give {NAME} minecraft:hero_of_the_village 48000 1 true")
        time.sleep(2)
        trade_prices(a)
    finally:
        json.dump(LOG, open(sys.argv[1], "w"), indent=1)
