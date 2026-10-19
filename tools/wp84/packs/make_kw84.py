"""Writes the wp84 test data pack `kw84` (our own small JSON, nothing copied from Mojang's files) next to this script: python make_kw84.py"""
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent / "kw84"

FILES = {}


def add(path, text):
    FILES[path] = text if isinstance(text, str) else json.dumps(text, indent=1)


add("pack.mcmeta", {"pack": {"description": "wp84 data pack: functions, macros, advancements, predicates, loot and recipe overrides, enchantments, tags", "min_format": 121, "max_format": 121}})
add("data/minecraft/tags/function/load.json", {"values": ["kw84:load"]})
add("data/minecraft/tags/function/tick.json", {"values": ["kw84:tick"]})
add("data/kw84/function/load.mcfunction", """scoreboard objectives add kw84 dummy
scoreboard players add #loads kw84 1
scoreboard objectives setdisplay sidebar kw84
""")
add("data/kw84/function/tick.mcfunction", """scoreboard players add #ticks kw84 1
execute as @a[predicate=kw84:holding_gold] at @s run particle minecraft:happy_villager ~ ~1 ~ 0.3 0.3 0.3 0 2
execute as @a[predicate=kw84:holding_gold] run scoreboard players add @s kw84 1
execute if score #ticks kw84 matches 100.. run function kw84:cycle
""")
add("data/kw84/function/cycle.mcfunction", """scoreboard players set #ticks kw84 0
scoreboard players add #cycles kw84 1
execute as @a at @s if predicate kw84:in_rain run effect give @s minecraft:speed 3 0 true
execute as @a at @s if predicate kw84:night run title @s actionbar {"text":"kw84 night cycle"}
""")
add("data/kw84/function/macro.mcfunction", """$give $(who) $(item) $(amount)
$tellraw $(who) {"text":"macro gave $(amount) x $(item)","color":"green"}
$scoreboard players set #macro kw84 $(amount)
""")
add("data/kw84/function/reward.mcfunction", """scoreboard players add #rewards kw84 1
tellraw @s {"text":"kw84 reward function ran","color":"gold"}
""")
add("data/kw84/function/nested.mcfunction", """function kw84:nested_inner {depth:1}
return 5
""")
add("data/kw84/function/nested_inner.mcfunction", """$scoreboard players set #inner kw84 $(depth)
return run scoreboard players get #inner kw84
""")
add("data/kw84/predicate/holding_gold.json", {"type": "minecraft:entity_properties", "entity": "this", "predicate": {"equipment": {"mainhand": {"items": "minecraft:gold_ingot"}}}})
add("data/kw84/predicate/night.json", {"type": "minecraft:time_check", "clock": "minecraft:overworld", "value": {"min": 13000, "max": 23000}, "period": 24000})
add("data/kw84/predicate/in_rain.json", {"type": "minecraft:weather_check", "raining": True})
add("data/kw84/advancement/root.json", {"display": {"icon": {"id": "minecraft:gold_ingot"}, "title": {"text": "kw84"}, "description": {"text": "wp84 pack"}, "background": "minecraft:gui/advancements/backgrounds/stone", "show_toast": False, "announce_to_chat": False}, "criteria": {"tick": {"trigger": "minecraft:tick"}}})
add("data/kw84/advancement/nuggets.json", {"parent": "kw84:root", "display": {"icon": {"id": "minecraft:gold_nugget"}, "title": {"text": "Nugget hoard"}, "description": {"text": "Hold three gold nuggets"}, "frame": "goal", "show_toast": True, "announce_to_chat": True},
                                           "criteria": {"three": {"trigger": "minecraft:inventory_changed", "conditions": {"items": [{"items": "minecraft:gold_nugget", "count": {"min": 3}}]}}},
                                           "rewards": {"experience": 37, "loot": ["kw84:gift"], "recipes": ["kw84:dirt_diamond"], "function": "kw84:reward"}})
add("data/kw84/advancement/bread.json", {"parent": "kw84:root", "display": {"icon": {"id": "minecraft:bread"}, "title": {"text": "Bread eater"}, "description": {"text": "Eat bread"}, "frame": "challenge", "show_toast": True, "announce_to_chat": True, "hidden": True},
                                         "criteria": {"ate": {"trigger": "minecraft:consume_item", "conditions": {"item": {"items": "minecraft:bread"}}}}, "rewards": {"experience": 5}})
add("data/kw84/advancement/pig_kill.json", {"parent": "kw84:nuggets", "display": {"icon": {"id": "minecraft:porkchop"}, "title": {"text": "Pig slayer"}, "description": {"text": "Kill a pig holding gold"}, "show_toast": True, "announce_to_chat": True},
                                            "criteria": {"kill": {"trigger": "minecraft:player_killed_entity", "conditions": {"player": "kw84:holding_gold",
                                                                                                                             "entity": {"type": "minecraft:entity_properties", "entity": "this", "predicate": {"minecraft:entity_type": "minecraft:pig"}}}}},
                                            "rewards": {"experience": 11}})
add("data/kw84/advancement/desert.json", {"parent": "kw84:root", "display": {"icon": {"id": "minecraft:sand"}, "title": {"text": "Dry feet"}, "description": {"text": "Stand in a desert"}, "show_toast": True, "announce_to_chat": True},
                                          "criteria": {"here": {"trigger": "minecraft:location", "conditions": {"player": {"type": "minecraft:location_check", "predicate": {"biomes": "minecraft:desert"}}}}}})
add("data/kw84/advancement/either.json", {"parent": "kw84:root", "display": {"icon": {"id": "minecraft:diamond"}, "title": {"text": "Either or"}, "description": {"text": "Hold a stick or a bone"}, "show_toast": True, "announce_to_chat": True},
                                          "criteria": {"a": {"trigger": "minecraft:inventory_changed", "conditions": {"items": [{"items": "minecraft:stick"}]}}, "b": {"trigger": "minecraft:inventory_changed", "conditions": {"items": [{"items": "minecraft:bone"}]}}},
                                          "requirements": [["a", "b"]], "rewards": {"experience": 2}})
add("data/kw84/loot_table/gift.json", {"type": "minecraft:advancement_reward", "pools": [{"rolls": 2, "entries": [
    {"type": "minecraft:item", "name": "minecraft:emerald", "weight": 3, "modifier": {"type": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": 2, "max": 4}}},
    {"type": "minecraft:item", "name": "minecraft:amethyst_shard", "weight": 1}]}]})
add("data/minecraft/loot_table/entities/pig.json", {"type": "minecraft:entity", "pools": [
    {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:emerald", "modifier": {"type": "minecraft:set_count", "count": 2}}], "conditions": [{"type": "minecraft:killed_by_player"}]},
    {"rolls": 1, "entries": [{"type": "minecraft:item", "name": "minecraft:porkchop", "modifier": {"type": "minecraft:set_count", "count": {"type": "minecraft:uniform", "min": 3, "max": 5}}}]}], "random_sequence": "minecraft:entities/pig"})
add("data/minecraft/loot_table/blocks/dirt.json", {"type": "minecraft:block", "pools": [{"rolls": 1, "entries": [{"type": "minecraft:alternatives", "children": [
    {"type": "minecraft:item", "name": "minecraft:dirt", "conditions": [{"type": "minecraft:match_tool", "predicate": {"predicates": {"minecraft:enchantments": [{"enchantments": "minecraft:silk_touch", "levels": {"min": 1}}]}}}]},
    {"type": "minecraft:item", "name": "minecraft:flint", "modifier": {"type": "minecraft:set_count", "count": 2}}]}]}], "random_sequence": "minecraft:blocks/dirt"})
add("data/minecraft/recipe/stick.json", {"type": "minecraft:crafting_shaped", "group": "sticks", "key": {"#": "#minecraft:planks"}, "pattern": ["#", "#"], "result": {"count": 7, "id": "minecraft:stick"}})
add("data/kw84/recipe/dirt_diamond.json", {"type": "minecraft:crafting_shapeless", "category": "misc", "ingredients": ["minecraft:dirt", "minecraft:dirt", "minecraft:gold_nugget"], "result": {"count": 2, "id": "minecraft:diamond"}})
add("data/kw84/recipe/gravel_glass.json", {"type": "minecraft:smelting", "category": "blocks", "ingredient": "minecraft:gravel", "result": {"id": "minecraft:glass", "count": 3}, "experience": 0.5, "cookingtime": 40})
COMMON = {"anvil_cost": 2, "weight": 10, "min_cost": {"base": 1, "per_level_above_first": 8}, "max_cost": {"base": 30, "per_level_above_first": 8}}
add("data/kw84/enchantment/venom.json", {**COMMON, "description": {"text": "Venom"}, "supported_items": "#minecraft:enchantable/sharp_weapon", "primary_items": "#minecraft:enchantable/sharp_weapon", "max_level": 3, "slots": ["mainhand"], "effects": {
    "minecraft:damage": [{"effect": {"type": "minecraft:add", "value": {"type": "minecraft:linear", "base": 2.0, "per_level_above_first": 1.5}},
                          "requirements": {"type": "minecraft:entity_properties", "entity": "this", "predicate": {"minecraft:entity_type": "#minecraft:zombies"}}}],
    "minecraft:post_attack": [{"enchanted": "attacker", "affected": "victim", "effect": {"type": "minecraft:apply_mob_effect", "to_apply": "minecraft:poison", "min_duration": 3.0, "max_duration": 3.0, "min_amplifier": 0.0, "max_amplifier": 0.0}}]}})
add("data/kw84/enchantment/plating.json", {**COMMON, "description": {"text": "Plating"}, "supported_items": "#minecraft:enchantable/chest_armor", "max_level": 2, "slots": ["chest"], "effects": {
    "minecraft:damage_protection": [{"effect": {"type": "minecraft:add", "value": {"type": "minecraft:linear", "base": 3.0, "per_level_above_first": 2.0}}}],
    "minecraft:attributes": [{"id": "kw84:plating_speed", "attribute": "minecraft:movement_speed", "amount": {"type": "minecraft:linear", "base": 0.1, "per_level_above_first": 0.1}, "operation": "add_multiplied_base"}]}})
add("data/kw84/enchantment/miner.json", {**COMMON, "description": {"text": "Miner"}, "supported_items": "#minecraft:enchantable/mining", "max_level": 2, "slots": ["mainhand"], "effects": {
    "minecraft:block_experience": [{"effect": {"type": "minecraft:add", "value": 5.0}}], "minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 4.0}}]}})
add("data/kw84/enchantment/looter.json", {**COMMON, "description": {"text": "Looter"}, "supported_items": "#minecraft:enchantable/sharp_weapon", "max_level": 2, "slots": ["mainhand"], "effects": {
    "minecraft:mob_experience": [{"effect": {"type": "minecraft:multiply", "factor": {"type": "minecraft:linear", "base": 2.0, "per_level_above_first": 1.0}}}]}})
add("data/kw84/damage_type/laser.json", {"message_id": "kw84Laser", "scaling": "when_caused_by_living_non_player", "exhaustion": 0.3})
add("data/minecraft/tags/block/mineable/axe.json", {"values": ["minecraft:dirt", "minecraft:glass"]})
add("data/minecraft/tags/block/climbable.json", {"values": ["minecraft:glass", "minecraft:cobblestone"]})
add("data/minecraft/tags/item/planks.json", {"values": ["minecraft:bamboo_block"]})
add("data/minecraft/tags/enchantment/in_enchanting_table.json", {"values": ["kw84:venom", "kw84:plating"]})

for rel, text in FILES.items():
    p = ROOT / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text if text.endswith("\n") else text + "\n", encoding="utf-8", newline="\n")
print(len(FILES), "files in", ROOT)
