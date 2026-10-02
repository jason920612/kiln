//! Parrots (`Parrot`, a `ShoulderRidingEntity`): small flyers tamed with seeds (one in ten) and
//! poisoned (and killed) by cookies, that follow their owner, sit when told and, once their
//! owner stands still enough, hop on the owner's shoulder. Wild or tame they imitate the
//! sounds of hostile mobs nearby, now and then.
//!
//! Flight is a `FlyingMoveControl` (max turn 10, no hovering) with a `FlyingPathNavigation`
//! that floats and opens no doors; falls hurt nothing; the vertical air drag is the
//! horizontal one (`omnidirectionalAirMover`). Jukebox dancing (`setRecordPlayingNearby`) is
//! kept as state only: in vanilla the dance is the client's (the server never calls it).

use super::tame::{self, FollowOwnerGoal, SitWhenOrderedToGoal, Tame, TamableAnimalPanicGoal};
use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::fly;
use crate::mob::goals::{self, Goal, LOOK, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::path::{self, PathType};
use crate::mob::{DamageSource, GroupData, MobData, MobKind, SpawnContext, item_tag};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Parrot;

pub static KIND: Parrot = Parrot;

static INFO: Info = Info {
    sounds: Some("parrot"),
    ..Info::animal("minecraft:parrot", &[(MaxHealth, 6.0), (FlyingSpeed, 0.4000000059604645), (MovementSpeed, 0.20000000298023224), (AttackDamage, 3.0)])
};

/// `Parrot.Variant`s: red/blue, blue, green, yellow/blue, gray.
pub const VARIANTS: i32 = 5;

#[derive(Clone, Debug)]
pub struct State {
    pub tame: Tame,
    /// `Parrot.flap`, `flapSpeed` and their old values, `flapping`, `nextFlap`.
    pub flap: f32,
    pub flap_speed: f32,
    pub o_flap: f32,
    pub o_flap_speed: f32,
    pub flapping: f32,
    pub next_flap: f32,
    /// `partyParrot` and `jukebox` (`setRecordPlayingNearby`).
    pub party: bool,
    pub jukebox: Option<BlockPos>,
    /// `ShoulderRidingEntity.rideCooldownCounter`: ticks since the parrot was made.
    pub ride_cooldown: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("parrot state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("parrot state")
}

/// The Parrot's variant (`getVariant`, clamped like `Variant.byId`).
pub fn variant(m: &MobData) -> i32 {
    m.variant.clamp(0, VARIANTS - 1)
}

/// `isFlying`: not on the ground.
fn is_flying(e: &Entity) -> bool {
    !e.on_ground
}

/// `Entity.isLeashed` (wp32 leashes: wired once leads exist on entities).
fn is_leashed(_e: &Entity) -> bool {
    false
}

/// `Parrot.setRecordPlayingNearby`.
pub fn set_record_playing_nearby(m: &mut MobData, pos: BlockPos, playing: bool) {
    if let Some(s) = ext::state_mut::<State>(m) {
        s.jukebox = Some(pos);
        s.party = playing;
    }
}

/// `Parrot.MOB_SOUND_MAP`: the sound a parrot makes for each type it imitates (in the order
/// the map is filled; the empty sound of the happy ghast is `minecraft:intentionally_empty`).
const IMITATIONS: &[(&str, &str)] = &[
    ("minecraft:blaze", "minecraft:entity.parrot.imitate.blaze"),
    ("minecraft:bogged", "minecraft:entity.parrot.imitate.bogged"),
    ("minecraft:breeze", "minecraft:entity.parrot.imitate.breeze"),
    ("minecraft:camel_husk", "minecraft:entity.parrot.imitate.camel_husk"),
    ("minecraft:cave_spider", "minecraft:entity.parrot.imitate.spider"),
    ("minecraft:creaking", "minecraft:entity.parrot.imitate.creaking"),
    ("minecraft:creeper", "minecraft:entity.parrot.imitate.creeper"),
    ("minecraft:drowned", "minecraft:entity.parrot.imitate.drowned"),
    ("minecraft:elder_guardian", "minecraft:entity.parrot.imitate.elder_guardian"),
    ("minecraft:ender_dragon", "minecraft:entity.parrot.imitate.ender_dragon"),
    ("minecraft:endermite", "minecraft:entity.parrot.imitate.endermite"),
    ("minecraft:evoker", "minecraft:entity.parrot.imitate.evoker"),
    ("minecraft:ghast", "minecraft:entity.parrot.imitate.ghast"),
    ("minecraft:happy_ghast", "minecraft:intentionally_empty"),
    ("minecraft:guardian", "minecraft:entity.parrot.imitate.guardian"),
    ("minecraft:hoglin", "minecraft:entity.parrot.imitate.hoglin"),
    ("minecraft:husk", "minecraft:entity.parrot.imitate.husk"),
    ("minecraft:illusioner", "minecraft:entity.parrot.imitate.illusioner"),
    ("minecraft:magma_cube", "minecraft:entity.parrot.imitate.magma_cube"),
    ("minecraft:parched", "minecraft:entity.parrot.imitate.parched"),
    ("minecraft:phantom", "minecraft:entity.parrot.imitate.phantom"),
    ("minecraft:piglin", "minecraft:entity.parrot.imitate.piglin"),
    ("minecraft:piglin_brute", "minecraft:entity.parrot.imitate.piglin_brute"),
    ("minecraft:pillager", "minecraft:entity.parrot.imitate.pillager"),
    ("minecraft:ravager", "minecraft:entity.parrot.imitate.ravager"),
    ("minecraft:shulker", "minecraft:entity.parrot.imitate.shulker"),
    ("minecraft:silverfish", "minecraft:entity.parrot.imitate.silverfish"),
    ("minecraft:skeleton", "minecraft:entity.parrot.imitate.skeleton"),
    ("minecraft:slime", "minecraft:entity.parrot.imitate.slime"),
    ("minecraft:spider", "minecraft:entity.parrot.imitate.spider"),
    ("minecraft:stray", "minecraft:entity.parrot.imitate.stray"),
    ("minecraft:vex", "minecraft:entity.parrot.imitate.vex"),
    ("minecraft:vindicator", "minecraft:entity.parrot.imitate.vindicator"),
    ("minecraft:warden", "minecraft:entity.parrot.imitate.warden"),
    ("minecraft:witch", "minecraft:entity.parrot.imitate.witch"),
    ("minecraft:wither", "minecraft:entity.parrot.imitate.wither"),
    ("minecraft:wither_skeleton", "minecraft:entity.parrot.imitate.wither_skeleton"),
    ("minecraft:zoglin", "minecraft:entity.parrot.imitate.zoglin"),
    ("minecraft:zombie", "minecraft:entity.parrot.imitate.zombie"),
    ("minecraft:zombie_horse", "minecraft:entity.parrot.imitate.zombie_horse"),
    ("minecraft:zombie_nautilus", "minecraft:entity.parrot.imitate.zombie_nautilus"),
    ("minecraft:zombie_villager", "minecraft:entity.parrot.imitate.zombie_villager"),
];

/// `Parrot.getImitatedSound`: the imitation of `type_name`, else the parrot's own call.
fn imitated_sound(type_name: &str) -> &'static str {
    let s = IMITATIONS.iter().find(|(t, _)| *t == type_name).map_or("minecraft:entity.parrot.ambient", |(_, s)| *s);
    crate::mob::sound_event(s)
}

/// `Parrot.getPitch`: `(nextFloat - nextFloat) * 0.2 + 1`.
pub fn pitch(r: &mut dyn RandomSource) -> f32 {
    let a = r.next_float();
    let b = r.next_float();
    (a - b) * 0.2 + 1.0
}

/// `Parrot.getAmbient(level, random)`: now and then (outside peaceful) the imitation of a
/// random type, else the parrot's call.
pub fn ambient_sound_at(level: &mut dyn EntityLevel) -> &'static str {
    if level.difficulty() != 0 && level.random().next_int_bounded(1000) == 0 {
        let i = level.random().next_int_bounded(IMITATIONS.len() as i32) as usize;
        return imitated_sound(IMITATIONS[i].0);
    }
    crate::mob::sound_event("minecraft:entity.parrot.ambient")
}

/// `Parrot.imitateNearbyMobs(level, entity)`: a mob of an imitated type within 20 blocks of
/// `entity` is heard by a parrot, which calls back with its sound. Whether it did.
pub fn imitate_nearby_mobs(level: &mut dyn EntityLevel, e: &Entity, alive: bool) -> bool {
    if !alive || e.silent || level.random().next_int_bounded(2) != 0 {
        return false;
    }
    let area = e.bounding_box().inflate(20.0, 20.0, 20.0);
    let list: Vec<i32> = level
        .entities_in(&area, EntityFilter::Living, i32::MIN)
        .into_iter()
        .filter(|&id| level.entity(id).is_some_and(|o| crate::mob::data(o).is_some() && IMITATIONS.iter().any(|(t, _)| *t == o.type_name)))
        .collect();
    if list.is_empty() {
        return false;
    }
    let pick = list[level.random().next_int_bounded(list.len() as i32) as usize];
    let Some(o) = level.entity(pick) else { return false };
    if o.silent {
        return false;
    }
    let sound = imitated_sound(o.type_name);
    let pos = e.position();
    let p = pitch(level.random());
    level.emit(Event::Sound { pos, sound, source: "neutral", volume: 0.7, pitch: p });
    true
}

/// `Parrot.checkParrotSpawnRules`.
fn check_spawn(view: &dyn SpawnView, pos: BlockPos) -> bool {
    super::wolf::block_in_tag(view.block(pos.below()), "minecraft:parrots_spawnable_on") && view.raw_brightness(pos, 0) > 8
}

impl Kind for Parrot {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// The constructor: a `FlyingMoveControl`, a floating `FlyingPathNavigation` without doors
    /// and no stepping on fire or cocoa.
    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.fly = true;
        m.nav.can_float = true;
        m.nav.can_open_doors = false;
        tame::set_malus(m, PathType::FireInNeighbor, -1.0);
        tame::set_malus(m, PathType::Fire, -1.0);
        tame::set_malus(m, PathType::Cocoa, -1.0);
        Some(Box::new(State {
            tame: Tame::default(),
            flap: 0.0,
            flap_speed: 0.0,
            o_flap: 0.0,
            o_flap_speed: 0.0,
            flapping: 1.0,
            next_flap: 1.0,
            party: false,
            jukebox: None,
            ride_cooldown: 0,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Custom(Box::new(TamableAnimalPanicGoal::new(1.25, "minecraft:panic_causes"))));
        g.add(0, Goal::Float);
        g.add(1, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(2, Goal::Custom(Box::new(SitWhenOrderedToGoal)));
        g.add(2, Goal::Custom(Box::new(FollowOwnerGoal::new(1.0, 5.0, 1.0))));
        g.add(2, Goal::Custom(Box::new(ParrotWanderGoal { wanted: Vec3::ZERO, speed: 1.0, interval: 120, force: false })));
        g.add(3, Goal::Custom(Box::new(LandOnOwnersShoulderGoal { sitting_on_shoulder: false })));
        g.add(3, Goal::Custom(Box::new(FollowMobGoal { speed: 1.0, stop_distance: 3.0, area_size: 7.0, following: None, recalc: 0, old_water_cost: 0.0 })));
    }

    /// `ShoulderRidingEntity.tick`: the ride cooldown counts up.
    fn pre_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).ride_cooldown += 1;
    }

    /// The first half of `Parrot.aiStep`: the dance ends when the jukebox is gone or far, now
    /// and then the parrot imitates a mob nearby.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let near = st(m).jukebox.is_some_and(|j| {
            let p = e.position();
            let (dx, dy, dz) = (j.x as f64 + 0.5 - p.x, j.y as f64 + 0.5 - p.y, j.z as f64 + 0.5 - p.z);
            dx * dx + dy * dy + dz * dz < 3.46 * 3.46 && crate::blocks::block_name(level.block(j)) == "minecraft:jukebox"
        });
        if !near {
            let s = st_mut(m);
            s.party = false;
            s.jukebox = None;
        }
        if level.random().next_int_bounded(400) == 0 {
            let alive = crate::mob::is_alive(e, m);
            imitate_nearby_mobs(level, e, alive);
        }
    }

    /// The second half of `Parrot.aiStep`: `calculateFlapping`.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let on_ground = e.on_ground;
        let passenger = e.vehicle.is_some();
        let s = st_mut(m);
        s.o_flap = s.flap;
        s.o_flap_speed = s.flap_speed;
        s.flap_speed += (if on_ground || passenger { -1.0f32 } else { 4.0f32 }) * 0.3;
        s.flap_speed = s.flap_speed.clamp(0.0, 1.0);
        if !on_ground && s.flapping < 1.0 {
            s.flapping = 1.0;
        }
        s.flapping *= 0.9;
        if !on_ground && e.delta.y < 0.0 {
            e.delta = e.delta.multiply(1.0, 0.6, 1.0);
        }
        s.flap += s.flapping * 2.0;
    }

    /// `processFlappingMovement` (called from the move in the air): a flap sounds once the
    /// parrot has flown far enough since the last.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let on_pos = e.block_position().below();
        if kiln_data::blocks_types::is_air(level.block(on_pos)) {
            let fly_dist = e.fly_dist;
            let s = st_mut(m);
            if fly_dist > s.next_flap {
                s.next_flap = fly_dist + s.flap_speed / 2.0;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.parrot.fly", source: "neutral", volume: 0.15, pitch: 1.0 });
                }
            }
        }
    }

    /// `FlyingMoveControl(this, 10, false)`.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        fly::tick_move(e, m, 10.0, false);
        true
    }

    fn omnidirectional_air_mover(&self) -> bool {
        true
    }

    fn checks_fall_damage(&self) -> bool {
        false
    }

    /// `Parrot.doPush`: players are not pushed by a parrot, nor push it.
    fn do_push_skips_players(&self) -> bool {
        true
    }

    /// `getVoicePitch`: `Parrot.getPitch` (the two draws the shared pitch took).
    fn voice_pitch(&self, _m: &MobData, pitch: f32) -> f32 {
        pitch
    }

    fn ambient_sound_mut(&self, _e: &mut Entity, _m: &MobData, level: &mut dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(Some(ambient_sound_at(level)))
    }

    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        tame::set_ordered_to_sit(m, false);
        None
    }

    fn can_attack(&self, m: &MobData, level: &dyn EntityLevel, t: &goals::Living) -> bool {
        // `TamableAnimal.canAttack`: never the owner.
        !(t.player && tame::get(m).and_then(|x| x.owner).is_some_and(|u| level.player(t.id).is_some_and(|p| p.uuid == u)))
    }

    fn can_mate(&self, _m: &MobData, _partner: &MobData) -> bool {
        false
    }

    /// `Parrot.finalizeSpawn`: a random variant from the level's random, in a group that never
    /// has babies.
    fn finalize_spawn(&self, _e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        m.variant = r.next_int_bounded(VARIANTS);
        group.ageable_group_size += 1;
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(check_spawn(view, pos))
    }

    /// `Parrot.mobInteract`.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Option<Outcome> {
        let item = if stack.is_empty() { 0 } else { stack.item() };
        let tamed = tame::is_tame(m);
        if !tamed && item > 0 && item_tag(item, "minecraft:parrot_food") {
            if !e.silent {
                let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.parrot.eat", source: "neutral", volume: 1.0, pitch });
            }
            if e.random.next_int_bounded(10) == 0 {
                let uuid = level.player(who.id).map_or(0, |p| p.uuid);
                tame::tame(m, uuid);
                level.emit(Event::EntityEvent { entity: e.id, event: 7 });
                let animal = crate::level::Seen::of_mob(e, m);
                level.emit(Event::Criterion { player: who.id, criterion: crate::level::Criterion::TameAnimal { animal } });
            } else {
                level.emit(Event::EntityEvent { entity: e.id, event: 6 });
            }
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if item > 0 && item_tag(item, "minecraft:parrot_poisonous_food") {
            if let Some(fx) = crate::effect::Effect::named("minecraft:poison", 900, 0) {
                crate::mob::effects::add(e, m, level, fx, None);
            }
            if who.creative || !e.invulnerable {
                let source = DamageSource {
                    kind: crate::level::DamageKind::PlayerAttack,
                    attacker: Some(who.id),
                    direct: Some(who.id),
                    pos: level.player(who.id).map(|p| p.pos),
                    attacker_is_player: true,
                };
                crate::mob::hurt(e, m, level, source, f32::MAX);
            }
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if !is_flying(e) && tamed && tame::owned_by(m, level, who.id) {
            let sit = !tame::ordered_to_sit(m);
            tame::set_ordered_to_sit(m, sit);
            return Some(Outcome::success(HeldChange::None));
        }
        None
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        tame::load(&mut st_mut(m).tame, r);
        m.variant = r.int_or("Variant", 0).clamp(0, VARIANTS - 1);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        tame::save(&st(m).tame, o);
        o.put("Variant", Tag::Int(variant(m)));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        tame::entity_data(&st(m).tame, d);
        d.set(data::parrot::VARIANT, &DataValue::Int(variant(m)));
    }
}

/// `Parrot.ParrotWanderGoal` (a `WaterAvoidingRandomFlyingGoal`): every so often a spot to fly
/// to, now and then a tree top near.
#[derive(Clone, Debug)]
struct ParrotWanderGoal {
    wanted: Vec3,
    speed: f64,
    interval: i32,
    force: bool,
}

impl ParrotWanderGoal {
    /// `ParrotWanderGoal.getPosition`.
    fn position(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Vec3> {
        let mut p = None;
        if e.is_in_water() {
            p = crate::mob::random_pos::land_pos(e, m, level, 15, 15);
        }
        // `probability` (0.001) is the one in `WaterAvoidingRandomStrollGoal`.
        if e.random.next_float() >= 0.001 {
            p = self.tree_pos(e, level);
        }
        match p {
            Some(p) => Some(p),
            None => self.flying_position(e, m, level),
        }
    }

    /// `WaterAvoidingRandomFlyingGoal.getPosition`: toward the look, hovering a few blocks over
    /// the ground, else anywhere in the air.
    fn flying_position(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Vec3> {
        let view = crate::ext_entity::fireball::view_vector(e.x_rot_o, m.y_head_rot_o);
        let angle = std::f32::consts::FRAC_PI_2;
        crate::mob::random_pos::hover_pos(e, m, level, 8, 7, view.x, view.z, angle, 3, 1)
            .or_else(|| crate::mob::random_pos::air_and_water_pos(e, m, level, 8, 4, -2, view.x, view.z, angle as f64))
    }

    /// `getTreePos`: the first free spot (two blocks of air) on top of leaves or a log within
    /// 3 blocks sideways and 6 vertically, other than where the parrot is.
    fn tree_pos(&self, e: &Entity, level: &dyn EntityLevel) -> Option<Vec3> {
        let here = e.block_position();
        let floor = |v: f64| v.floor() as i32;
        let (x0, y0, z0) = (floor(e.x() - 3.0), floor(e.y() - 6.0), floor(e.z() - 3.0));
        let (x1, y1, z1) = (floor(e.x() + 3.0), floor(e.y() + 6.0), floor(e.z() + 3.0));
        // `BlockPos.betweenClosed`: x fastest, then y, then z.
        for z in z0..=z1 {
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let p = BlockPos::new(x, y, z);
                    if p == here {
                        continue;
                    }
                    let below = level.block(p.below());
                    let tree = crate::blocks::block_name(below).ends_with("_leaves") || super::wolf::block_in_tag(below, "minecraft:logs");
                    if tree && kiln_data::blocks_types::is_air(level.block(p)) && kiln_data::blocks_types::is_air(level.block(p.above())) {
                        return Some(Vec3::new(x as f64 + 0.5, y as f64, z as f64 + 0.5));
                    }
                }
            }
        }
        None
    }
}

impl CustomGoal for ParrotWanderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ParrotWanderGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !self.force {
            if m.no_action_time >= 100 {
                return false;
            }
            if e.random.next_int_bounded(reduced_tick_delay(self.interval)) != 0 {
                return false;
            }
        }
        match self.position(e, m, &*level) {
            Some(p) => {
                self.wanted = p;
                self.force = false;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let w = self.wanted;
        path::move_to(e, m, level, w.x, w.y, w.z, self.speed);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.nav.stop();
    }
}

/// `LandOnOwnersShoulderGoal`: a parrot touching its (player) owner, who stands where a
/// parrot can sit, flies onto the owner's shoulder once it has lived 100 ticks.
#[derive(Clone, Debug)]
struct LandOnOwnersShoulderGoal {
    sitting_on_shoulder: bool,
}

/// The owner of parrot `m`, if it is a player in the level.
fn owner_player(m: &MobData, level: &dyn EntityLevel) -> Option<crate::level::PlayerView> {
    tame::owner(m, level)
}

impl CustomGoal for LandOnOwnersShoulderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "LandOnOwnersShoulderGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(p) = owner_player(m, level) else { return false };
        // `!spectator && !flying && !isInWater && !isInPowderSnow`, then not told to sit and
        // `canSitOnShoulder` (alive more than 100 ticks).
        !tame::ordered_to_sit(m) && p.parrot_may_land && st(m).ride_cooldown > 100
    }
    fn interruptable(&self) -> bool {
        !self.sitting_on_shoulder
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.sitting_on_shoulder = false;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.sitting_on_shoulder || tame::get(m).is_some_and(|t| t.sitting) || is_leashed(e) {
            return;
        }
        let Some(p) = owner_player(m, level) else { return };
        if !e.bounding_box().intersects(&crate::level::player_box(&p)) {
            return;
        }
        // `ShoulderRidingEntity.setEntityOnShoulder(player)`: the player takes the saved parrot
        // (when a shoulder is free and it stands, dry, on the ground), and the parrot is gone.
        if !p.parrot_can_sit {
            self.sitting_on_shoulder = false;
            return;
        }
        let saved = std::mem::replace(&mut e.kind, EntityKind::Mob(Box::new(m.clone())));
        let tag = crate::persist::save(e, &|_| None);
        e.kind = saved;
        level.emit(Event::MountShoulder { player: p.id, entity: e.id, tag });
        e.discard();
        self.sitting_on_shoulder = true;
    }
}

/// `FollowMobGoal(parrot, 1.0, 3.0, 7.0)`: follows a visible mob of another class within
/// `area_size`, to within `stop_distance`.
#[derive(Clone, Debug)]
struct FollowMobGoal {
    speed: f64,
    stop_distance: f32,
    area_size: f32,
    following: Option<i32>,
    recalc: i32,
    old_water_cost: f32,
}

impl CustomGoal for FollowMobGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FollowMobGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let a = self.area_size as f64;
        let area = e.bounding_box().inflate(a, a, a);
        for id in level.entities_in(&area, EntityFilter::Living, e.id) {
            let Some(o) = level.entity(id) else { continue };
            // `Mob`s of another class (the types stand for the classes), visible.
            let Some(om) = crate::mob::data(o) else { continue };
            if o.type_name == e.type_name || crate::mob::effects::invisible(om) {
                continue;
            }
            self.following = Some(id);
            return true;
        }
        false
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(o) = self.following.and_then(|id| level.entity(id)) else { return false };
        !m.nav.is_done() && e.position().distance_to_sqr(o.position()) > (self.stop_distance * self.stop_distance) as f64
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.recalc = 0;
        self.old_water_cost = path::malus(m, PathType::Water);
        tame::set_malus(m, PathType::Water, 0.0);
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.following = None;
        m.nav.stop();
        tame::set_malus(m, PathType::Water, self.old_water_cost);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if is_leashed(e) {
            return;
        }
        let Some(o) = self.following.and_then(|id| level.entity(id)) else { return };
        let (ox, oy, oz, eye) = (o.x(), o.y(), o.z(), o.eye_y());
        let max_x = m.max_head_x_rot() as f32;
        m.look.set_look_at(ox, eye, oz, 10.0, max_x);
        self.recalc -= 1;
        if self.recalc > 0 {
            return;
        }
        self.recalc = reduced_tick_delay(10);
        let (dx, dy, dz) = (e.x() - ox, e.y() - oy, e.z() - oz);
        let d = dx * dx + dy * dy + dz * dz;
        let stop = self.stop_distance as f64;
        if d <= (self.stop_distance * self.stop_distance) as f64 {
            m.nav.stop();
            // Closer than the stop distance, or looked at by the followed mob: step away from it.
            let looked_at = level.entity(self.following.unwrap_or(0)).and_then(crate::mob::data).is_some_and(|om| om.look.wanted == [e.x(), e.y(), e.z()]);
            if d <= stop || looked_at {
                let (px, pz) = (ox - e.x(), oz - e.z());
                path::move_to(e, m, level, e.x() - px, e.y(), e.z() - pz, self.speed);
            }
            return;
        }
        let target = BlockPos::containing(ox, oy, oz);
        path::move_to_entity(e, m, level, target, self.speed);
    }
}

/// `Parrot.MOB_SOUND_MAP` keys, for tests.
pub fn imitated_types() -> impl Iterator<Item = &'static str> {
    IMITATIONS.iter().map(|(t, _)| *t)
}

/// Whether `kind` is a parrot.
pub fn is_parrot(kind: MobKind) -> bool {
    kind == MobKind::Parrot
}
