"""wp68 #4: vaults and the ominous trial spawner with TWO real clients on one server (switch.sh K|V both; TARGET=K|V): each player opens a vault
once with a key of their own and gets a reward of their own; a trial omen turns a trial spawner ominous. Compared K vs V."""
from p68 import *
import sys

a = C(25999)
b = C(26000)
scenario("vault2")
ME = "KilnTest"
OBS = "Obs"


def lines(c=a):
    return json.loads(c.cmd("spylog clear"))


def state_of(c, pos):
    return blk(c, *pos)


def vault_round(ominous):
    tag = "ominous" if ominous else "normal"
    key = "minecraft:ominous_trial_key" if ominous else "minecraft:trial_key"
    loot = "reward_ominous" if ominous else "reward"
    V = (20, 200, 18)
    sv("fill 12 200 12 28 204 28 minecraft:air", show=False)
    sv("kill @e[type=item]", show=False)
    sv(f"setblock {V[0]} {V[1]} {V[2]} minecraft:vault[ominous={str(ominous).lower()},facing=south]{{config:{{key_item:{{id:\"{key}\",count:1}},loot_table:\"minecraft:chests/trial_chambers/{loot}\"}}}}", show=False)
    time.sleep(0.5)
    for nm in (ME, OBS):
        sv(f"clear {nm}", show=False)
        give(nm, key, 4)
    sv(f"tp {ME} 19.5 200 21.5", show=False)
    sv(f"tp {OBS} 21.5 200 21.5", show=False)
    time.sleep(2.0)
    F(tag, "vault block:", state_of(a, V))
    results = {}
    for who, c in ((ME, a), (OBS, b), (ME, a), (OBS, b)):
        c.select(key)
        c.lookat(V[0] + 0.5, V[1] + 0.5, V[2] + 0.5)
        time.sleep(0.6)
        before = c.inv().get(key, 0)
        for cc in (a, b):
            cc.cmd("spy on")
            cc.cmd("spylog clear")
        c.mouse("right", "tap")
        t0 = time.time()
        seen = set()
        items = collections.Counter()
        states = []
        while time.time() - t0 < 14:
            time.sleep(0.4)
            st = re.sub(r"vault\[|\]", "", state_of(a, V))
            if not states or states[-1][1] != st:
                states.append((round(time.time() - t0, 1), st))
            for e in a.state(12)["entities"]:
                if e["t"] == "item" and e["id"] not in seen:
                    seen.add(e["id"])
                    items[e.get("item")] += e.get("n", 1)
            if len(states) >= 3 and "inactive" in st and time.time() - t0 > 4:
                break
        after = c.inv().get(key, 0)
        snd = [re.sub(r"vol=\S+|pitch=\S+", "", l)[:70] for l in lines(c) if "sound" in l and "step" not in l and "item.pickup" not in l]
        F(tag, who, "used the key: keys", before, "->", after, "states:", [s[1] for s in states], "items:", sum(items.values()), "kinds", len(items), "sounds:", collections.Counter(snd).most_common(5))
        a.collect(6, 3)
        b.collect(6, 3)
        time.sleep(1.0)
    data = sv(f"data get block {V[0]} {V[1]} {V[2]}", show=False, wait=0.8)
    F(tag, "vault data:", re.findall(r"rewarded_players: \[[^\]]*(?:\]\s*,\s*\[[^\]]*)*\]|state: \"\w+\"|stored_items: \[[^\]]*\]", data)[:4], "rewarded players:", len(re.findall(r"\[I; -?\d+, -?\d+, -?\d+, -?\d+\]", data.split("rewarded_players")[1].split("]]")[0] if "rewarded_players" in data else "")))
    F(tag, "keys left: A", a.inv().get(key, 0), "B", b.inv().get(key, 0), "got (A,B) distinct items:", sorted(k for k in a.inv() if k not in (key, "minecraft:ominous_trial_key", "minecraft:trial_key")), "|", sorted(k for k in b.inv() if k not in (key, "minecraft:ominous_trial_key", "minecraft:trial_key")))


def omen():
    S = (20, 200, 28)
    sv("fill 12 200 12 30 204 36 minecraft:air", show=False)
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs true", show=False)
    sv("difficulty normal", show=False)
    sv(f"setblock {S[0]} {S[1]} {S[2]} minecraft:trial_spawner{{normal_config:\"minecraft:trial_chamber/melee/zombie/normal\",ominous_config:\"minecraft:trial_chamber/melee/zombie/ominous\"}}", show=False)
    for nm in (ME, OBS):
        sv(f"effect clear {nm}", show=False)
        sv(f"effect give {nm} minecraft:resistance 99999 4 true", show=False)
        sv(f"effect give {nm} minecraft:regeneration 99999 2 true", show=False)
    sv(f"tp {ME} 20.5 200 24.5", show=False)
    sv(f"tp {OBS} 22.5 200 24.5", show=False)
    time.sleep(2.0)
    F("spawner at start:", state_of(a, S))
    sv(f"effect give {ME} minecraft:trial_omen 600 0", show=False)
    t0 = time.time()
    states = []
    while time.time() - t0 < 40:
        time.sleep(1.0)
        st = re.sub(r"trial_spawner\[|\]", "", state_of(a, S))
        if not states or states[-1][1] != st:
            states.append((round(time.time() - t0), st))
        sv("kill @e[type=!player,type=!item,type=!experience_orb]", show=False, wait=0.2)
        if any("ominous=true" in s[1] for s in states) and time.time() - t0 > 12:
            break
    effs = lambda nm: re.findall(r"id: \"minecraft:(\w+)\"", sv(f"data get entity {nm} active_effects", show=False, wait=0.5))
    F("spawner states:", states)
    F("effects after: A", effs(ME), "B", effs(OBS))
    F("spawner data:", re.findall(r"required_player_range: \d+|target_cooldown_length: \d+|ominous[^,}]*", sv(f"data get block {S[0]} {S[1]} {S[2]}", show=False, wait=0.8))[:6])
    sv("kill @e[type=!player]", show=False)
    sv("gamerule spawn_mobs false", show=False)


if __name__ == "__main__":
    alive_auto(a)
    alive_b(b)
    setup_two()
    for which in sys.argv[1:] or ["normal", "ominous", "omen"]:
        try:
            if which == "omen":
                omen()
            else:
                vault_round(which == "ominous")
        except Exception as ex:
            import traceback
            traceback.print_exc()
            F(which, "ERROR", repr(ex)[:300])
