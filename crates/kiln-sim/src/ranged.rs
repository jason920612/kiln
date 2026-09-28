//! Items players throw and shoot: snowballs, eggs, ender pearls, splash and lingering potions
//! and experience bottles (`ThrowableItem.use` → `Projectile.spawnProjectileFromRotation`),
//! bows (`BowItem`: drawn while the use key is held, shot on release with the draw's power)
//! and the ammunition rules they share with crossbows (`ProjectileWeaponItem`: the held or
//! first inventory arrow, `draw` and `useAmmo` with infinity and multishot, the arrow's
//! pierce level, power, punch and flame from the weapon's enchantments).
//!
//! What a thrown projectile does when it lands is carried out in
//! [`crate::entities`] (`Event::ProjectileHit`).

use crate::Player;
use kiln_blocks::Level;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use crate::player_stats::{self, Stat};
use kiln_entity::math::Vec3;
use kiln_entity::mob::mth;
use kiln_entity::projectile::Throwable;
use kiln_inventory::Container;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_loot::effects::{ItemContext, ValueComponent};
use kiln_proto::packets::world_fx::SoundSource;

/// A thrown item: its projectile, the throw sound, the speed and the pitch offset.
fn throwable(name: &str) -> Option<(Throwable, &'static str, f32, f32)> {
    Some(match name {
        "minecraft:snowball" => (Throwable::Snowball, "minecraft:entity.snowball.throw", 1.5, 0.0),
        "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => (Throwable::Egg, "minecraft:entity.egg.throw", 1.5, 0.0),
        "minecraft:ender_pearl" => (Throwable::EnderPearl, "minecraft:entity.ender_pearl.throw", 1.5, 0.0),
        "minecraft:splash_potion" => (Throwable::SplashPotion, "minecraft:entity.splash_potion.throw", 0.5, -20.0),
        "minecraft:lingering_potion" => (Throwable::LingeringPotion, "minecraft:entity.lingering_potion.throw", 0.5, -20.0),
        "minecraft:experience_bottle" => (Throwable::ExperienceBottle, "minecraft:entity.experience_bottle.throw", 0.7, -20.0),
        _ => return None,
    })
}

/// Items whose `Item.use` this module handles.
pub(crate) fn handles(name: &str) -> bool {
    throwable(name).is_some() || name == "minecraft:bow"
}

/// `Item.use` for the items of [`handles`]; `off_hand`: which hand. The player is alive and
/// not a spectator.
pub(crate) fn use_item(p: &mut Player, level: &mut RegionLevel, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let stack = p.in_hand(off_hand).clone();
    if p.using.is_some() || p.on_cooldown(&stack) {
        return;
    }
    let name = stack.item_name();
    if let Some((kind, sound, speed, roll)) = throwable(name) {
        throw(p, level, off_hand, &stack, kind, sound, speed, roll, spawns);
        return;
    }
    if name == "minecraft:bow" {
        // `BowItem.use`: drawn only with something to shoot.
        if p.infinite_materials() || projectile_slot(p, &stack).is_some() {
            p.start_using(off_hand, &stack, 72000);
        }
    }
}

/// `Item.use` of held items that are simply used over time: shields (`blocks_attacks`),
/// spyglasses and goat horns (the instrument's sound, then a cooldown as long as the use).
/// Returns whether the item was one of these.
pub(crate) fn use_held(p: &mut Player, off_hand: bool, stack: &ItemStack) -> bool {
    if p.using.is_some() {
        return false;
    }
    let name = stack.item_name();
    if name == "minecraft:spyglass" {
        p.sound_for_all("minecraft:item.spyglass.use", SoundSource::Players, 1.0, 1.0);
        p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
        p.start_using(off_hand, stack, 1200);
        return true;
    }
    if name == "minecraft:goat_horn" {
        let Some((sound, seconds, range)) = instrument(stack) else { return true };
        p.start_using(off_hand, stack, (seconds * 20.0) as i32);
        // `level.playSound(player, player, ...)`: the player's client plays it itself.
        if let Some(id) = kiln_data::builtin_id("minecraft:sound_event", &sound) {
            use kiln_proto::packets::world_fx;
            let seed = p.sound_seed.next_long();
            let pkt = world_fx::sound(&world_fx::Sound::Registered(id), SoundSource::Records, p.pos, range / 16.0, 1.0, seed);
            p.pending_sounds.push(pkt);
        }
        p.add_cooldown(stack, (seconds * 20.0) as i32);
        p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
        return true;
    }
    if stack.get(kiln_item::keys::BLOCKS_ATTACKS).is_some() {
        p.start_using(off_hand, stack, 72000);
        return true;
    }
    false
}

/// A goat horn's instrument (`minecraft:instrument`): its sound, use seconds and range. The
/// vanilla horns all play for 7 seconds with a range of 256.
fn instrument(stack: &ItemStack) -> Option<(String, f32, f32)> {
    const HORNS: [&str; 8] = ["ponder", "sing", "seek", "feel", "admire", "call", "yearn", "dream"];
    let comp = stack.get(kiln_item::keys::INSTRUMENT)?;
    match &comp.0 {
        kiln_item::Holder::Reference(id) => {
            let name = kiln_item::registry::INSTRUMENT.name(*id)?;
            let short = name.strip_prefix("minecraft:").unwrap_or(name).strip_suffix("_goat_horn")?;
            let i = HORNS.iter().position(|h| *h == short)?;
            Some((format!("minecraft:item.goat_horn.sound.{i}"), 7.0, 256.0))
        }
        kiln_item::Holder::Direct(def) => {
            let sound = match &def.sound_event {
                kiln_item::Holder::Reference(id) => kiln_item::registry::SOUND_EVENT.name(*id)?.to_owned(),
                kiln_item::Holder::Direct(_) => return None,
            };
            Some((sound, def.use_duration, def.range))
        }
    }
}

/// `ThrowableItem.use`: the sound (the level's random for the pitch), the projectile from the
/// eyes, one used up, `use_cooldown`.
#[allow(clippy::too_many_arguments)]
fn throw(p: &mut Player, level: &mut RegionLevel, off_hand: bool, stack: &ItemStack, kind: Throwable, sound: &'static str, speed: f32, roll: f32, spawns: &mut Vec<Spawn>) {
    let pitch = 0.4 / (level.random().next_float() * 0.4 + 0.8);
    p.sound_for_all(sound, SoundSource::Neutral, 0.5, pitch);
    let seed = projectile_seed(level, p, spawns.len() as u64);
    let pos = Vec3::new(p.pos[0], p.eye_position()[1] - 0.10000000149011612, p.pos[2]);
    let mut e = kiln_entity::projectile::new(0, 0, kind, pos, Vec3::ZERO, Some(p.entity_id), seed);
    if let kiln_entity::EntityKind::Throwable(d) = &mut e.kind {
        d.item = Some(stack.with_count(1));
    }
    shoot_from_rotation(&mut e, p, p.rot[1], p.rot[0], roll, speed, 1.0);
    push_spawn(spawns, e);
    p.award_stat(Stat::item(player_stats::USED, stack.item()), 1);
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
    p.apply_use_cooldown(stack);
}

/// A seed for a projectile's own random (`Entity.random`), independent of how regions split
/// the world.
fn projectile_seed(level: &RegionLevel, p: &Player, n: u64) -> i64 {
    let at = kiln_blocks::BlockPos::new(p.entity_id, p.tick_count, n as i32);
    crate::container::pos_random(level, at, 0x7072_6f6a).next_long()
}

fn push_spawn(spawns: &mut Vec<Spawn>, e: kiln_entity::Entity) {
    let Some(kind) = kiln_data::entities::by_name(e.type_name) else { return };
    let (pos, vel) = (e.position(), e.delta);
    spawns.push(Spawn { kind, pos: [pos.x, pos.y, pos.z], vel: [vel.x, vel.y, vel.z], body: Body::Ready(Box::new(e)) });
}

/// `Projectile.shootFromRotation`: along the rotation (with `roll` added to the pitch), plus the
/// shooter's own movement (not its vertical movement on the ground).
pub(crate) fn shoot_from_rotation(e: &mut kiln_entity::Entity, p: &Player, x_rot: f32, y_rot: f32, roll: f32, speed: f32, inaccuracy: f32) {
    const RAD: f32 = 0.017453292;
    let x = -mth::sin((y_rot * RAD) as f64) * mth::cos((x_rot * RAD) as f64);
    let y = -mth::sin(((x_rot + roll) * RAD) as f64);
    let z = mth::cos((y_rot * RAD) as f64) * mth::cos((x_rot * RAD) as f64);
    kiln_entity::mob::species::shoot(e, x as f64, y as f64, z as f64, speed, inaccuracy);
    let m = p.known_movement;
    e.delta = e.delta.add(m[0], if p.on_ground { 0.0 } else { m[1] }, m[2]);
}

/// `BowItem.getPowerForTime`.
pub(crate) fn bow_power(ticks: i32) -> f32 {
    let f = ticks as f32 / 20.0;
    ((f * f + f * 2.0) / 3.0).min(1.0)
}

/// Whether a stack is an arrow (`#minecraft:arrows`, `ProjectileWeaponItem.ARROW_ONLY`).
fn is_arrow(s: &ItemStack) -> bool {
    !s.is_empty() && kiln_inventory::tags::contains("minecraft:item", "minecraft:arrows", s.item())
}

/// `Player.getProjectile(weapon)`: the slot of the arrow a bow or crossbow shoots (a held one
/// first), else the first arrow in the inventory. `None`: nothing to shoot (creative players
/// get a fresh arrow instead). Approximation: crossbows do not load held fireworks.
pub(crate) fn projectile_slot(p: &Player, _weapon: &ItemStack) -> Option<usize> {
    let held_ok = is_arrow;
    for off in [true, false] {
        if held_ok(p.in_hand(off)) {
            return Some(p.hand_index(off));
        }
    }
    (0..p.inv.size()).find(|&i| is_arrow(p.inv.item(i)))
}

/// The weapon's enchantments' value effects of `c` on `value`, each in the item context of
/// `tool` (`EnchantmentHelper.modifyItemFilteredCount` and friends).
pub(crate) fn weapon_value(p: &Player, weapon: &ItemStack, tool: &ItemStack, c: ValueComponent, value: f32) -> f32 {
    let Some(loot) = p.loot.clone() else { return value };
    let mut rng = LegacyRandom::new(0);
    let mut v = value;
    loot.for_each_enchantment(weapon, |e, lvl| {
        let ctx = ItemContext { tool, level: lvl };
        v = loot.apply_value_effects(e, c, lvl, &ctx, &mut rng, v);
    });
    v
}

/// Whether the weapon has an enchantment with `projectile_spawned` effects (flame).
fn sets_projectiles_on_fire(p: &Player, weapon: &ItemStack) -> bool {
    let Some(loot) = p.loot.clone() else { return false };
    let mut fire = false;
    loot.for_each_enchantment(weapon, |e, _| {
        fire |= e.effects.other.iter().any(|id| id.as_str() == "minecraft:projectile_spawned");
    });
    fire
}

/// `ProjectileWeaponItem.draw`: the projectiles one shot fires, taken from the ammunition in
/// `slot` (`None`: a creative player's free arrow). The first uses ammunition as
/// `useAmmo` says (infinity and creative players keep it, marked intangible); multishot's
/// extra copies are always intangible.
pub(crate) fn draw(p: &mut Player, weapon: &ItemStack, slot: Option<usize>) -> Vec<ItemStack> {
    let ammo = match slot {
        Some(i) => p.inv.item(i).clone(),
        None => ItemStack::of("minecraft:arrow", 1).unwrap_or_else(ItemStack::empty),
    };
    if ammo.is_empty() {
        return Vec::new();
    }
    let count = weapon_value(p, weapon, weapon, ValueComponent::ProjectileCount, 1.0).max(0.0) as i32;
    let copy = ammo.clone();
    let mut out = Vec::with_capacity(count as usize);
    for j in 0..count {
        let s = if j == 0 { use_ammo(p, weapon, slot, &ammo, false) } else { use_ammo(p, weapon, None, &copy, true) };
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

/// `ProjectileWeaponItem.useAmmo`.
fn use_ammo(p: &mut Player, weapon: &ItemStack, slot: Option<usize>, ammo: &ItemStack, intangible: bool) -> ItemStack {
    let used = if !intangible && !p.infinite_materials() {
        weapon_value(p, weapon, ammo, ValueComponent::AmmoUse, 1.0).max(0.0) as i32
    } else {
        0
    };
    if used > ammo.count() {
        return ItemStack::empty();
    }
    if used == 0 {
        let mut one = ammo.with_count(1);
        one.insert(kiln_item::keys::INTANGIBLE_PROJECTILE, kiln_item::component::IntangibleProjectile);
        return one;
    }
    let Some(i) = slot else { return ammo.with_count(used) };
    let taken = p.inv.item_mut(i).split(used);
    p.inv.times_changed += 1;
    taken
}

/// `ArrowItem.createArrow` and `ProjectileWeaponItem.createProjectile`: an arrow (spectral,
/// tipped or plain) from the shooter's eyes, with the weapon's pierce level, damage and
/// knockback, picked up as its item unless it was intangible (creative only then).
pub(crate) fn create_arrow(p: &Player, weapon: &ItemStack, ammo: &ItemStack, crit: bool, seed: i64) -> kiln_entity::Entity {
    let spectral = ammo.item_name() == "minecraft:spectral_arrow";
    let type_name = if spectral { "minecraft:spectral_arrow" } else { "minecraft:arrow" };
    let pos = Vec3::new(p.pos[0], p.eye_position()[1] - 0.10000000149011612, p.pos[2]);
    let mut e = kiln_entity::arrow::new(0, 0, type_name, pos, Vec3::ZERO, Some(p.entity_id), seed);
    let mut pickup_item = ammo.with_count(1);
    let intangible = pickup_item.has(kiln_item::component::ids::INTANGIBLE_PROJECTILE);
    pickup_item.remove(kiln_item::component::ids::INTANGIBLE_PROJECTILE);
    let pierce = weapon_value(p, weapon, ammo, ValueComponent::ProjectilePiercing, 0.0).max(0.0) as i32;
    let damage = weapon_value(p, weapon, weapon, ValueComponent::Damage, 2.0) as f64;
    let knockback = weapon_value(p, weapon, weapon, ValueComponent::Knockback, 0.0) as f64;
    let effects = match pickup_item.get(kiln_item::keys::POTION_CONTENTS) {
        Some(c) if !spectral => crate::effects::potion_effects(c, 1.0)
            .into_iter()
            .filter_map(|fx| kiln_data::builtin_entries("minecraft:mob_effect").and_then(|l| l.get(fx.id as usize).copied()).map(|n| (n, fx.duration, fx.amplifier)))
            .collect(),
        _ => Vec::new(),
    };
    if let kiln_entity::EntityKind::Arrow(a) = &mut e.kind {
        a.pickup = if intangible { kiln_entity::arrow::PICKUP_CREATIVE_ONLY } else { kiln_entity::arrow::PICKUP_ALLOWED };
        a.pickup_item = Some(pickup_item);
        a.weapon = Some(weapon.clone());
        a.pierce_level = pierce.clamp(0, 127) as u8;
        a.base_damage = damage;
        a.knockback = knockback;
        a.crit = crit;
        a.effects = effects;
    }
    e
}

/// `ProjectileWeaponItem.shoot`: each projectile spread over the weapon's `projectile_spread`
/// (alternating sides), shot by `aim`, the weapon worn per projectile. Returns the entities
/// shot, for the crossbow's triggers.
#[allow(clippy::too_many_arguments)]
pub(crate) fn shoot(
    p: &mut Player,
    level: &RegionLevel,
    off_hand: bool,
    weapon: &ItemStack,
    projectiles: Vec<ItemStack>,
    crit: bool,
    spawns: &mut Vec<Spawn>,
    aim: &dyn Fn(&mut kiln_entity::Entity, &Player, usize, f32),
) -> usize {
    let spread = weapon_value(p, weapon, weapon, ValueComponent::ProjectileSpread, 0.0);
    let n = projectiles.len();
    let step = if n == 1 { 0.0 } else { 2.0 * spread / (n - 1) as f32 };
    let start = ((n - 1) % 2) as f32 * step / 2.0;
    let mut sign = 1.0f32;
    let mut shot = 0;
    let fire = sets_projectiles_on_fire(p, weapon);
    for (j, ammo) in projectiles.into_iter().enumerate() {
        if ammo.is_empty() {
            continue;
        }
        let angle = start + sign * ((j + 1) / 2) as f32 * step;
        sign = -sign;
        let seed = projectile_seed(level, p, (spawns.len() + j) as u64);
        let mut e = create_arrow(p, weapon, &ammo, crit, seed);
        aim(&mut e, p, j, angle);
        // `applyOnProjectileSpawned`: flame sets the arrow alight.
        if fire && matches!(e.kind, kiln_entity::EntityKind::Arrow(_)) {
            e.ignite_for_seconds(100.0);
        }
        push_spawn(spawns, e);
        shot += 1;
        p.hurt_and_break(if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand }, 1, None);
        if p.in_hand(off_hand).is_empty() {
            break;
        }
    }
    shot
}

/// `BowItem.releaseUsing` after `ticks` of drawing.
pub(crate) fn release_bow(p: &mut Player, level: &mut RegionLevel, off_hand: bool, bow: &ItemStack, ticks: i32, spawns: &mut Vec<Spawn>) {
    let slot = projectile_slot(p, bow);
    if slot.is_none() && !p.infinite_materials() {
        return;
    }
    let power = bow_power(ticks);
    if (power as f64) < 0.1 {
        return;
    }
    let projectiles = draw(p, bow, slot);
    if !projectiles.is_empty() {
        let aim = |e: &mut kiln_entity::Entity, p: &Player, _: usize, angle: f32| {
            shoot_from_rotation(e, p, p.rot[1], p.rot[0] + angle, 0.0, power * 3.0, 1.0);
        };
        shoot(p, level, off_hand, bow, projectiles, power == 1.0, spawns, &aim);
    }
    let pitch = 1.0 / (level.random().next_float() * 0.4 + 1.2) + power * 0.5;
    p.sound_for_all("minecraft:entity.arrow.shoot", SoundSource::Players, 1.0, pitch);
    p.award_stat(Stat::item(player_stats::USED, bow.item()), 1);
}

/// `ServerPlayer.releaseUsingItem` (the use key let go): the item's `releaseUsing`, then the
/// use stops.
pub(crate) fn release_using(p: &mut Player, level: &mut RegionLevel, spawns: &mut Vec<Spawn>) {
    let Some(u) = p.using else { return };
    let stack = p.in_hand(u.off_hand).clone();
    if !stack.is_empty() && stack.item() == u.item {
        let ticks = u.duration - u.remaining;
        match stack.item_name() {
            "minecraft:bow" => release_bow(p, level, u.off_hand, &stack, ticks, spawns),
            "minecraft:crossbow" => crate::crossbow::release(p, level, u.off_hand, &stack, ticks),
            "minecraft:trident" => crate::trident::release(p, level, u.off_hand, &stack, ticks, spawns),
            "minecraft:spyglass" => p.sound_for_all("minecraft:item.spyglass.stop_using", SoundSource::Players, 1.0, 1.0),
            _ => {}
        }
    }
    p.stop_using();
}

impl Player {
    /// `LivingEntity.startUsingItem` for an item used over `duration` ticks.
    pub(crate) fn start_using(&mut self, off_hand: bool, stack: &ItemStack, duration: i32) {
        self.using = Some(crate::consume::Using { off_hand, item: stack.item(), remaining: duration, duration, sounds: 0 });
        self.meta_dirty = true;
    }

    /// `level.playSound(null, player position, ...)`: the player and everyone near hear it.
    pub(crate) fn sound_for_all(&mut self, sound: &str, source: SoundSource, volume: f32, pitch: f32) {
        use kiln_proto::packets::world_fx;
        let Some(id) = kiln_data::builtin_id("minecraft:sound_event", sound) else { return };
        let seed = self.sound_seed.next_long();
        let pkt = world_fx::sound(&world_fx::Sound::Registered(id), source, self.pos, volume, pitch, seed);
        self.send(pkt.clone());
        self.pending_sounds.push(pkt);
    }

    /// `ItemCooldowns.getCooldownGroup`: the `use_cooldown` group, else the item's id.
    fn cooldown_group(stack: &ItemStack) -> String {
        stack
            .get(kiln_item::keys::USE_COOLDOWN)
            .and_then(|c| c.cooldown_group.as_ref().map(|g| g.to_string()))
            .unwrap_or_else(|| stack.item_name().to_owned())
    }

    /// `ItemCooldowns.isOnCooldown`.
    pub(crate) fn on_cooldown(&self, stack: &ItemStack) -> bool {
        let group = Self::cooldown_group(stack);
        self.item_cooldowns.iter().any(|(g, end)| *g == group && *end > self.tick_count)
    }

    /// `ItemCooldowns.addCooldown` (`ServerItemCooldowns` tells the client).
    pub(crate) fn add_cooldown(&mut self, stack: &ItemStack, ticks: i32) {
        let group = Self::cooldown_group(stack);
        self.item_cooldowns.retain(|(g, _)| *g != group);
        self.item_cooldowns.push((group.clone(), self.tick_count + ticks));
        self.send(kiln_proto::packets::player::cooldown(&group, ticks));
    }

    /// `UseCooldown.apply` after a use.
    pub(crate) fn apply_use_cooldown(&mut self, stack: &ItemStack) {
        if let Some(c) = stack.get(kiln_item::keys::USE_COOLDOWN) {
            let ticks = (c.seconds * 20.0) as i32;
            if ticks > 0 {
                self.add_cooldown(stack, ticks);
            }
        }
    }

    /// `ItemCooldowns.tick`: ended cooldowns are forgotten (the client is told).
    pub(crate) fn tick_cooldowns(&mut self) {
        let now = self.tick_count;
        let ended: Vec<String> = self.item_cooldowns.iter().filter(|(_, end)| *end <= now).map(|(g, _)| g.clone()).collect();
        if ended.is_empty() {
            return;
        }
        self.item_cooldowns.retain(|(_, end)| *end > now);
        for g in ended {
            self.send(kiln_proto::packets::player::cooldown(&g, 0));
        }
    }
}
