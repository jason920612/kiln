//! What a step sounds like: `Entity.applyMovementEmissionAndPlaySound` and what it calls
//! (`vibrationAndSoundEffectsFromBlock`, `walkingStepSound`, `playStepSound` with the overrides of
//! players and mobs, the combination, muffled and amethyst sounds), and `LivingEntity.playBlockFallSound`.
//!
//! The per-type facts (`getMovementEmission`, the sound `playStepSound` makes on stone, the fall sounds) are
//! read off the vanilla game (`tools/gen_step_sounds.py`); the types whose step depends on the block
//! (horses, camels) or the state of the mob (a strider in lava) are coded here.

use crate::blocks::{Tag, has_tag};
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use kiln_data::block_sounds::sound_type;
use kiln_data::blocks_types::is_air;
use kiln_javamath::random::RandomSource;
use std::collections::HashMap;
use std::sync::OnceLock;

/// `Entity.MovementEmission`: what moving makes (sounds, game events).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emission {
    None,
    Sounds,
    Events,
    All,
}

impl Emission {
    pub fn sounds(self) -> bool {
        matches!(self, Emission::Sounds | Emission::All)
    }

    pub fn events(self) -> bool {
        matches!(self, Emission::Events | Emission::All)
    }

    pub fn anything(self) -> bool {
        self != Emission::None
    }
}

/// What `playStepSound` does for a type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// `Entity.playStepSound`: the block's step sound at 0.15 of its volume.
    Block,
    /// The type overrides it with nothing.
    Silent,
    /// The type plays this sound at this volume, pitch 1.
    Fixed(&'static str, f32),
}

/// One living entity type.
#[derive(Clone, Copy, Debug)]
pub struct StepRow {
    pub type_name: &'static str,
    pub emission: Emission,
    /// `getSoundSource()`.
    pub source: &'static str,
    pub step: Step,
    /// What a baby makes of it, when that is another sound.
    pub baby: Option<(&'static str, f32)>,
    /// `getFallSounds()`: the small and the big one.
    pub fall: (&'static str, &'static str),
}

/// What a step depends on in the mob (its data is out of the entity while it moves: the tick leaves this behind).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepHint {
    pub baby: bool,
    /// A sulfur cube with a swallowed item makes no step.
    pub swallowed: bool,
    /// A copper golem's `WeatherState` (0 unaffected .. 3 oxidized).
    pub weather: u8,
    /// A killer bunny is hostile.
    pub evil: bool,
}

/// The hint of a mob.
pub fn hint_of(m: &crate::mob::MobData) -> StepHint {
    use crate::mob::MobKind;
    let mut h = StepHint { baby: m.baby(), ..StepHint::default() };
    match m.kind {
        MobKind::CopperGolem => h.weather = crate::mob::kinds::copper_golem::st(m).weather,
        MobKind::SulfurCube => h.swallowed = crate::mob::kinds::sulfur_cube::has_body(m),
        MobKind::Rabbit => h.evil = crate::mob::kinds::rabbit::variant(m) == crate::mob::kinds::rabbit::EVIL,
        _ => {}
    }
    h
}

#[path = "gen/step_sounds.rs"]
mod table;

fn rows() -> &'static HashMap<&'static str, &'static StepRow> {
    static ROWS: OnceLock<HashMap<&'static str, &'static StepRow>> = OnceLock::new();
    ROWS.get_or_init(|| table::STEP_ROWS.iter().map(|r| (r.type_name, r)).collect())
}

/// The facts of living entity type `type_name`.
pub fn row(type_name: &str) -> Option<&'static StepRow> {
    rows().get(type_name).copied()
}

/// `getFallSounds()` of a type: (small, big).
pub fn fall_sounds(type_name: &str, monster: bool) -> (&'static str, &'static str) {
    row(type_name).map_or_else(
        || if monster { ("minecraft:entity.hostile.small_fall", "minecraft:entity.hostile.big_fall") } else { ("minecraft:entity.generic.small_fall", "minecraft:entity.generic.big_fall") },
        |r| r.fall,
    )
}

/// `Entity.isStateClimbable`.
fn is_state_climbable(state: u16) -> bool {
    has_tag(state, Tag::Climbable) || crate::blocks::block_name(state) == "minecraft:powder_snow"
}

impl Entity {
    /// `getMovementEmission()`.
    fn movement_emission(&self) -> Emission {
        match &self.kind {
            // `Player.getMovementEmission`: nothing while flying, nor while sneaking on the ground.
            EntityKind::Player(p) => {
                if p.flying || (self.on_ground && self.shift_key_down) { Emission::None } else { Emission::All }
            }
            _ => row(self.type_name).map_or(Emission::All, |r| r.emission),
        }
    }

    /// `getSoundSource()` of what plays the step.
    fn step_source(&self) -> &'static str {
        match &self.kind {
            EntityKind::Player(_) => "player",
            _ if self.step_hint.evil => "hostile",
            _ => row(self.type_name).map_or("neutral", |r| r.source),
        }
    }

    /// `Entity.playSound(sound, volume, pitch)` with the entity's own sound source.
    fn play_own_sound(&mut self, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
        if !self.silent {
            let source = self.step_source();
            level.emit(Event::Sound { pos: self.position(), sound, source, volume, pitch });
        }
    }

    /// `Entity.applyMovementEmissionAndPlaySound` for the move just made (`movement` is what the collision allowed,
    /// `pos` and `state` the block `getOnPosLegacy` finds).
    pub(crate) fn apply_movement_emission(&mut self, level: &mut dyn EntityLevel, movement: Vec3, pos: BlockPos, state: u16) {
        if !self.can_emit_movement() {
            return;
        }
        let emission = self.movement_emission();
        if !emission.anything() {
            return;
        }
        let len = (movement.length() * 0.6000000238418579) as f32;
        let horizontal = (movement.horizontal_distance() * 0.6000000238418579) as f32;
        let on_pos = self.on_pos(level, 1.0e-5);
        let on_state = level.block(on_pos);
        self.move_dist += if is_state_climbable(on_state) { len } else { horizontal };
        self.fly_dist += len;
        if !(self.move_dist > self.next_step) || is_air(on_state) {
            return;
        }
        let same = on_pos == pos;
        let mut done = self.sound_and_vibration(level, pos, state, emission.sounds(), same, movement);
        if !same {
            done |= self.sound_and_vibration(level, on_pos, on_state, false, emission.events(), movement);
        }
        if done {
            self.next_step = self.next_step_after();
        } else if self.is_in_water() {
            self.next_step = self.next_step_after();
            if emission.sounds() {
                self.water_swim_sound(level);
            }
            if emission.events() {
                level.emit(Event::GameEvent { event: "minecraft:swim", pos: self.position(), entity: Some(self.id) });
            }
        }
    }

    /// `isSwimming()`: the swimming pose (not tracked: a swimmer is in water, off the ground).
    fn is_swimming(&self) -> bool {
        false
    }

    /// `nextStep()`: how far the next step is (a turtle, a strider and a warden step more often than once a block).
    fn next_step_after(&self) -> f32 {
        match self.type_name {
            "minecraft:turtle" => self.move_dist + 0.15,
            "minecraft:strider" => self.move_dist + 0.6,
            "minecraft:warden" => self.move_dist + 0.55,
            _ => (self.move_dist as i32 + 1) as f32,
        }
    }

    /// Whether this kind of entity makes steps here at all (the ones the level simulates: players and mobs).
    fn can_emit_movement(&self) -> bool {
        matches!(self.kind, EntityKind::Mob(_) | EntityKind::MobTicking { .. } | EntityKind::Player(_)) && self.vehicle.is_none()
    }

    /// `Entity.vibrationAndSoundEffectsFromBlock`.
    fn sound_and_vibration(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16, play_sound: bool, emit_event: bool, movement: Vec3) -> bool {
        if is_air(state) {
            return false;
        }
        let climbable = is_state_climbable(state);
        let crouching = self.shift_key_down && matches!(self.kind, EntityKind::Player(_));
        if (self.on_ground || climbable || (crouching && movement.y == 0.0)) && !self.is_swimming() {
            if play_sound {
                self.walking_step_sound(level, pos, state);
            }
            if emit_event {
                level.block_game_event("minecraft:step", self.position(), Some(self.id), state);
            }
            return true;
        }
        false
    }

    /// `Entity.walkingStepSound`.
    fn walking_step_sound(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16) {
        self.play_step_sound(level, pos, state);
        if has_tag(state, Tag::CrystalSoundBlocks) && self.tick_count >= self.last_crystal_sound_play_tick + 20 {
            self.play_amethyst_step_sound(level);
        }
    }

    /// `Entity.playAmethystStepSound`: a chime that grows louder the more often one walks on crystal.
    fn play_amethyst_step_sound(&mut self, level: &mut dyn EntityLevel) {
        self.crystal_sound_intensity *= 0.997f64.powf((self.tick_count - self.last_crystal_sound_play_tick) as f64) as f32;
        self.crystal_sound_intensity = (self.crystal_sound_intensity + 0.07).min(1.0);
        let pitch = 0.5 + self.crystal_sound_intensity * self.random.next_float() * 1.2;
        let volume = 0.1 + self.crystal_sound_intensity * 1.2;
        self.play_own_sound(level, "minecraft:block.amethyst_block.chime", volume, pitch);
        self.last_crystal_sound_play_tick = self.tick_count;
    }

    /// `Entity.playStepSound` and its overrides.
    fn play_step_sound(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16) {
        if matches!(self.kind, EntityKind::Player(_)) {
            self.play_player_step_sound(level, pos, state);
            return;
        }
        match self.type_name {
            "minecraft:horse" | "minecraft:donkey" | "minecraft:mule" | "minecraft:skeleton_horse" | "minecraft:zombie_horse" => self.play_horse_step_sound(level, pos, state),
            "minecraft:camel" => {
                let sound = if has_tag(state, Tag::CamelSandStepSoundBlocks) { "minecraft:entity.camel.step_sand" } else { "minecraft:entity.camel.step" };
                self.play_own_sound(level, sound, 1.0, 1.0);
            }
            "minecraft:camel_husk" => {
                let sound = if has_tag(state, Tag::CamelSandStepSoundBlocks) { "minecraft:entity.camel_husk.step_sand" } else { "minecraft:entity.camel_husk.step" };
                self.play_own_sound(level, sound, 0.4, 1.0);
            }
            "minecraft:strider" => {
                let sound = if self.is_in_lava() { "minecraft:entity.strider.step_lava" } else { "minecraft:entity.strider.step" };
                self.play_own_sound(level, sound, 1.0, 1.0);
            }
            "minecraft:copper_golem" => {
                let sound = ["minecraft:entity.copper_golem.step", "minecraft:entity.copper_golem.step", "minecraft:entity.copper_golem_weathered.step", "minecraft:entity.copper_golem_oxidized.step"]
                    [match self.step_hint.weather { 0 | 1 => 0, 2 => 2, _ => 3 }];
                self.play_own_sound(level, sound, 1.0, 1.0);
            }
            "minecraft:sulfur_cube" => {
                if !self.step_hint.swallowed {
                    self.play_block_step_sound(level, state);
                }
            }
            name => {
                let row = row(name);
                match row.map_or(Step::Block, |r| r.step) {
                    Step::Block => self.play_block_step_sound(level, state),
                    Step::Silent => {}
                    Step::Fixed(sound, volume) => {
                        let (sound, volume) = row.and_then(|r| r.baby).filter(|_| self.step_hint.baby).unwrap_or((sound, volume));
                        self.play_own_sound(level, sound, volume, 1.0);
                    }
                }
            }
        }
    }

    /// `Entity.playStepSound`: the block's step sound at 0.15 of its volume.
    fn play_block_step_sound(&mut self, level: &mut dyn EntityLevel, state: u16) {
        let t = sound_type(state);
        self.play_own_sound(level, t.step_sound, t.volume * 0.15, t.pitch);
    }

    /// `Entity.playMuffledStepSound`.
    fn play_muffled_step_sound(&mut self, level: &mut dyn EntityLevel, state: u16) {
        let t = sound_type(state);
        self.play_own_sound(level, t.step_sound, t.volume * 0.05, t.pitch * 0.8);
    }

    /// `Entity.playCombinationStepSounds`: the thin block on top and the one under it, both.
    fn play_combination_step_sounds(&mut self, level: &mut dyn EntityLevel, top: u16, bottom: u16) {
        let t = sound_type(top);
        self.play_own_sound(level, t.step_sound, t.volume * 0.15, t.pitch);
        self.play_muffled_step_sound(level, bottom);
    }

    /// `Player.playStepSound`.
    fn play_player_step_sound(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16) {
        if self.is_in_water() {
            self.water_swim_sound(level);
            self.play_muffled_step_sound(level, state);
            return;
        }
        // `Entity.getPrimaryStepSoundBlockPos`: a carpet or a leaf of snow on top is what one hears.
        let above = pos.above();
        let above_state = level.block(above);
        let primary = if has_tag(above_state, Tag::InsideStepSoundBlocks) || has_tag(above_state, Tag::CombinationStepSoundBlocks) { above } else { pos };
        if pos != primary {
            let s = level.block(primary);
            if has_tag(s, Tag::CombinationStepSoundBlocks) {
                self.play_combination_step_sounds(level, s, state);
            } else {
                self.play_block_step_sound(level, s);
            }
        } else {
            self.play_block_step_sound(level, state);
        }
    }

    /// `AbstractHorse.playStepSound`: wood and the snow on top change the sound; a ridden horse that can gallop
    /// walks on wood for five steps, then gallops every third.
    fn play_horse_step_sound(&mut self, level: &mut dyn EntityLevel, pos: BlockPos, state: u16) {
        if kiln_data::block_logic::fluid(state).kind != kiln_data::block_logic::FluidKind::Empty {
            return;
        }
        let above = level.block(pos.above());
        let t = if crate::blocks::block_name(above) == "minecraft:snow" { sound_type(above) } else { sound_type(state) };
        let wood = ["minecraft:block.wood.step", "minecraft:block.nether_wood.step", "minecraft:block.stem.step", "minecraft:block.cherry_wood.step", "minecraft:block.bamboo_wood.step"];
        let wood = wood.contains(&t.step_sound);
        let can_gallop = !matches!(self.type_name, "minecraft:donkey" | "minecraft:mule");
        if !self.passengers.is_empty() && can_gallop {
            self.gallop_sound_counter += 1;
            if self.gallop_sound_counter > 5 && self.gallop_sound_counter % 3 == 0 {
                self.play_own_sound(level, "minecraft:entity.horse.gallop", t.volume * 0.15, t.pitch);
                // `Horse.playGallopSound`: now and then it snorts.
                if self.type_name == "minecraft:horse" && self.random.next_int_bounded(10) == 0 {
                    let breathe = if self.step_hint.baby { "minecraft:entity.baby_horse.breathe" } else { "minecraft:entity.horse.breathe" };
                    self.play_own_sound(level, breathe, t.volume * 0.6, t.pitch);
                }
            } else if self.gallop_sound_counter <= 5 {
                self.play_own_sound(level, "minecraft:entity.horse.step_wood", t.volume * 0.15, t.pitch);
            }
        } else if wood {
            self.play_own_sound(level, "minecraft:entity.horse.step_wood", t.volume * 0.15, t.pitch);
        } else {
            let sound = if self.step_hint.baby { "minecraft:entity.baby_horse.step" } else { "minecraft:entity.horse.step" };
            self.play_own_sound(level, sound, t.volume * 0.15, t.pitch);
        }
    }

    /// `Entity.waterSwimSound`: the splash of a swimmer, louder the faster it goes.
    fn water_swim_sound(&mut self, level: &mut dyn EntityLevel) {
        let Some(sound) = (match &self.kind {
            EntityKind::Player(_) => Some("minecraft:entity.player.swim"),
            _ => crate::mob::swim_sound_of(self),
        }) else {
            return;
        };
        let d = self.delta;
        let volume = (1.0f32).min(((d.x * d.x * 0.20000000298023224 + d.y * d.y + d.z * d.z * 0.20000000298023224).sqrt() as f32) * 0.35);
        let pitch = 1.0 + (self.random.next_float() - self.random.next_float()) * 0.4;
        self.play_own_sound(level, sound, volume, pitch);
    }

    /// `LivingEntity.playBlockFallSound`: the block at the feet lands with a sound of its own.
    pub fn play_block_fall_sound(&mut self, level: &mut dyn EntityLevel) {
        if self.silent {
            return;
        }
        let p = self.position();
        let at = BlockPos::new(crate::math::floor(p.x), crate::math::floor(p.y - 0.20000000298023224), crate::math::floor(p.z));
        let state = level.block(at);
        if is_air(state) {
            return;
        }
        let t = sound_type(state);
        self.play_own_sound(level, t.fall_sound, t.volume * 0.5, t.pitch * 0.75);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_knows_the_living_types() {
        let pig = row("minecraft:pig").expect("pig");
        assert_eq!(pig.step, Step::Fixed("minecraft:entity.pig.step", 0.15));
        assert_eq!(pig.emission, Emission::All);
        assert_eq!(row("minecraft:creeper").unwrap().step, Step::Block);
        assert_eq!(row("minecraft:bee").unwrap().step, Step::Silent);
        assert_eq!(row("minecraft:shulker").unwrap().emission, Emission::None);
        assert_eq!(row("minecraft:warden").unwrap().step, Step::Fixed("minecraft:entity.warden.step", 10.0));
        assert_eq!(fall_sounds("minecraft:zombie", true).1, "minecraft:entity.hostile.big_fall");
    }
}
