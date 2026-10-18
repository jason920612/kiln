"""wp80: right-click every item of a list on every mob of a list (real client, survival, NoAI mobs 2 blocks away), record what changed: the player's inventory, the keys of
the mob's NBT that changed, the sounds / particles / entity events the client got. Run on both servers at once with par.sh; compare with p80_matrix_cmp.py.
usage: par.sh p80_matrix.py [mob,mob...] [item,item...]"""
from p72 import *
import collections
a = C(int(os.environ.get("KILN_AGENT_PORT", "25999")))
ME = "KilnTest"
MOBS = "cow,sheep,pig,chicken,horse,donkey,mule,llama,camel,camel_husk,cat,wolf,fox,panda,polar_bear,rabbit,turtle,axolotl,frog,tadpole,goat,sniffer,armadillo,bee,parrot,ocelot,allay,happy_ghast,villager,wandering_trader,iron_golem,snow_golem,copper_golem,mooshroom,strider,hoglin,piglin,zombie_horse,skeleton_horse,zombie,husk,drowned,skeleton,creeper,spider,slime,magma_cube,enderman,witch,squid,glow_squid,cod,salmon,tropical_fish,pufferfish,dolphin,bat,nautilus,zombie_nautilus,bogged,breeze,creaking,warden,ravager,vindicator,pillager".split(",")
ITEMS = "wheat,carrot,wheat_seeds,bone,golden_apple,cod,rotten_flesh,bucket,water_bucket,shears,name_tag,saddle,lead,glass_bottle,bowl,brush,honeycomb,spider_eye,amethyst_shard,sweet_berries,iron_ingot,slime_ball,apple,dandelion,melon_slice,potato,hay_block,cactus,tropical_fish,string,black_dye,flint_and_steel,nautilus_shell,golden_carrot,glow_berries,copper_ingot,emerald,gold_ingot,pumpkin_pie,raw_beef".split(",")
if len(sys.argv) > 1 and sys.argv[1] != "-":
    MOBS = sys.argv[1].split(",")
if len(sys.argv) > 2:
    ITEMS = sys.argv[2].split(",")
NOISE = {"Pos", "Motion", "Rotation", "UUID", "Air", "FallDistance", "fall_distance", "OnGround", "Brain", "HurtTime", "HurtByTimestamp", "AbsorptionAmount", "PortalCooldown", "Age", "ForcedAge",
         "InLove", "DeathTime", "TicksFrozen", "ticks_frozen", "Fire", "attributes", "SleepingPos", "current_impulse_context_reset_grace_time", "Invulnerable", "Tags", "NoAI"}


def split_top(s):
    s = s.strip()
    if s.startswith("{"):
        s = s[1:-1]
    out, depth, cur, q = {}, 0, "", None
    parts = []
    for ch in s:
        if q:
            cur += ch
            if ch == q:
                q = None
            continue
        if ch in "\"'":
            q = ch
            cur += ch
        elif ch in "{[(":
            depth += 1
            cur += ch
        elif ch in "}])":
            depth -= 1
            cur += ch
        elif ch == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        parts.append(cur)
    for p in parts:
        if ":" in p:
            k, v = p.split(":", 1)
            out[k.strip()] = v.strip()
    return out


def snap():
    return split_top(nbt_of("@e[tag=m,limit=1]", "", 0.5))


out_path = HOME / "wp80" / f"matrix_{T}.jsonl"
if T == "V":
    sv("op KilnTest", show=False)
alive_auto(a)
sv("gamerule spawn_mobs false", show=False); sv("gamerule mob_griefing false", show=False); sv("difficulty normal", show=False)
sv("time set day", show=False); sv("weather clear", show=False)
platform(0, 0, 40, 40, y=199, clear=12)
out = open(out_path, "a")
for mob in MOBS:
    for item in ITEMS:
        kill_all()
        kit(ME)
        sv(f"effect give {ME} minecraft:resistance 99999 4 true", show=False)
        sv(f"tp {ME} 20.5 200 20.5", show=False)
        extra = ""
        if mob == "creeper":
            extra = ",ExplosionRadius:0b"
        sv(f"summon minecraft:{mob} 20.5 200 23.5 {{NoAI:1b,PersistenceRequired:1b,Silent:1b,Tags:[\"m\"]{extra}}}", show=False)
        time.sleep(0.7)
        ents = [e for e in a.state(12)["entities"] if e["t"] == mob]
        if not ents:
            out.write(json.dumps({"mob": mob, "item": item, "err": "not seen"}) + "\n"); out.flush()
            continue
        eid = ents[0]["id"]
        if item in ("string", "lead") and mob in ("horse",):
            pass
        sv(f"give {ME} minecraft:{item} 2", show=False, wait=0.3)
        if item == "saddle" and mob in ("horse", "donkey", "mule", "camel", "camel_husk", "strider", "pig", "llama", "zombie_horse", "skeleton_horse", "nautilus", "zombie_nautilus"):
            pass
        before = snap()
        inv0 = a.inv()
        rec_start(a)
        ok = use_on_entity(a, eid, f"minecraft:{item}")
        time.sleep(0.6)
        lines = json.loads(a.cmd("spylog clear"))
        after = snap() if (ents := [e for e in a.state(12)["entities"] if e["t"] == mob]) or True else {}
        inv1 = a.inv()
        delta = {k: inv1.get(k, 0) - inv0.get(k, 0) for k in set(inv0) | set(inv1) if inv1.get(k, 0) != inv0.get(k, 0)}
        changed = sorted(k for k in set(before) | set(after) if k not in NOISE and before.get(k) != after.get(k))
        kinds = collections.Counter()
        for l in lines:
            c = l.split(" ")[0]
            if c == "sound":
                kinds["sound " + l.split(" ")[1]] += 1
            elif c == "particle":
                kinds["particle " + l.split(" ")[1]] += 1
            elif c in ("entity_event", "SetEquipment", "SetPassengers", "EntityLink", "UpdateMobEffect", "TakeItemEntity", "RemoveEntities", "AddEntity", "OpenScreen", "ContainerSetSlot"):
                kinds[c + (" " + " ".join(l.split(" ")[1:3]) if c == "entity_event" else "")] += 1
        rec = {"mob": mob, "item": item, "ok": ok, "inv": delta, "changed": changed, "fx": sorted(kinds), "after": {k: after.get(k, "")[:60] for k in changed}}
        out.write(json.dumps(rec) + "\n"); out.flush()
        if a.screen() not in (None, "null", ""):
            try:
                a.close()
            except Exception:
                pass
    print(mob, "done", flush=True)
kill_all()
