//! `Mob.convertTo` with `ConversionType.SINGLE`: a mob turns into one of another type (a zombie
//! drowning into a drowned, a husk into a zombie, a skeleton freezing into a stray).

use super::{MobData, MobKind};
use crate::entity::Entity;
use crate::level::EntityLevel;

/// `Mob.convertTo(type, ConversionParams.single(mob, keep_equipment, preserve_loot), after)`: the
/// new mob takes the old one's place, `after` sets it up (the caller's `AfterConversion`), it
/// joins the level and the old one is discarded. Returns the new mob's id.
///
/// Copied (`ConversionType.SINGLE.convert` and `convertCommon`): position and rotation, motion,
/// equipment with its drop chances (when kept, the old mob's slots are emptied), fall distance,
/// the player-hurt memory, hurt time, body yaw, on-ground, absorption, the baby flag and age,
/// loot pickup (when preserved), left-handedness, no-AI, persistence, invulnerability, no
/// gravity, silence and custom name, and for zombie to zombie the door breaking. Not copied:
/// health (the new type's default), attributes, fire ticks. Riding, leashes, teams and effects
/// are not simulated on mobs yet.
pub fn convert_to(
    e: &mut Entity,
    m: &mut MobData,
    level: &mut dyn EntityLevel,
    kind: MobKind,
    keep_equipment: bool,
    preserve_loot: bool,
    after: impl FnOnce(&mut Entity, &mut MobData, &mut dyn EntityLevel),
) -> Option<i32> {
    if e.is_removed() {
        return None;
    }
    let id = level.next_entity_id();
    let seed = level.fresh_seed();
    let uuid = (seed as u64 as u128) << 64 | (seed.rotate_left(17) as u64 as u128);
    let mut ne = super::new(kind, id, uuid, seed);
    let mut nm = super::take(&mut ne);
    // `copyPosition`.
    ne.set_pos(e.position());
    ne.y_rot = e.y_rot;
    ne.x_rot = e.x_rot;
    ne.set_old_pos_and_rot();
    ne.delta = e.delta;
    if keep_equipment {
        for i in 0..6 {
            if !m.equipment[i].is_empty() {
                nm.equipment[i] = std::mem::replace(&mut m.equipment[i], kiln_item::ItemStack::empty());
                nm.drop_chances[i] = m.drop_chances[i];
            }
        }
    }
    ne.fall_distance = e.fall_distance;
    nm.last_hurt_by_player_memory = m.last_hurt_by_player_memory;
    nm.hurt_time = m.hurt_time;
    nm.y_body_rot = m.y_body_rot;
    ne.on_ground = e.on_ground;
    // `convertCommon`.
    nm.absorption = m.absorption;
    if m.baby() {
        set_baby(&mut ne, &mut nm, true);
    }
    if super::breed::is_ageable(m.kind) && super::breed::is_ageable(kind) {
        let age = m.age;
        super::set_age(&mut ne, &mut nm, age);
        nm.forced_age = m.forced_age;
        nm.forced_age_timer = m.forced_age_timer;
    }
    if preserve_loot {
        nm.can_pick_up_loot = m.can_pick_up_loot;
    }
    nm.left_handed = m.left_handed;
    nm.no_ai = m.no_ai;
    if m.persistence_required {
        nm.persistence_required = true;
    }
    ne.invulnerable = e.invulnerable;
    ne.no_gravity = e.no_gravity;
    ne.silent = e.silent;
    for key in ["CustomName", "CustomNameVisible", "Tags"] {
        if let Some(v) = e.extra.iter().find(|(k, _)| k == key).cloned() {
            ne.extra.push(v);
        }
    }
    if m.kind.is_zombie() && kind.is_zombie() && super::kinds::zombie::can_break_doors(m) {
        super::kinds::zombie::set_can_break_doors(&mut nm, true);
    }
    after(&mut ne, &mut nm, level);
    super::put(&mut ne, nm);
    level.add_entity(ne);
    e.discard();
    Some(id)
}

/// `setBaby(true)`: a zombie's baby flag and speed bonus, or an ageable mob's baby age.
pub fn set_baby(e: &mut Entity, m: &mut MobData, baby: bool) {
    if m.kind.is_zombie() {
        super::kinds::zombie::set_baby(e, m, baby);
    } else if super::breed::is_ageable(m.kind) {
        super::set_age(e, m, if baby { super::breed::BABY_START_AGE } else { 0 });
    }
}
