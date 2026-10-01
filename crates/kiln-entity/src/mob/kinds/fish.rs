//! Fish (`WaterAnimal` → `AbstractFish`): cod, salmon (three sizes), tropical fish (patterns and
//! colors) and pufferfish. They swim with `WaterBoundPathNavigation` and the fish move control,
//! panic when hurt, keep away from players, flop about on land and drown there. Cod, salmon and
//! tropical fish school (`AbstractSchoolingFish`, `FollowFlockLeaderGoal`); a pufferfish puffs up
//! near anything scary and stings what touches it with poison. A water bucket scoops a fish up
//! (`Bucketable`); [`apply_bucket`] releases one.

use crate::custom_goal_boilerplate;
use crate::entity::{Entity, MoverType};
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{Aabb, BlockPos, Vec3};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::control::{self, Operation};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, Placement, SpawnView};
use crate::mob::goals::{self, Goal, Living, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, Category, DamageSource, GroupData, MobData, MobKind, SpawnContext, mth, path};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FishType {
    Cod,
    Salmon,
    TropicalFish,
    Pufferfish,
}

pub struct Fish(pub FishType);

pub static COD: Fish = Fish(FishType::Cod);
pub static SALMON: Fish = Fish(FishType::Salmon);
pub static TROPICAL_FISH: Fish = Fish(FishType::TropicalFish);
pub static PUFFERFISH: Fish = Fish(FishType::Pufferfish);

/// `AbstractFish.createAttributes` (`Mob.createMobAttributes` with 3 health).
const fn info(name: &'static str) -> Info {
    Info {
        category: Category::WaterAmbient,
        breathes_under_water: true,
        ambient_interval: 120,
        ..Info::misc(name, &[(MaxHealth, 3.0)])
    }
}

static COD_INFO: Info = info("minecraft:cod");
static SALMON_INFO: Info = Info { sounds: Some("salmon"), ..info("minecraft:salmon") };
static TROPICAL_INFO: Info = info("minecraft:tropical_fish");
static PUFFER_INFO: Info = Info { sounds: Some("puffer_fish"), ..info("minecraft:pufferfish") };

/// Salmon sizes (`Salmon.Variant`): name and bounding box scale.
pub const SALMON_SIZES: [(&str, f32); 3] = [("small", 0.5), ("medium", 1.0), ("large", 1.5)];
const SALMON_MEDIUM: i32 = 1;

/// `TropicalFish.Pattern` packed ids in `values()` order.
pub const PATTERNS: [i32; 12] = [0, 256, 512, 768, 1024, 1280, 1, 257, 513, 769, 1025, 1281];

/// `TropicalFish.COMMON_VARIANTS` as (pattern, base color, pattern color).
const COMMON_VARIANTS: [(i32, i32, i32); 22] = {
    const KOB: i32 = 0;
    const SUNSTREAK: i32 = 256;
    const SNOOPER: i32 = 512;
    const DASHER: i32 = 768;
    const BRINELY: i32 = 1024;
    const SPOTTY: i32 = 1280;
    const FLOPPER: i32 = 1;
    const STRIPEY: i32 = 257;
    const GLITTER: i32 = 513;
    const BLOCKFISH: i32 = 769;
    const BETTY: i32 = 1025;
    const CLAYFISH: i32 = 1281;
    const WHITE: i32 = 0;
    const ORANGE: i32 = 1;
    const LIGHT_BLUE: i32 = 3;
    const YELLOW: i32 = 4;
    const LIME: i32 = 5;
    const PINK: i32 = 6;
    const GRAY: i32 = 7;
    const CYAN: i32 = 9;
    const PURPLE: i32 = 10;
    const BLUE: i32 = 11;
    const RED: i32 = 14;
    [
        (STRIPEY, ORANGE, GRAY),
        (FLOPPER, GRAY, GRAY),
        (FLOPPER, GRAY, BLUE),
        (CLAYFISH, WHITE, GRAY),
        (SUNSTREAK, BLUE, GRAY),
        (KOB, ORANGE, WHITE),
        (SPOTTY, PINK, LIGHT_BLUE),
        (BLOCKFISH, PURPLE, YELLOW),
        (CLAYFISH, WHITE, RED),
        (SPOTTY, WHITE, YELLOW),
        (GLITTER, WHITE, GRAY),
        (CLAYFISH, WHITE, ORANGE),
        (DASHER, CYAN, PINK),
        (BRINELY, LIME, LIGHT_BLUE),
        (BETTY, RED, WHITE),
        (SNOOPER, GRAY, RED),
        (BLOCKFISH, RED, WHITE),
        (FLOPPER, WHITE, YELLOW),
        (KOB, RED, WHITE),
        (SUNSTREAK, GRAY, WHITE),
        (DASHER, CYAN, YELLOW),
        (FLOPPER, YELLOW, YELLOW),
    ]
};

/// `TropicalFish.packVariant`.
pub fn pack_variant(pattern: i32, base: i32, pattern_color: i32) -> i32 {
    (pattern & 0xFFFF) | ((base & 0xFF) << 16) | ((pattern_color & 0xFF) << 24)
}

#[derive(Clone, Debug)]
pub struct FishState {
    /// `AbstractFish.FROM_BUCKET`.
    pub from_bucket: bool,
    /// `AbstractSchoolingFish.leader` and `schoolSize`.
    pub leader: Option<i32>,
    pub school_size: i32,
    /// The salmon's size id, the tropical fish's packed variant.
    pub variant: i32,
    /// `TropicalFish.isSchool`.
    pub is_school: bool,
    /// `Pufferfish.PUFF_STATE`, `inflateCounter`, `deflateTimer`.
    pub puff: i32,
    inflate_counter: i32,
    deflate_timer: i32,
    /// `FollowFlockLeaderGoal`'s first `nextStartTick` (drawn in the constructor).
    flock_start: i32,
}

fn st(m: &MobData) -> &FishState {
    ext::state::<FishState>(m).expect("fish state")
}

fn st_mut(m: &mut MobData) -> &mut FishState {
    ext::state_mut::<FishState>(m).expect("fish state")
}

fn state_of(level: &dyn EntityLevel, id: i32) -> Option<&FishState> {
    level.entity(id).and_then(mob::data).and_then(ext::state::<FishState>)
}

/// `f` on the fish state of `id`: the ticking fish's own (`me`), or another fish of the level.
fn with_fish<R>(me: i32, m: &mut MobData, level: &mut dyn EntityLevel, id: i32, f: impl FnOnce(&mut FishState) -> R) -> Option<R> {
    if id == me {
        return Some(f(st_mut(m)));
    }
    let o = level.entity_mut(id)?;
    let om = mob::data_mut(o)?;
    ext::state_mut::<FishState>(om).map(f)
}

impl Fish {
    fn schooling(&self) -> bool {
        self.0 != FishType::Pufferfish
    }

    fn bucket_item(&self) -> &'static str {
        match self.0 {
            FishType::Cod => "minecraft:cod_bucket",
            FishType::Salmon => "minecraft:salmon_bucket",
            FishType::TropicalFish => "minecraft:tropical_fish_bucket",
            FishType::Pufferfish => "minecraft:pufferfish_bucket",
        }
    }

    fn flop_sound(&self) -> &'static str {
        match self.0 {
            FishType::Cod => "minecraft:entity.cod.flop",
            FishType::Salmon => "minecraft:entity.salmon.flop",
            FishType::TropicalFish => "minecraft:entity.tropical_fish.flop",
            FishType::Pufferfish => "minecraft:entity.puffer_fish.flop",
        }
    }
}

/// `AbstractSchoolingFish.getMaxSchoolSize`.
fn max_school_size(kind: MobKind) -> i32 {
    if kind == MobKind::Salmon { 5 } else { 8 }
}

/// `isFollower`: a leader that is still alive.
fn is_follower(s: &FishState, level: &dyn EntityLevel) -> bool {
    s.leader.is_some_and(|id| leader_alive(level, id))
}

/// `isFollower` of another fish while fish `me` (alive or not) ticks outside the level.
fn is_follower_near(s: &FishState, level: &dyn EntityLevel, me: (i32, bool)) -> bool {
    s.leader.is_some_and(|id| if id == me.0 { me.1 } else { leader_alive(level, id) })
}

fn leader_alive(level: &dyn EntityLevel, id: i32) -> bool {
    level.entity(id).is_some_and(|o| o.is_alive() && mob::data(o).is_some_and(|om| om.health > 0.0))
}

/// `canBeFollowed`.
fn can_be_followed(s: &FishState, kind: MobKind) -> bool {
    s.school_size > 1 && s.school_size < max_school_size(kind)
}

impl Kind for Fish {
    fn info(&self) -> &'static Info {
        match self.0 {
            FishType::Cod => &COD_INFO,
            FishType::Salmon => &SALMON_INFO,
            FishType::TropicalFish => &TROPICAL_INFO,
            FishType::Pufferfish => &PUFFER_INFO,
        }
    }

    /// `WaterAnimal` and `AbstractFish` constructors: water costs nothing, water-bound navigation.
    fn new_state(&self, m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.maluses.push((path::PathType::Water, 0.0));
        m.nav.water_bound = true;
        let variant = if self.0 == FishType::Salmon { SALMON_MEDIUM } else { 0 };
        // `Mob`'s constructor registers the goals: `FollowFlockLeaderGoal` draws its first delay.
        let flock_start = if self.schooling() { FollowFlockLeaderGoal::next_start_tick(random) } else { 0 };
        Some(Box::new(FishState {
            from_bucket: false,
            leader: None,
            school_size: 1,
            variant,
            is_school: true,
            puff: 0,
            inflate_counter: 0,
            deflate_timer: 0,
            flock_start,
        }))
    }

    /// `AbstractFish.registerGoals` (with `AbstractSchoolingFish`'s and `Pufferfish`'s additions).
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Panic { speed: 1.25, pos: Vec3::ZERO });
        g.add(2, Goal::Custom(Box::new(AvoidPlayerGoal::new("AvoidEntityGoal", 8.0, 1.6, 1.4, None))));
        g.add(4, Goal::Custom(Box::new(RandomSwimmingGoal { name: "FishSwimGoal", speed: 1.0, interval: 40, wanted: Vec3::ZERO })));
        if self.schooling() {
            let next_start = st(m).flock_start;
            m.goals.add(5, Goal::Custom(Box::new(FollowFlockLeaderGoal { recalc: 0, next_start })));
        } else {
            g.add(1, Goal::Custom(Box::new(PufferfishPuffGoal)));
        }
    }

    /// `Pufferfish.tick` before `Mob.tick`: puffing up while scared, then down again.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.0 != FishType::Pufferfish || !mob::is_alive(e, m) || m.no_ai {
            return;
        }
        let s = st(m);
        let (puff, inflate, deflate) = (s.puff, s.inflate_counter, s.deflate_timer);
        if inflate > 0 {
            if puff == 0 {
                mob::make_sound(e, m, level, "minecraft:entity.puffer_fish.blow_up");
                puff_to(e, m, &*level, 1);
            } else if inflate > 40 && puff == 1 {
                mob::make_sound(e, m, level, "minecraft:entity.puffer_fish.blow_up");
                puff_to(e, m, &*level, 2);
            }
            st_mut(m).inflate_counter += 1;
        } else if puff != 0 {
            if deflate > 60 && puff == 2 {
                mob::make_sound(e, m, level, "minecraft:entity.puffer_fish.blow_out");
                puff_to(e, m, &*level, 1);
            } else if deflate > 100 && puff == 1 {
                mob::make_sound(e, m, level, "minecraft:entity.puffer_fish.blow_out");
                puff_to(e, m, &*level, 0);
            }
            st_mut(m).deflate_timer += 1;
        }
    }

    /// `AbstractSchoolingFish.tick` after `Mob.tick`: a leader left alone forgets its school.
    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !self.schooling() || st(m).school_size <= 1 || level.random().next_int_bounded(200) != 1 {
            return;
        }
        let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
        let kind = m.kind;
        let others = level
            .entities_in(&area, crate::level::EntityFilter::Living, e.id)
            .into_iter()
            .filter(|&id| level.entity(id).and_then(mob::data).is_some_and(|om| om.kind == kind))
            .count();
        if others == 0 {
            st_mut(m).school_size = 1;
        }
    }

    /// `AbstractFish.aiStep` before `LivingEntity.aiStep`: flopping on land.
    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !e.is_in_water() && e.on_ground && e.vertical_collision {
            let x = (e.random.next_float() * 2.0 - 1.0) * 0.05;
            let z = (e.random.next_float() * 2.0 - 1.0) * 0.05;
            e.delta = e.delta.add(x as f64, 0.4f32 as f64, z as f64);
            e.set_on_ground(&*level, false);
            e.needs_sync = true;
            mob::make_sound(e, m, level, self.flop_sound());
        }
    }

    /// `Pufferfish.aiStep` after `Mob.aiStep`: stinging the mobs it touches while puffed.
    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let puff = st(m).puff;
        if self.0 != FishType::Pufferfish || !mob::is_alive(e, m) || puff <= 0 {
            return;
        }
        let area = e.bounding_box().inflate_all(0.3);
        for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
            if level.player(id).is_some() {
                continue;
            }
            let Some(t) = goals::living(level, id) else { continue };
            if !scary(level, &t) || !t.alive {
                continue;
            }
            let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
            if mob::hurt_living(level, &t, source, (1 + puff) as f32) {
                level.add_effect(id, "minecraft:poison", 60 * puff, 0, Some(e.id));
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.puffer_fish.sting", source: "neutral", volume: 1.0, pitch: 1.0 });
                }
            }
        }
    }

    /// `Pufferfish.playerTouch`: a puffed-up pufferfish stings the player with poison.
    fn player_touch(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, player: &Living) {
        let puff = st(m).puff;
        if self.0 != FishType::Pufferfish || puff <= 0 {
            return;
        }
        let source = DamageSource { kind: DamageKind::MobAttack, attacker: Some(e.id), direct: Some(e.id), pos: Some(e.position()), attacker_is_player: false };
        if level.hurt_player(player.id, source, (1 + puff) as f32) {
            level.add_effect(player.id, "minecraft:poison", 60 * puff, 0, Some(e.id));
        }
    }

    /// `FishMoveControl.tick`: rising while under water, steering toward the wanted position.
    fn tick_move(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if e.fluid.is_eye_in_water() {
            e.delta = e.delta.add(0.0, 0.005, 0.0);
        }
        if m.mov.operation == Operation::MoveTo && !m.nav.is_done() {
            let target = (m.mov.speed_modifier * m.attrs.value(Attr::MovementSpeed)) as f32;
            let speed = mth::lerp_f(0.125, m.speed, target);
            control::set_speed(m, speed);
            let [wx, wy, wz] = m.mov.wanted;
            let (xd, yd, zd) = (wx - e.x(), wy - e.y(), wz - e.z());
            if yd != 0.0 {
                let dd = (xd * xd + yd * yd + zd * zd).sqrt();
                e.delta = e.delta.add(0.0, m.speed as f64 * (yd / dd) * 0.1, 0.0);
            }
            if xd != 0.0 || zd != 0.0 {
                let yaw = (mth::atan2(zd, xd) * 57.2957763671875) as f32 - 90.0;
                e.y_rot = control::rotlerp(e.y_rot, yaw, 90.0);
                m.y_body_rot = e.y_rot;
            }
        } else {
            control::set_speed(m, 0.0);
        }
        true
    }

    /// `AbstractFish.travelInWater`: slow, free of the water's drag, sinking without a target.
    fn travel(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, input: Vec3) -> bool {
        if !e.is_in_water() {
            return false;
        }
        mob::move_relative(e, 0.01, input);
        let d = e.delta;
        e.do_move(level, MoverType::SelfMove, d);
        e.delta = e.delta.scale(0.9);
        if m.target.is_none() {
            e.delta = e.delta.add(0.0, -0.005, 0.0);
        }
        true
    }

    fn after_base_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, air_before: i32) {
        ext::water_animal_air(e, m, level, air_before);
    }

    fn pushed_by_fluid(&self) -> bool {
        false
    }

    fn swim_sound(&self) -> Option<&'static str> {
        Some("minecraft:entity.fish.swim")
    }

    fn experience(&self, e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(1 + e.random.next_int_bounded(3))
    }

    fn walk_target_value(&self, _m: &MobData, _level: &dyn EntityLevel, _p: BlockPos) -> Option<f32> {
        Some(0.0)
    }

    fn spawn_ignores_light(&self) -> bool {
        true
    }

    fn placement(&self) -> Placement {
        Placement::InWater
    }

    fn spawn_in_liquids(&self) -> bool {
        true
    }

    fn max_spawn_cluster(&self) -> i32 {
        if self.0 == FishType::Salmon { 5 } else { 8 }
    }

    /// `removeWhenFarAway`: not once it came out of a bucket (or has a name).
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        Some(!st(m).from_bucket)
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        if self.0 == FishType::TropicalFish {
            let water = |p: BlockPos| crate::physics::fluid_state(view.block(p)).kind.is_water();
            let ok = water(pos.below())
                && crate::blocks::block_name(view.block(pos.above())) == "minecraft:water"
                && (mob::species::biome_tag(view.biome(pos), "minecraft:allows_tropical_fish_spawns_at_any_height")
                    || super::squid::surface_water_rules(view, pos));
            return Some(ok);
        }
        Some(super::squid::surface_water_rules(view, pos))
    }

    /// `Salmon`/`TropicalFish.finalizeSpawn` around `Mob.finalizeSpawn`.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        if self.0 == FishType::Salmon {
            // A weighted size from the salmon's own random: small 30, medium 50, large 15.
            let i = e.random.next_int_bounded(95);
            let size = if i < 30 {
                0
            } else if i < 80 {
                1
            } else {
                2
            };
            st_mut(m).variant = size;
            mob::refresh_dimensions(e, m);
        }
        ext::mob_finalize(m, r);
        if self.0 == FishType::TropicalFish {
            let variant = if let Some(v) = group.variant {
                v
            } else if r.next_float() < 0.9 {
                let (p, b, c) = COMMON_VARIANTS[r.next_int_bounded(COMMON_VARIANTS.len() as i32) as usize];
                let v = pack_variant(p, b, c);
                group.variant = Some(v);
                v
            } else {
                st_mut(m).is_school = false;
                let p = PATTERNS[r.next_int_bounded(12) as usize];
                let b = r.next_int_bounded(16);
                let c = r.next_int_bounded(16);
                pack_variant(p, b, c)
            };
            st_mut(m).variant = variant;
        }
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        bucket_pickup(e, m, level, stack, self.bucket_item(), "minecraft:item.bucket.fill_fish")
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        let scale = match self.0 {
            FishType::Salmon => SALMON_SIZES[st(m).variant.clamp(0, 2) as usize].1,
            // `Pufferfish.getScale`.
            FishType::Pufferfish => match st(m).puff {
                0 => 0.5,
                1 => 0.7,
                _ => 1.0,
            },
            _ => 1.0,
        };
        (base.0 * scale, base.1 * scale, base.2 * scale)
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let from_bucket = r.bool_or("FromBucket", false);
        let s = st_mut(m);
        s.from_bucket = from_bucket;
        match self.0 {
            FishType::Salmon => {
                let name = r.get("type").and_then(Tag::as_str);
                s.variant = name.and_then(|n| SALMON_SIZES.iter().position(|(x, _)| *x == n)).map_or(SALMON_MEDIUM, |i| i as i32);
            }
            FishType::TropicalFish => s.variant = r.int_or("Variant", 0),
            FishType::Pufferfish => {
                let p = r.int_or("PuffState", 0).min(2);
                set_puff(e, m, p);
            }
            FishType::Cod => {}
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("FromBucket", Tag::Byte(s.from_bucket as i8));
        match self.0 {
            FishType::Salmon => o.put("type", Tag::String(SALMON_SIZES[s.variant.clamp(0, 2) as usize].0.into())),
            FishType::TropicalFish => o.put("Variant", Tag::Int(s.variant)),
            FishType::Pufferfish => o.put("PuffState", Tag::Int(s.puff)),
            FishType::Cod => {}
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::abstract_fish::FROM_BUCKET, &DataValue::Boolean(s.from_bucket));
        match self.0 {
            FishType::Salmon => {
                d.set(data::salmon::TYPE, &DataValue::Int(s.variant));
            }
            FishType::TropicalFish => {
                d.set(data::tropical_fish::ID_TYPE_VARIANT, &DataValue::Int(s.variant));
            }
            FishType::Pufferfish => {
                d.set(data::pufferfish::PUFF_STATE, &DataValue::Int(s.puff));
            }
            FishType::Cod => {}
        }
    }
}

/// `setPuffState` (the size follows).
fn set_puff(e: &mut Entity, m: &mut MobData, puff: i32) {
    st_mut(m).puff = puff;
    mob::refresh_dimensions(e, m);
}

/// `setPuffState` in the pufferfish's tick: growing pushes it to a free spot.
fn puff_to(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel, puff: i32) {
    st_mut(m).puff = puff;
    mob::refresh_dimensions_in(e, m, level);
}

/// `Pufferfish.SCARY_MOB` with the non-combat conditions: anything but creative players and
/// the `not_scary_for_pufferfish` types.
fn scary(_level: &dyn EntityLevel, t: &Living) -> bool {
    if t.player {
        return !t.creative && !t.spectator && t.alive;
    }
    t.alive && !mob::entity_type_tag(t.type_name, "minecraft:not_scary_for_pufferfish")
}

// ---------------------------------------------------------------------- buckets

/// `Bucketable.bucketMobPickup`: a water bucket takes the mob (its health and flags in the
/// bucket's `bucket_entity_data`, the variant as the bucket's components).
pub fn bucket_pickup(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, stack: &ItemStack, bucket: &str, sound: &'static str) -> Option<Outcome> {
    if stack.is_empty() || mob::item_name(stack) != "minecraft:water_bucket" || !mob::is_alive(e, m) {
        return None;
    }
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: m.kind.sound_source(), volume: 1.0, pitch: 1.0 });
    }
    let mut filled = ItemStack::of(bucket, 1)?;
    save_to_bucket(e, m, &mut filled);
    e.discard();
    Some(Outcome::success(HeldChange::Fill(filled)))
}

/// `saveToBucketTag`: `Bucketable.saveDefaultDataToBucketTag` and the type's components.
pub fn save_to_bucket(e: &Entity, m: &MobData, bucket: &mut ItemStack) {
    let mut tag: Vec<(String, Tag)> = Vec::new();
    if m.no_ai {
        tag.push(("NoAI".into(), Tag::Byte(1)));
    }
    if e.silent {
        tag.push(("Silent".into(), Tag::Byte(1)));
    }
    if e.no_gravity {
        tag.push(("NoGravity".into(), Tag::Byte(1)));
    }
    if e.invulnerable {
        tag.push(("Invulnerable".into(), Tag::Byte(1)));
    }
    if m.persistence_required {
        tag.push(("PersistenceRequired".into(), Tag::Byte(1)));
    }
    tag.push(("Health".into(), Tag::Float(m.health)));
    bucket.insert(kiln_item::keys::BUCKET_ENTITY_DATA, kiln_item::component::CustomData(Tag::Compound(tag)));
    let Some(s) = ext::state::<FishState>(m) else { return };
    use kiln_item::component::DyeColor;
    use kiln_item::component::variant::{SalmonSize, TropicalFishPattern};
    match m.kind {
        MobKind::Salmon => {
            if let Some(size) = SalmonSize::ALL.get(s.variant.clamp(0, 2) as usize) {
                bucket.insert(kiln_item::keys::SALMON_SIZE, *size);
            }
        }
        MobKind::TropicalFish => {
            let v = s.variant;
            if let Some(p) = TropicalFishPattern::ALL.iter().find(|p| p.id() == v & 0xFFFF) {
                bucket.insert(kiln_item::keys::TROPICAL_FISH_PATTERN, *p);
            }
            if let Some(c) = DyeColor::from_id((v >> 16) & 0xFF) {
                bucket.insert(kiln_item::keys::TROPICAL_FISH_BASE_COLOR, c);
            }
            if let Some(c) = DyeColor::from_id((v >> 24) & 0xFF) {
                bucket.insert(kiln_item::keys::TROPICAL_FISH_PATTERN_COLOR, c);
            }
        }
        _ => {}
    }
}

/// The mob type a mob bucket item releases (`MobBucketItem.type`).
pub fn bucket_mob(item: &str) -> Option<MobKind> {
    Some(match item {
        "minecraft:cod_bucket" => MobKind::Cod,
        "minecraft:salmon_bucket" => MobKind::Salmon,
        "minecraft:tropical_fish_bucket" => MobKind::TropicalFish,
        "minecraft:pufferfish_bucket" => MobKind::Pufferfish,
        "minecraft:tadpole_bucket" => MobKind::Tadpole,
        _ => return None,
    })
}

/// `MobBucketItem.spawn` on a mob just made and finalized: the bucket's components (salmon size,
/// tropical fish pattern and colors), then `loadFromBucketTag` and `setFromBucket(true)`.
pub fn apply_bucket(e: &mut Entity, bucket: &ItemStack) {
    let Some(m) = mob::data_mut(e) else { return };
    let tropical = m.kind == MobKind::TropicalFish;
    if let Some(s) = ext::state_mut::<FishState>(m) {
        if let Some(size) = bucket.get(kiln_item::keys::SALMON_SIZE) {
            s.variant = size.id();
        }
        let mut v = s.variant;
        if let Some(p) = bucket.get(kiln_item::keys::TROPICAL_FISH_PATTERN) {
            v = pack_variant(p.id(), (v >> 16) & 0xFF, (v >> 24) & 0xFF);
        }
        if let Some(c) = bucket.get(kiln_item::keys::TROPICAL_FISH_BASE_COLOR) {
            v = pack_variant(v & 0xFFFF, c.id(), (v >> 24) & 0xFF);
        }
        if let Some(c) = bucket.get(kiln_item::keys::TROPICAL_FISH_PATTERN_COLOR) {
            v = pack_variant(v & 0xFFFF, (v >> 16) & 0xFF, c.id());
        }
        if tropical {
            s.variant = v;
        }
        s.from_bucket = true;
    }
    let tag = bucket.get(kiln_item::keys::BUCKET_ENTITY_DATA).map(|c| c.0.clone());
    if let Some(Tag::Compound(fields)) = tag {
        for (k, v) in &fields {
            let on = v.as_f64().is_some_and(|b| b != 0.0);
            match k.as_str() {
                "NoAI" => m.no_ai = on,
                "PersistenceRequired" if on => m.persistence_required = true,
                "Health" => {
                    if let Some(h) = v.as_f64() {
                        m.set_health(h as f32);
                    }
                }
                _ => {}
            }
        }
        for (k, v) in &fields {
            let on = v.as_f64().is_some_and(|b| b != 0.0);
            match k.as_str() {
                "Silent" => e.silent = on,
                "NoGravity" => e.no_gravity = on,
                "Invulnerable" => e.invulnerable = on,
                _ => {}
            }
        }
    }
    if let Some(m) = mob::data(e).cloned() {
        mob::refresh_dimensions(e, &m);
    }
}

// ---------------------------------------------------------------------- goals

/// `RandomSwimmingGoal` (a `RandomStrollGoal` whose target is `getRandomSwimmablePos(10, 7)`);
/// as `AbstractFish.FishSwimGoal` only for fish that do not follow a leader.
#[derive(Clone, Debug)]
pub struct RandomSwimmingGoal {
    pub name: &'static str,
    pub speed: f64,
    pub interval: i32,
    pub wanted: Vec3,
}

impl CustomGoal for RandomSwimmingGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        // `FishSwimGoal.canUse`: `canRandomSwim` first (not for followers).
        if self.name == "FishSwimGoal" && ext::state::<FishState>(m).is_some_and(|s| m.kind != MobKind::Pufferfish && is_follower(s, level)) {
            return false;
        }
        if m.no_action_time >= 100 {
            return false;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(self.interval)) != 0 {
            return false;
        }
        match path::random_swimmable_pos(e, m, level, 10, 7) {
            Some(p) => {
                self.wanted = p;
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

/// `AvoidEntityGoal<Player>`: runs from the nearest player it could fight within `max_dist`
/// (sprinting within 7 blocks). `unless`: a state that keeps the goal from starting
/// (`OcelotAvoidEntityGoal`: trusting), which also means `NO_CREATIVE_OR_SPECTATOR` players.
#[derive(Clone, Debug)]
pub struct AvoidPlayerGoal {
    pub name: &'static str,
    pub max_dist: f32,
    pub walk: f64,
    pub sprint: f64,
    pub unless: Option<fn(&MobData) -> bool>,
    to_avoid: Option<i32>,
    path: Option<path::Path>,
}

impl AvoidPlayerGoal {
    pub fn new(name: &'static str, max_dist: f32, walk: f64, sprint: f64, unless: Option<fn(&MobData) -> bool>) -> AvoidPlayerGoal {
        AvoidPlayerGoal { name, max_dist, walk, sprint, unless, to_avoid: None, path: None }
    }
}

impl CustomGoal for AvoidPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        self.name
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.unless.is_some_and(|f| f(m)) {
            return false;
        }
        let d = self.max_dist as f64;
        let area = e.bounding_box().inflate(d, 3.0, d);
        let mut best: Option<(f64, Living)> = None;
        for p in level.players_in(&area).iter() {
            let h = if p.sneaking { 1.5 } else { 1.8 };
            let pb = Aabb::new(p.pos.x - 0.3, p.pos.y, p.pos.z - 0.3, p.pos.x + 0.3, p.pos.y + h, p.pos.z + 0.3);
            if !pb.intersects(&area) || p.spectator {
                continue;
            }
            // `EntitySelector.NO_CREATIVE_OR_SPECTATOR` (ocelots) or `NO_SPECTATORS` (fish).
            if self.unless.is_some() && p.creative {
                continue;
            }
            let t = goals::living_player(p);
            if !goals::targeting_ok(e, m, level, &t, true, d, true) {
                continue;
            }
            let dist = e.position().distance_to_sqr(p.pos);
            if best.as_ref().is_none_or(|(b, _)| dist < *b) {
                best = Some((dist, t));
            }
        }
        let Some((_, t)) = best else { return false };
        self.to_avoid = Some(t.id);
        let Some(away) = crate::mob::random_pos::default_pos_away(e, m, level, 16, 7, t.pos) else { return false };
        if t.pos.distance_to_sqr(away) < t.pos.distance_to_sqr(e.position()) {
            return false;
        }
        self.path = path::create_path(e, m, level, BlockPos::containing(away.x, away.y, away.z), 0);
        self.path.is_some()
    }
    fn can_continue(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !m.nav.is_done()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let p = self.path.take();
        path::move_to_path(e, m, level, p, self.walk);
    }
    fn stop(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.to_avoid = None;
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = self.to_avoid.and_then(|id| goals::living(level, id)) else { return };
        m.nav.speed_modifier = if e.position().distance_to_sqr(t.pos) < 49.0 { self.sprint } else { self.walk };
    }
}

/// `FollowFlockLeaderGoal`: a fish without a school looks for one every 10 to 20 seconds
/// (joining a leader with room, or leading the loners about), then keeps within 11 blocks of
/// its leader.
#[derive(Clone, Debug)]
struct FollowFlockLeaderGoal {
    recalc: i32,
    /// `nextStartTick`.
    next_start: i32,
}

impl FollowFlockLeaderGoal {
    fn next_start_tick(r: &mut dyn RandomSource) -> i32 {
        mth::reduced_tick_delay(200 + r.next_int_bounded(200) % 20)
    }
}

impl CustomGoal for FollowFlockLeaderGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "FollowFlockLeaderGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        if s.school_size > 1 {
            return false;
        }
        if is_follower(s, level) {
            return true;
        }
        if self.next_start > 0 {
            self.next_start -= 1;
            return false;
        }
        self.next_start = Self::next_start_tick(&mut e.random);
        let kind = m.kind;
        let max = max_school_size(kind);
        let me = (e.id, mob::is_alive(e, m));
        // `getEntitiesOfClass(this class, box, canBeFollowed || !isFollower)`, with the fish itself
        // in its place (the section, then the id).
        let area = e.bounding_box().inflate(8.0, 8.0, 8.0);
        let mut list: Vec<i32> = level
            .entities_in(&area, crate::level::EntityFilter::Living, e.id)
            .into_iter()
            .filter(|&id| {
                let Some(s) = state_of(level, id) else { return false };
                level.entity(id).and_then(mob::data).is_some_and(|om| om.kind == kind) && (can_be_followed(s, kind) || !is_follower_near(s, level, me))
            })
            .collect();
        {
            let me = st(m);
            if can_be_followed(me, kind) || !is_follower(me, level) {
                let key = section_key(e.block_position());
                let at = list
                    .iter()
                    .position(|&id| level.entity(id).is_some_and(|o| (section_key(o.block_position()), o.id) > (key, e.id)))
                    .unwrap_or(list.len());
                list.insert(at, e.id);
            }
        }
        let followed = |level: &dyn EntityLevel, m: &MobData, id: i32| {
            let s = if id == e.id { Some(st(m)) } else { state_of(level, id) };
            s.is_some_and(|s| can_be_followed(s, kind))
        };
        let leader = list.iter().copied().find(|&id| followed(level, m, id)).unwrap_or(e.id);
        // `addFollowers`: the non-followers, `limit(max - schoolSize)` before dropping the leader.
        let leader_size = if leader == e.id { st(m).school_size } else { state_of(level, leader).map_or(1, |s| s.school_size) };
        let mut budget = (max - leader_size).max(0);
        for id in list {
            if budget == 0 {
                break;
            }
            let follower = if id == e.id { is_follower(st(m), level) } else { state_of(level, id).is_some_and(|s| is_follower_near(s, level, me)) };
            if follower {
                continue;
            }
            budget -= 1;
            if id == leader {
                continue;
            }
            // `startFollowing(leader)`.
            with_fish(e.id, m, level, id, |s| s.leader = Some(leader));
            with_fish(e.id, m, level, leader, |s| s.school_size += 1);
        }
        is_follower(st(m), level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let s = st(m);
        is_follower(s, level) && s.leader.and_then(|id| level.entity(id)).is_some_and(|l| e.position().distance_to_sqr(l.position()) <= 121.0)
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.recalc = 0;
    }
    /// `stopFollowing`.
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(leader) = st_mut(m).leader.take() {
            with_fish(e.id, m, level, leader, |s| s.school_size -= 1);
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.recalc -= 1;
        if self.recalc > 0 {
            return;
        }
        self.recalc = mth::reduced_tick_delay(10);
        // `pathToLeader`.
        if let Some(l) = st(m).leader.filter(|&id| leader_alive(level, id)).and_then(|id| level.entity(id)) {
            let p = l.block_position();
            path::move_to_entity(e, m, level, p, 1.0);
        }
    }
}

/// Vanilla's entity section order: by section x, then the packed (z, y) key.
fn section_key(p: BlockPos) -> (i32, i64) {
    let (sx, sy, sz) = (p.x >> 4, p.y >> 4, p.z >> 4);
    (sx, (((sz as i64) & 0x3F_FFFF) << 20) | ((sy as i64) & 0xF_FFFF))
}

/// `Pufferfish.PufferfishPuffGoal`: while anything scary is within 2 blocks.
#[derive(Clone, Debug)]
struct PufferfishPuffGoal;

impl CustomGoal for PufferfishPuffGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PufferfishPuffGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let area = e.bounding_box().inflate_all(2.0);
        level.entities_in(&area, crate::level::EntityFilter::Living, e.id).into_iter().any(|id| goals::living(level, id).is_some_and(|t| scary(level, &t)))
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let s = st_mut(m);
        s.inflate_counter = 1;
        s.deflate_timer = 0;
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).inflate_counter = 0;
    }
}

/// The variant components entity predicates see (`salmon/size`, `tropical_fish/*`,
/// `mooshroom/variant`).
pub fn variant_components(m: &MobData) -> Option<Vec<kiln_item::Component>> {
    use kiln_item::Component as C;
    use kiln_item::component::DyeColor;
    use kiln_item::component::variant::{MooshroomVariant, SalmonSize, TropicalFishPattern};
    if m.kind == MobKind::Mooshroom {
        let brown = super::mooshroom::is_brown(m);
        return Some(vec![C::MooshroomVariant(if brown { MooshroomVariant::Brown } else { MooshroomVariant::Red })]);
    }
    let s = ext::state::<FishState>(m)?;
    Some(match m.kind {
        MobKind::Salmon => vec![C::SalmonSize(*SalmonSize::ALL.get(s.variant.clamp(0, 2) as usize)?)],
        MobKind::TropicalFish => {
            let v = s.variant;
            let p = *TropicalFishPattern::ALL.iter().find(|p| p.id() == v & 0xFFFF).unwrap_or(&TropicalFishPattern::Kob);
            vec![
                C::TropicalFishPattern(p),
                C::TropicalFishBaseColor(DyeColor::from_id((v >> 16) & 0xFF)?),
                C::TropicalFishPatternColor(DyeColor::from_id((v >> 24) & 0xFF)?),
            ]
        }
        _ => return None,
    })
}
