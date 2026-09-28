//! Mob NBT: `LivingEntity`, `Mob`, `AgeableMob` and the types' `addAdditionalSaveData` /
//! `readAdditionalSaveData`. Fields Kiln does not model stay in [`Entity::extra`].

use super::attributes::{Attr, Op};
use super::{MobData, MobKind, SLOT_NAMES, Species};
use crate::entity::{Entity, EntityKind};
use crate::persist::{Input, Output, uuid_to_tag};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;

fn op_name(op: Op) -> &'static str {
    match op {
        Op::AddValue => "add_value",
        Op::AddMultipliedBase => "add_multiplied_base",
        Op::AddMultipliedTotal => "add_multiplied_total",
    }
}

fn op_of(name: &str) -> Option<Op> {
    Some(match name {
        "add_value" => Op::AddValue,
        "add_multiplied_base" => Op::AddMultipliedBase,
        "add_multiplied_total" => Op::AddMultipliedTotal,
        _ => return None,
    })
}

/// Builds the mob of a loaded compound (the kind was `MobTicking` until now).
pub(crate) fn load(e: &mut Entity, kind: MobKind, r: &mut Input) {
    let mut m = MobData::new(kind, &mut e.random);
    e.max_up_step = m.attrs.value(Attr::StepHeight) as f32;
    // `LivingEntity.readAdditionalSaveData`.
    m.absorption = r.float_or("AbsorptionAmount", 0.0);
    if let Some(Tag::List(list)) = r.get("attributes") {
        for a in list {
            let Some(name) = a.get("id").and_then(Tag::as_str) else { continue };
            let Some(attr) = Attr::by_name(name) else { continue };
            if m.attrs.get(attr).is_none() {
                continue;
            }
            if let Some(base) = a.get("base").and_then(Tag::as_f64)
                && let Some(i) = m.attrs.get_mut(attr)
            {
                i.base = base;
            }
            if let Some(Tag::List(mods)) = a.get("modifiers") {
                for md in mods {
                    let (Some(id), Some(amount), Some(op)) = (
                        md.get("id").and_then(Tag::as_str),
                        md.get("amount").and_then(Tag::as_f64),
                        md.get("operation").and_then(Tag::as_str).and_then(op_of),
                    ) else {
                        continue;
                    };
                    m.attrs.set_modifier(attr, id, amount, op);
                }
            }
        }
    }
    let health = r.num("Health");
    m.health = health.map_or(m.max_health(), |h| h as f32);
    m.hurt_time = r.short_or("HurtTime", 0);
    m.death_time = r.short_or("DeathTime", 0);
    m.last_hurt_by_mob_timestamp = r.int_or("HurtByTimestamp", 0);
    if let Some(Tag::Compound(eq)) = r.get("equipment") {
        for (k, v) in eq {
            if let Some(i) = SLOT_NAMES.iter().position(|s| s == k)
                && let Ok(stack) = ItemStack::from_nbt(v)
            {
                m.equipment[i] = stack;
            }
        }
    }
    // `Mob.readAdditionalSaveData`.
    m.can_pick_up_loot = r.bool_or("CanPickUpLoot", false);
    m.persistence_required = r.bool_or("PersistenceRequired", false);
    if let Some(Tag::Compound(dc)) = r.get("drop_chances") {
        for (k, v) in dc {
            if let (Some(i), Some(f)) = (SLOT_NAMES.iter().position(|s| s == k), v.as_f64()) {
                m.drop_chances[i] = f as f32;
            }
        }
    }
    m.left_handed = r.bool_or("LeftHanded", false);
    m.no_ai = r.bool_or("NoAI", false);
    // `AgeableMob`.
    if kind.is_animal() {
        m.age = r.int_or("Age", 0);
    }
    match &mut m.species {
        Species::Sheep { color, sheared } => {
            *sheared = r.bool_or("Sheared", false);
            *color = r.byte_or("Color", 0) as u8 & 15;
        }
        Species::Chicken { egg_time } => {
            if let Some(t) = r.num("EggLayTime") {
                *egg_time = t as i32;
            }
        }
        Species::Zombie { can_break_doors } => {
            *can_break_doors = r.bool_or("CanBreakDoors", false);
            m.zombie_baby = r.bool_or("IsBaby", false);
        }
        Species::Creeper { powered, max_swell, radius, ignited, .. } => {
            *powered = r.bool_or("powered", false);
            if let Some(f) = r.num("Fuse") {
                *max_swell = f as i64 as i16 as i32;
            }
            if let Some(x) = r.num("ExplosionRadius") {
                *radius = x as i64 as i8 as i32;
            }
            *ignited = r.bool_or("ignited", false);
        }
        _ => {}
    }
    if m.zombie_baby {
        m.attrs.set_modifier(Attr::MovementSpeed, "minecraft:baby", 0.5, Op::AddMultipliedBase);
    }
    super::reassess_weapon_goal(&mut m, false);
    e.kind = EntityKind::Mob(Box::new(m));
}

/// Writes the mob's fields.
pub(crate) fn save(e: &Entity, m: &MobData, o: &mut Output) {
    o.put("Health", Tag::Float(m.health));
    o.put("HurtTime", Tag::Short(m.hurt_time as i16));
    o.put("HurtByTimestamp", Tag::Int(m.last_hurt_by_mob_timestamp));
    o.put("DeathTime", Tag::Short(m.death_time as i16));
    o.put("AbsorptionAmount", Tag::Float(m.absorption));
    let attrs: Vec<Tag> = m
        .attrs
        .list
        .iter()
        .filter(|i| !i.modifiers.is_empty() || i.base != i.attr.info().1 || matches!(i.attr, Attr::MovementSpeed | Attr::FollowRange))
        .map(|i| {
            let mut c = vec![("id".to_owned(), Tag::String(i.attr.name().to_owned())), ("base".to_owned(), Tag::Double(i.base))];
            if !i.modifiers.is_empty() {
                let mods = i
                    .modifiers
                    .iter()
                    .map(|md| {
                        Tag::Compound(vec![
                            ("id".into(), Tag::String(md.id.clone())),
                            ("amount".into(), Tag::Double(md.amount)),
                            ("operation".into(), Tag::String(op_name(md.op).into())),
                        ])
                    })
                    .collect();
                c.push(("modifiers".into(), Tag::List(mods)));
            }
            Tag::Compound(c)
        })
        .collect();
    o.put("attributes", Tag::List(attrs));
    o.put("FallFlying", Tag::Byte(0));
    let eq: Vec<(String, Tag)> =
        m.equipment.iter().zip(SLOT_NAMES).filter(|(s, _)| !s.is_empty()).map(|(s, n)| (n.to_owned(), s.to_nbt())).collect();
    if !eq.is_empty() {
        o.put("equipment", Tag::Compound(eq));
    }
    if m.last_hurt_by_player_memory > 0 {
        o.put("last_hurt_by_player_memory_time", Tag::Int(m.last_hurt_by_player_memory));
    }
    o.put("CanPickUpLoot", Tag::Byte(m.can_pick_up_loot as i8));
    o.put("PersistenceRequired", Tag::Byte(m.persistence_required as i8));
    let dc: Vec<(String, Tag)> = m
        .drop_chances
        .iter()
        .zip(SLOT_NAMES)
        .filter(|(c, _)| **c != 0.085)
        .map(|(c, n)| (n.to_owned(), Tag::Float(*c)))
        .collect();
    if !dc.is_empty() {
        o.put("drop_chances", Tag::Compound(dc));
    }
    o.put("LeftHanded", Tag::Byte(m.left_handed as i8));
    if m.no_ai {
        o.put("NoAI", Tag::Byte(1));
    }
    if m.kind.is_animal() {
        o.put("Age", Tag::Int(m.age));
        o.put("ForcedAge", Tag::Int(0));
        o.put("InLove", Tag::Int(0));
    }
    match &m.species {
        Species::Sheep { color, sheared } => {
            o.put("Sheared", Tag::Byte(*sheared as i8));
            o.put("Color", Tag::Byte(*color as i8));
        }
        Species::Chicken { egg_time } => {
            o.put("IsChickenJockey", Tag::Byte(0));
            o.put("EggLayTime", Tag::Int(*egg_time));
        }
        Species::Zombie { can_break_doors } => {
            o.put("IsBaby", Tag::Byte(m.zombie_baby as i8));
            o.put("CanBreakDoors", Tag::Byte(*can_break_doors as i8));
            o.put("InWaterTime", Tag::Int(-1));
            o.put("DrownedConversionTime", Tag::Int(-1));
        }
        Species::Creeper { powered, max_swell, radius, ignited, .. } => {
            if *powered {
                o.put("powered", Tag::Byte(1));
            }
            o.put("Fuse", Tag::Short(*max_swell as i16));
            o.put("ExplosionRadius", Tag::Byte(*radius as i8));
            o.put("ignited", Tag::Byte(*ignited as i8));
        }
        Species::Skeleton => o.put("StrayConversionTime", Tag::Int(-1)),
        _ => {}
    }
    if !e.extra.iter().any(|(k, _)| k == "Brain") {
        o.put("Brain", Tag::Compound(vec![("memories".into(), Tag::Compound(vec![]))]));
    }
    if let Some(p) = m.last_hurt_by_player.filter(|_| false) {
        o.put("last_hurt_by_player", uuid_to_tag(p as u128));
    }
}
