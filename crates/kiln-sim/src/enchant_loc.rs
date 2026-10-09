//! Enchantments that act where their wearer stands: the `location_changed` effects of soul speed (a movement
//! speed and efficiency modifier while on soul sand or soil, worn down now and then) and frost walker (a disk
//! of frosted ice over the water around the wearer), and soul speed's `tick` effects (soul particles and a
//! sound while walking on soul blocks). Vanilla runs them whenever the wearer's block position changes or the
//! wearer lands (`LivingEntity.onChangedBlock` → `EnchantmentHelper.runLocationChangedEffects`), and when the
//! boots are put on or taken off.
//!
//! Frost walker's immunity to hot floors (`damage_immunity`) is the loot engine's generic one.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::hazards::BlockAt;
use kiln_blocks::{Effect, Level};
use kiln_entity::math::BlockPos;
use kiln_data::blocks::default_state as d;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;

/// `LevelBasedValue.Linear(0.0405, 0.0105)` at `level`, as the float vanilla adds up.
pub(crate) fn soul_speed_amount(level: i32) -> f64 {
    (0.0405f32 + 0.0105f32 * (level - 1) as f32) as f64
}

fn enchant_level(stack: &ItemStack, name: &str) -> i32 {
    if stack.is_empty() {
        return 0;
    }
    let Some(id) = kiln_item::registry::ENCHANTMENT.id(name) else { return 0 };
    kiln_loot::enchant::item_level(stack, id)
}

impl Player {
    /// `Entity.isFlying` of `EntityFlagsPredicate`: gliding, or flying with the abilities.
    fn flag_flying(&self) -> bool {
        self.fall_flying || self.flying
    }

    /// `getBlockPosBelowThatAffectsMyMovement`: the block under the feet that sets the speed.
    fn block_affecting_movement(&self, block: BlockAt) -> u16 {
        let blocks = |p: BlockPos| Some(block(p));
        block(self.on_pos(&blocks, 0.500001))
    }

    /// `LivingEntity.baseTick`: the block position changed since the last tick.
    pub(crate) fn tick_location_changed(&mut self, block: BlockAt) {
        let now = [self.pos[0].floor() as i32, self.pos[1].floor() as i32, self.pos[2].floor() as i32];
        if self.loc_last_pos != Some(now) {
            self.loc_last_pos = Some(now);
            self.run_location_changed(block);
        }
        self.soul_speed_tick(block);
    }

    /// `LivingEntity.checkFallDamage`: a landing runs the effects too.
    pub(crate) fn landed_location_changed(&mut self, block: BlockAt) {
        if std::mem::take(&mut self.loc_landed) {
            self.run_location_changed(block);
        }
    }

    /// `ServerPlayer`: the boots changed (put on, taken off or broken): their effects start or stop at once.
    pub(crate) fn boots_changed(&mut self, block: BlockAt) {
        self.run_location_changed(block);
    }

    /// `EnchantmentHelper.runLocationChangedEffects` for the boots.
    fn run_location_changed(&mut self, block: BlockAt) {
        let boots = self.worn(EquipmentSlot::Feet).clone();
        let soul = enchant_level(&boots, "minecraft:soul_speed");
        let frost = enchant_level(&boots, "minecraft:frost_walker");
        let riding = self.vehicle.is_some();
        let on_ground = self.on_ground;
        // Soul speed: the first effect (its modifiers, with their own active check) ...
        if soul > 0 {
            let on_soul = kiln_blocks::tags::is(self.block_affecting_movement(block), "minecraft:soul_speed_blocks");
            let flying = self.flag_flying();
            let active = self.soul_speed.is_some();
            let holds = !riding && !flying && if active { on_soul || !on_ground } else { on_soul };
            if holds {
                if self.soul_speed != Some(soul) {
                    self.soul_speed = Some(soul);
                    self.attributes_dirty = true;
                }
            } else if active {
                self.soul_speed = None;
                self.attributes_dirty = true;
            }
            // ... and the second: one time in 25 per level, on soul blocks, the boots lose a point.
            let chance = (0.04f32 * soul as f32).min(1.0);
            let rolled = self.level_rng.next_float() < chance;
            if rolled && on_ground && on_soul {
                self.hurt_and_break(EquipmentSlot::Feet, 1, None);
            }
        } else if self.soul_speed.take().is_some() {
            self.attributes_dirty = true;
        }
        if frost > 0 && on_ground && !riding {
            // `ReplaceDisk`: radius 3 (one more per level above the first, at most 16), one layer, centred under the feet.
            let radius = (3.0f32 + (frost - 1) as f32).clamp(0.0, 16.0) as i32;
            let origin = BlockPos::new(self.pos[0].floor() as i32, self.pos[1].floor() as i32 - 1, self.pos[2].floor() as i32);
            self.block_edits.push(crate::fall::BlockEdit::FrostWalker { origin, radius, pos: self.pos });
        }
    }

    /// Soul speed's `tick` effects: every fifth tick of walking on soul blocks, soul particles (and now and then
    /// a sound).
    fn soul_speed_tick(&mut self, block: BlockAt) {
        let boots = self.worn(EquipmentSlot::Feet);
        if enchant_level(boots, "minecraft:soul_speed") == 0 {
            return;
        }
        if self.tick_count % 5 != 0 || self.flag_flying() || !self.on_ground || self.speed_h < 1.0e-5 {
            return;
        }
        if !kiln_blocks::tags::is(self.block_affecting_movement(block), "minecraft:soul_speed_blocks") {
            return;
        }
        // `SpawnParticlesEffect`: a soul particle in the body's box, drifting back and up.
        if let Some(soul) = kiln_data::builtin_id("minecraft:particle_type", "minecraft:soul") {
            use kiln_proto::packets::world_fx;
            let (w, _, _) = self.dimensions();
            let x = self.pos[0] + (self.level_rng.next_double() - 0.5) * w as f64;
            let z = self.pos[2] + (self.level_rng.next_double() - 0.5) * w as f64;
            let pkt = world_fx::level_particles(&world_fx::LevelParticles {
                particle: world_fx::Particle { kind: soul, options: world_fx::ParticleOptions::None },
                override_limiter: false,
                always_show: false,
                pos: [x, self.pos[1] + 0.1, z],
                offset: [(self.vel[0] * -0.2) as f32, 0.1, (self.vel[2] * -0.2) as f32],
                max_speed: [1.0, 1.0, 1.0],
                count: 0,
                randomization: world_fx::ParticleRandomization::Alternative,
            });
            self.send(pkt);
        }
        // `PlaySoundEffect`: 35% of the time, at a pitch from 0.6 to 1.
        if self.level_rng.next_float() < 0.35 {
            let pitch = 0.6 + self.level_rng.next_float() * 0.4;
            self.sound_for_all("minecraft:particle.soul_escape", kiln_proto::packets::world_fx::SoundSource::Players, 0.6, pitch);
        }
    }
}

/// `ReplaceDisk` of frost walker: the water source blocks under open air within `radius` of `pos` (on the layer
/// of `origin`) turn into frosted ice, unless something solid stands in the block.
pub(crate) fn frost_walker_disk(level: &mut RegionLevel, origin: BlockPos, radius: i32, pos: [f64; 3]) {
    let origin = kiln_blocks::BlockPos::new(origin.x, origin.y, origin.z);
    let r2 = (radius * radius) as f64;
    for z in -radius..=radius {
        for x in -radius..=radius {
            let at = origin.offset(x, 0, z);
            let (dx, dz) = (at.x as f64 + 0.5 - pos[0], at.z as f64 + 0.5 - pos[2]);
            if dx * dx + dz * dz >= r2 {
                continue;
            }
            if !kiln_blocks::state::is(level.block(at), d::WATER) || !kiln_data::blocks_types::is_air(level.block(at.above())) {
                continue;
            }
            let (min, max) = ([at.x as f64, at.y as f64, at.z as f64], [at.x as f64 + 1.0, at.y as f64 + 1.0, at.z as f64 + 1.0]);
            if level.bodies.iter().any(|b| b.blocks_building && b.intersects(min, max)) {
                continue;
            }
            if kiln_blocks::set_block_and_update(level, at, d::FROSTED_ICE) {
                level.effect(Effect::GameEvent { pos: at, event: "minecraft:block_place" });
            }
        }
    }
}
