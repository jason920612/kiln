//! Checks `EnchantmentHelper`-level functions against the vanilla vectors
//! `tools/CombatVectors.java` writes to `enchant_helpers.jsonl` (`KILN_ENCHANT_VECTORS`, set by
//! `tools/combat_vectors.py`): `modifyDamage` (against players, undead, arthropods, aquatic
//! mobs...), `modifyKnockback`, `modifyArmorEffectiveness`, `getDamageProtection` and
//! `isImmuneToDamage` for armor sets and damage types, `processDurabilityChange` with a seeded
//! level random (and the random's state after it), `forEachModifier` per slot, and
//! `Player.getDestroySpeed` with efficiency and aqua affinity. Floats must match bit for bit.
//! Skipped when the vectors are not there.

use crate::combat_parity::{enchant, vanilla_loot};
use crate::enchant::{DamageContext, EntityView, player_type};
use crate::health::{Attacker, Cause, Source, static_damage_type};
use crate::testing::join;
use crate::{Sim, SimConfig};
use kiln_item::component::EquipmentSlot;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use serde_json::Value;

fn view(type_name: &str, pos: [f64; 3]) -> EntityView {
    EntityView {
        type_id: kiln_item::registry::ENTITY_TYPE.id(type_name).unwrap_or_else(|| panic!("entity type {type_name}")),
        pos,
        on_ground: false,
        on_fire: false,
        sneaking: false,
        sprinting: false,
        flying: false,
    }
}

fn stack(item: &Value, enchantments: &Value) -> ItemStack {
    let Some(name) = item.as_str() else { return ItemStack::empty() };
    let mut s = ItemStack::of(name, 1).unwrap_or_else(|| panic!("unknown item {name}"));
    enchant(&mut s, enchantments);
    s
}

fn attacker() -> Attacker {
    Attacker {
        id: 1,
        name: "EnchHelperA".into(),
        pos: [0.5, 100.0, 0.5],
        creative: false,
        weapon: None,
        view: EntityView { type_id: player_type(), ..view("minecraft:player", [0.5, 100.0, 0.5]) },
    }
}

/// `CombatVectors`' `EnchantHelperVectors.source`.
fn source(name: &str) -> Source {
    match name {
        "player_attack" => Source::melee(attacker(), ItemStack::empty()),
        "generic" => Cause::Other("minecraft:generic").into(),
        other => {
            let full = format!("minecraft:{other}");
            let name = static_damage_type(&full);
            assert_eq!(name, full, "damage type {other}");
            Cause::Other(name).into()
        }
    }
}

fn f32_of(v: &Value) -> f32 {
    v.as_f64().unwrap() as f32
}

fn check(line: &Value, loot: &kiln_loot::LootData, sim: &mut Sim) -> Result<(), String> {
    let kind = line["kind"].as_str().unwrap();
    let mut rng = LegacyRandom::new(0);
    let eq = |what: &str, got: String, want: String| if got == want { Ok(()) } else { Err(format!("{what}: kiln {got}, vanilla {want}")) };
    match kind {
        "damage" | "knockback" | "armor_effectiveness" => {
            let weapon = stack(&line["item"], &line["enchantments"]);
            let target = view(line["target"].as_str().unwrap(), [0.5, 100.0, 2.5]);
            let source = Source::melee(attacker(), weapon.clone());
            let base = f32_of(&line["base"]);
            let ctx = |level| DamageContext { level, this: &target, source: &source };
            let got = match kind {
                "damage" => loot.modify_damage(&weapon, &mut rng, base, ctx),
                "knockback" => loot.modify_knockback(&weapon, &mut rng, base, ctx),
                _ => loot.modify_armor_effectiveness(&weapon, &mut rng, base, ctx),
            };
            eq("result", format!("{got:?}"), format!("{:?}", f32_of(&line["result"])))
        }
        "protection" => {
            let armor: Vec<ItemStack> =
                (0..4).map(|i| stack(&line["armor"][i], &line["armor_enchantments"][i])).collect();
            let slots = [EquipmentSlot::Feet, EquipmentSlot::Legs, EquipmentSlot::Chest, EquipmentSlot::Head];
            let equipment: Vec<(EquipmentSlot, &ItemStack)> = slots.iter().copied().zip(armor.iter()).collect();
            let source = source(line["source"].as_str().unwrap());
            let this = view("minecraft:player", [0.5, 100.0, 2.5]);
            let ctx = |level| DamageContext { level, this: &this, source: &source };
            let got = loot.damage_protection(&equipment, &mut rng, ctx);
            eq("result", format!("{got:?}"), format!("{:?}", f32_of(&line["result"])))?;
            let immune = loot.is_immune_to_damage(&equipment, &mut rng, ctx);
            eq("immune", format!("{immune}"), format!("{}", line["immune"].as_bool().unwrap()))
        }
        "durability" => {
            let s = stack(&line["item"], &line["enchantments"]);
            let mut rng = LegacyRandom::new(line["seed"].as_i64().unwrap());
            let got = loot.process_durability_change(&s, &mut rng, line["amount"].as_i64().unwrap() as i32);
            eq("result", format!("{got}"), format!("{}", line["result"].as_i64().unwrap()))?;
            eq("next_int", format!("{}", rng.next_int()), format!("{}", line["next_int"].as_i64().unwrap()))
        }
        "modifiers" => {
            let s = stack(&line["item"], &line["enchantments"]);
            let slot = EquipmentSlot::from_name(line["slot"].as_str().unwrap()).unwrap();
            let mut got = Vec::new();
            loot.enchantment_modifiers(&s, slot, |attr, id, amount, op| {
                let attr = kiln_item::registry::ATTRIBUTE.name(attr).unwrap_or("?");
                got.push(format!("{attr} {id} {amount:?} {}", op.name()));
            });
            let want: Vec<String> = line["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| format!("{} {} {:?} {}", m[0].as_str().unwrap(), m[1].as_str().unwrap(), m[2].as_f64().unwrap(), m[3].as_str().unwrap()))
                .collect();
            // Vanilla walks the enchantments in identity hash order, which changes between runs;
            // modifiers of different enchantments are independent, so compare them as sets.
            let (mut got, mut want) = (got, want);
            got.sort();
            want.sort();
            eq("modifiers", format!("{got:?}"), format!("{want:?}"))
        }
        "destroy_speed" => {
            let p = sim.players.get_mut(&1).unwrap();
            p.loot = vanilla_loot();
            p.inv = kiln_inventory::PlayerInventory::new();
            p.inv.items[0] = stack(&line["item"], &line["enchantments"]);
            if line["helmet_aqua_affinity"].as_bool().unwrap() {
                let mut helmet = ItemStack::of("minecraft:turtle_helmet", 1).unwrap();
                enchant(&mut helmet, &serde_json::json!({"minecraft:aqua_affinity": 1}));
                p.inv.equipment[3] = helmet;
            }
            for (i, slot) in crate::combat::SLOTS.iter().enumerate() {
                p.equipment_seen[i] = p.inv.equipped(*slot).clone();
            }
            p.on_ground = line["on_ground"].as_bool().unwrap();
            let efficiency = p.attribute(crate::combat::MINING_EFFICIENCY);
            eq("mining_efficiency", format!("{efficiency:?}"), format!("{:?}", line["mining_efficiency"].as_f64().unwrap()))?;
            let submerged = p.attribute(crate::combat::SUBMERGED_MINING_SPEED);
            eq("submerged_mining_speed", format!("{submerged:?}"), format!("{:?}", line["submerged_mining_speed"].as_f64().unwrap()))?;
            let block = line["block"].as_str().unwrap();
            let state = kiln_blocks::BlockId::by_name(block).unwrap_or_else(|| panic!("block {block}")).default_state();
            let got = p.destroy_speed(state, line["eye_in_water"].as_bool().unwrap());
            eq("result", format!("{got:?}"), format!("{:?}", f32_of(&line["result"])))
        }
        other => Err(format!("unknown kind {other}")),
    }
}

#[test]
fn enchant_parity() {
    let Some(path) = std::env::var_os("KILN_ENCHANT_VECTORS") else {
        eprintln!("skipped: set KILN_ENCHANT_VECTORS (tools/combat_vectors.py)");
        return;
    };
    let loot = vanilla_loot().expect("the vanilla datapack (KILN_WORK/generated)");
    let mut sim = Sim::new(SimConfig::new(2, 2, None));
    let (msg, _stats) = join(1, "EnchHelperB", 2);
    assert!(sim.step([msg]));
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (std::collections::BTreeMap::<String, usize>::new(), Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        assert!(v.get("error").is_none(), "vanilla failed: {}", v["error"]);
        match check(&v, &loot, &mut sim) {
            Ok(()) => *passed.entry(v["kind"].as_str().unwrap().to_owned()).or_default() += 1,
            Err(e) => {
                if failed.len() < 40 {
                    println!("FAIL {line}\n  {e}");
                }
                failed.push(e);
            }
        }
    }
    println!("enchant parity: {passed:?} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "{} vectors failed", failed.len());
}
