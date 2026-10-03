//! Horses, donkeys, mules and llamas (`AbstractHorse`, `AbstractChestedHorse`, `Llama`,
//! `TraderLlama`): random stats at spawn, grazing and rearing, tamed by riding (temper,
//! `RunAroundLikeCrazyGoal` bucks the rider), fed, saddled and then steered by their rider, chests
//! on donkeys, mules and llamas (15 slots; a llama's three per strength), body armor (a llama's
//! carpet), the inventory the screen shows (saddle, armor and the chest's slots, kept in
//! [`State`], dropped when the animal dies), breeding. The llamas' own goals and spit are in
//! [`super::llama`].
//! Not modelled: horse-donkey cross breeding (mules).

use super::tame::TamableAnimalPanicGoal;
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, PlayerView};
use crate::math::Vec3;
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{Goal, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{DamageSource, GroupData, MobData, MobKind, SpawnContext, item_name, item_tag, path, random_pos};
use crate::persist::{Input, Output, uuid_to_tag};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Horse,
    Donkey,
    Mule,
    /// `SkeletonHorse` (the trap horse of thunderstorms, see [`super::skeleton_horse`]).
    Skeleton,
    /// `Llama` and `TraderLlama` (see [`super::llama`]): a chested equine with a strength, spit.
    Llama,
    TraderLlama,
    /// `ZombieHorse`: a monster that burns in daylight (its body armor keeps the sun off), is
    /// never a baby, cannot breed and is steered by a mob rider (the zombie of a natural spawn).
    Zombie,
}

pub struct Equine(pub Which);

pub static KIND: Equine = Equine(Which::Horse);
pub static DONKEY: Equine = Equine(Which::Donkey);
pub static MULE: Equine = Equine(Which::Mule);
pub static SKELETON: Equine = Equine(Which::Skeleton);
pub static LLAMA: Equine = Equine(Which::Llama);
pub static TRADER_LLAMA: Equine = Equine(Which::TraderLlama);
pub static ZOMBIE: Equine = Equine(Which::Zombie);

static HORSE_INFO: Info = Info {
    ambient_interval: 400,
    ..Info::animal("minecraft:horse", &[(JumpStrength, 0.7), (MaxHealth, 53.0), (MovementSpeed, 0.22499999403953552), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5)])
};
static DONKEY_INFO: Info = Info {
    ambient_interval: 400,
    ..Info::animal("minecraft:donkey", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)])
};
static MULE_INFO: Info = Info {
    ambient_interval: 400,
    ..Info::animal("minecraft:mule", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)])
};

static LLAMA_INFO: Info = Info {
    ambient_interval: 400,
    ..Info::animal("minecraft:llama", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)])
};
/// (`TraderLlama` has no sounds of its own: it makes the llama's.)
static TRADER_LLAMA_INFO: Info = Info {
    ambient_interval: 400,
    sounds: Some("llama"),
    ..Info::animal("minecraft:trader_llama", &[(MaxHealth, 53.0), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5), (MovementSpeed, 0.17499999701976776), (JumpStrength, 0.5)])
};

static ZOMBIE_INFO: Info = Info {
    category: crate::mob::Category::Monster,
    burns_in_daylight: true,
    breathes_under_water: true,
    ambient_interval: 400,
    ..Info::animal("minecraft:zombie_horse", &[(JumpStrength, 0.7), (MaxHealth, 25.0), (MovementSpeed, 0.22499999403953552), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5)])
};

static SKELETON_INFO: Info = Info {
    ambient_interval: 400,
    ..Info::animal("minecraft:skeleton_horse", &[(JumpStrength, 0.7), (MaxHealth, 15.0), (MovementSpeed, 0.20000000298023224), (StepHeight, 1.0), (SafeFallDistance, 6.0), (FallDamageMultiplier, 0.5)])
};

#[derive(Clone, Debug)]
pub struct State {
    pub tamed: bool,
    pub bred: bool,
    pub eating: bool,
    pub standing: bool,
    pub open_mouth: bool,
    pub temper: i32,
    pub owner: Option<u128>,
    eating_counter: i32,
    mouth_counter: i32,
    stand_counter: i32,
    tail_counter: i32,
    sprint_counter: i32,
    eat_anim: f32,
    eat_anim_o: f32,
    stand_anim: f32,
    stand_anim_o: f32,
    mouth_anim: f32,
    mouth_anim_o: f32,
    allow_stand_sliding: bool,
    /// `EquipmentSlot.SADDLE`.
    pub saddle: ItemStack,
    /// `AbstractChestedHorse.hasChest`.
    pub chest: bool,
    /// `AbstractHorse.inventory`: the chest's slots (`3 * columns`, none without a chest).
    pub inventory: Vec<ItemStack>,
    /// Saved `Items` entries that did not decode, written back unchanged.
    inv_undecoded: Vec<(i32, Tag)>,
    /// `EquipmentSlot.BODY`: horse armor (a llama's carpet).
    pub body: ItemStack,
    /// What `dropChances` says about the saddle and the body slot (a slot filled by a player
    /// is guaranteed to drop: 2).
    pub saddle_drop: f32,
    pub body_drop: f32,
    /// Counts the times the inventory was made anew (the screen on the old one closes).
    pub inv_serial: u32,
    /// Equip sounds of changes made through the screen: `onEquipItem` drew its seed when the
    /// change was made, the sound itself goes out at the animal's next tick.
    pending_equip: Vec<&'static str>,
    /// `Horse.DATA_ID_TYPE_VARIANT`: variant | markings << 8.
    pub type_variant: i32,
    /// `SkeletonHorse.isTrap` and `trapTime`.
    pub trap: bool,
    pub trap_time: i32,
    /// `Llama.DATA_STRENGTH_ID` (1 to 5; 0 until set).
    pub strength: i32,
    /// `Llama.DATA_VARIANT_ID` (`Llama.Variant` id).
    pub llama_variant: i32,
    /// `Llama.didSpit`.
    pub did_spit: bool,
    /// `Llama.caravanHead` and `caravanTail` (entity ids).
    pub caravan_head: Option<i32>,
    pub caravan_tail: Option<i32>,
    /// `TraderLlama.despawnDelay`.
    pub despawn_delay: i32,
}

/// The horse state of `m`, if it is one of the family.
pub(super) fn state(m: &MobData) -> Option<&State> {
    ext::state::<State>(m)
}

pub(super) fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("horse state")
}

pub(super) fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("horse state")
}

/// `AbstractHorse.isTamed`.
pub fn is_tamed(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.tamed)
}

fn is(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && item_name(stack) == name
}

fn sound(m: &MobData, what: &str) -> &'static str {
    // (A trader llama has the llama's sounds.)
    let name = if m.kind == MobKind::TraderLlama { "llama" } else { m.kind.short_name() };
    crate::mob::sound_event(&format!("minecraft:entity.{name}.{what}"))
}

/// `getMaxTemper`: 100, a llama's 30.
fn max_temper(m: &MobData) -> i32 {
    if super::llama::is_llama(m.kind) { 30 } else { 100 }
}

/// `LivingEntity.getVoicePitch` (two draws).
fn voice_pitch(e: &mut Entity, m: &MobData) -> f32 {
    let d = (e.random.next_float() - e.random.next_float()) * 0.2;
    if m.baby() { d + 1.5 } else { d + 1.0 }
}

fn play(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, volume: f32, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume, pitch });
    }
}

/// `setStanding(20)` through `standIfPossible` (`canPerformRearing`: llamas never rear).
fn stand(m: &mut MobData) {
    if super::llama::is_llama(m.kind) {
        return;
    }
    let s = st_mut(m);
    s.eating = false;
    s.standing = true;
    s.stand_counter = 20;
}

/// `AbstractHorse.onElasticLeashPull`: a pulled horse stops eating.
pub fn stop_eating(m: &mut MobData) {
    if let Some(s) = ext::state_mut::<State>(m) {
        s.eating = false;
    }
}

fn clear_standing(m: &mut MobData) {
    let s = st_mut(m);
    s.standing = false;
    s.stand_counter = 0;
}

/// `makeMad`: rears up and makes its angry sound (skeleton horses have none: `getAngrySound`
/// is null, and no voice pitch is drawn).
fn make_mad(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if st(m).standing {
        return;
    }
    stand(m);
    if m.kind == MobKind::SkeletonHorse {
        return;
    }
    let pitch = voice_pitch(e, m);
    play(e, level, sound(m, "angry"), 0.8, pitch);
}

fn heal(m: &mut MobData, amount: f32) {
    if m.health > 0.0 {
        let h = m.health + amount;
        m.set_health(h);
    }
}

/// `eating`: the mouth opens with the eating sound.
fn eating(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let s = st_mut(m);
    s.mouth_counter = 1;
    s.open_mouth = true;
    // (`getEatingSound` is null for skeleton horses: no sound, no draws.)
    if !e.silent && m.kind != MobKind::SkeletonHorse {
        let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
        level.emit(Event::Sound { pos: e.position(), sound: sound(m, "eat"), source: "neutral", volume: 1.0, pitch });
    }
    level.emit(Event::GameEvent { event: "minecraft:eat", pos: e.position(), entity: Some(e.id) });
}

/// `Llama.handleEating`: wheat and hay bales heal, make a baby grow and raise the temper (a hay
/// bale also puts a tame, grown llama in love); the eating sound, no open mouth.
fn handle_eating_llama(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> bool {
    let mut ate = false;
    let (age_up, temper_up, heal_by): (i32, i32, f32) = match item_name(stack) {
        "minecraft:wheat" => (10, 3, 2.0),
        "minecraft:hay_block" => {
            if st(m).tamed && m.age == 0 && m.in_love <= 0 {
                ate = true;
                crate::mob::breed::set_in_love(e, m, level, Some(who.id));
            }
            (90, 6, 10.0)
        }
        _ => (0, 0, 0.0),
    };
    if m.health < m.max_health() && heal_by > 0.0 {
        heal(m, heal_by);
        ate = true;
    }
    if m.baby() && age_up > 0 && !m.age_locked {
        crate::mob::random_point(e, 1.0);
        crate::mob::age_up(e, m, age_up, false);
        ate = true;
    }
    if temper_up > 0 && (ate || !st(m).tamed) && st(m).temper < max_temper(m) {
        let max = max_temper(m);
        let s = st_mut(m);
        s.temper = (s.temper + temper_up).clamp(0, max);
        ate = true;
    }
    if ate && !e.silent {
        let pitch = 1.0 + (e.random.next_float() - e.random.next_float()) * 0.2;
        level.emit(Event::Sound { pos: e.position(), sound: sound(m, "eat"), source: "neutral", volume: 1.0, pitch });
    }
    ate
}

/// `handleEating`: whether the horse ate `stack`.
fn handle_eating(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> bool {
    if super::llama::is_llama(m.kind) {
        return handle_eating_llama(e, m, level, who, stack);
    }
    let mut ate = false;
    let (heal_by, age_up, temper_up): (f32, i32, i32) = match item_name(stack) {
        "minecraft:wheat" => (2.0, 20, 3),
        "minecraft:sugar" => (1.0, 30, 3),
        "minecraft:hay_block" => (20.0, 180, 0),
        "minecraft:apple" => (3.0, 60, 3),
        "minecraft:red_mushroom" => (3.0, 0, 3),
        "minecraft:carrot" => (3.0, 60, 3),
        "minecraft:golden_carrot" => (4.0, 60, 5),
        "minecraft:golden_apple" | "minecraft:enchanted_golden_apple" => (10.0, 240, 10),
        _ => (0.0, 0, 0),
    };
    let love_food = matches!(item_name(stack), "minecraft:golden_carrot" | "minecraft:golden_apple" | "minecraft:enchanted_golden_apple");
    if love_food && st(m).tamed && m.age == 0 && m.in_love <= 0 {
        ate = true;
        crate::mob::breed::set_in_love(e, m, level, Some(who.id));
    }
    if m.health < m.max_health() && heal_by > 0.0 {
        heal(m, heal_by);
        ate = true;
    }
    if m.baby() && age_up > 0 && !m.age_locked {
        crate::mob::random_point(e, 1.0);
        crate::mob::age_up(e, m, age_up, false);
        ate = true;
    }
    if temper_up > 0 && (ate || !st(m).tamed) && st(m).temper < 100 {
        let s = st_mut(m);
        s.temper = (s.temper + temper_up).clamp(0, 100);
        ate = true;
    }
    if ate {
        eating(e, m, level);
    }
    ate
}

/// `generateMaxHealth`.
fn random_health(r: &mut dyn RandomSource) -> f32 {
    15.0 + r.next_int_bounded(8) as f32 + r.next_int_bounded(9) as f32
}

/// `generateJumpStrength`.
fn random_jump(r: &mut dyn RandomSource) -> f64 {
    0.4000000059604645 + r.next_double() * 0.2 + r.next_double() * 0.2 + r.next_double() * 0.2
}

/// `generateSpeed`.
fn random_speed(r: &mut dyn RandomSource) -> f64 {
    (0.44999998807907104 + r.next_double() * 0.3 + r.next_double() * 0.3 + r.next_double() * 0.3) * 0.25
}

fn set_base(m: &mut MobData, a: Attr, v: f64) {
    // (`setBaseValue` leaves the health alone: `LivingEntity.tick` brings it down to the new
    // maximum at its end, `refreshDirtyAttributes`, see `post_tick`.)
    if let Some(i) = m.attrs.get_mut(a) {
        i.base = v;
    }
}

/// `createOffspringAttribute`.
fn offspring_attribute(a: f64, b: f64, min: f64, max: f64, r: &mut dyn RandomSource) -> f64 {
    let a = a.clamp(min, max);
    let b = b.clamp(min, max);
    let margin = 0.15 * (max - min);
    let spread = (a - b).abs() + margin * 2.0;
    let mean = (a + b) / 2.0;
    let x = (r.next_double() + r.next_double() + r.next_double()) / 3.0 - 0.5;
    let v = mean + spread * x;
    if v > max {
        return max - (v - max);
    }
    if v < min {
        return min + (min - v);
    }
    v
}

impl Equine {
    fn chested(&self) -> bool {
        matches!(self.0, Which::Donkey | Which::Mule | Which::Llama | Which::TraderLlama)
    }

    fn llama(&self) -> bool {
        matches!(self.0, Which::Llama | Which::TraderLlama)
    }

    /// `getInventoryColumns`: five with a chest (a llama: its strength).
    fn columns(&self, m: &MobData) -> usize {
        if !(self.chested() && st(m).chest) {
            0
        } else if self.llama() {
            st(m).strength.clamp(0, 5) as usize
        } else {
            5
        }
    }

    /// `createInventory`: a new container of `getInventorySize()` slots that takes over what fits
    /// of the old one.
    fn create_inventory(&self, m: &mut MobData) {
        let n = self.columns(m) * 3;
        let s = st_mut(m);
        let mut inv = vec![ItemStack::empty(); n];
        for (i, slot) in s.inventory.iter().enumerate().take(n) {
            inv[i] = slot.clone();
        }
        s.inventory = inv;
        s.inv_serial += 1;
    }

    /// The type's passenger attachment height (`passengerAttachments`).
    fn attach_height(&self, m: &MobData) -> (f64, f64) {
        match (self.0, m.baby()) {
            (Which::Horse, false) => (1.44375, 0.0),
            (Which::Horse, true) => (((1.6f32 - 0.125) * 0.7) as f64, 0.0),
            (Which::Donkey, false) => (1.1125, 0.0),
            (Which::Mule, false) => (1.2125, 0.0),
            (Which::Skeleton | Which::Zombie, false) => (1.31875f32 as f64, 0.0),
            (Which::Skeleton, true) => (((1.6f32 - 0.25) * 0.7) as f64, 0.0),
            (Which::Llama | Which::TraderLlama, false) => (1.37, -0.3f32 as f64),
            (Which::Llama | Which::TraderLlama, true) => (((1.87f32 - 0.25) * 0.5) as f64, (-0.3f32 * 0.5) as f64),
            (_, true) => (((1.5f32 + 0.03125) * 0.5) as f64, (-0.3125f32 * 0.5) as f64),
        }
    }
}

impl Kind for Equine {
    /// `SpawnPlacements`: a zombie horse spawns by `Monster.checkMonsterSpawnRules` (dark enough,
    /// not in peaceful), not by the animals' rules (light and grass) the other horses use.
    fn check_spawn_rules(&self, view: &dyn ext::SpawnView, pos: crate::math::BlockPos, r: &mut kiln_javamath::random::LegacyRandom) -> Option<bool> {
        match self.0 {
            Which::Zombie => Some(super::zombie::monster_rules(view, pos, r)),
            _ => None,
        }
    }

    fn info(&self) -> &'static Info {
        match self.0 {
            Which::Horse => &HORSE_INFO,
            Which::Donkey => &DONKEY_INFO,
            Which::Mule => &MULE_INFO,
            Which::Skeleton => &SKELETON_INFO,
            Which::Llama => &LLAMA_INFO,
            Which::TraderLlama => &TRADER_LLAMA_INFO,
            Which::Zombie => &ZOMBIE_INFO,
        }
    }

    fn can_be_baby(&self) -> bool {
        self.0 != Which::Zombie
    }

    fn allowed_in_peaceful(&self) -> Option<bool> {
        (self.0 == Which::Zombie).then_some(true)
    }

    fn remove_when_far_away(&self, _m: &MobData) -> Option<bool> {
        (self.0 == Which::Zombie).then_some(true)
    }

    fn sun_protection_on_body(&self) -> bool {
        self.0 == Which::Zombie
    }

    fn body_slot_mut<'a>(&self, m: &'a mut MobData) -> Option<&'a mut ItemStack> {
        ext::state_mut::<State>(m).map(|s| &mut s.body)
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State {
            tamed: false,
            bred: false,
            eating: false,
            standing: false,
            open_mouth: false,
            temper: 0,
            owner: None,
            eating_counter: 0,
            mouth_counter: 0,
            stand_counter: 0,
            tail_counter: 0,
            sprint_counter: 0,
            eat_anim: 0.0,
            eat_anim_o: 0.0,
            stand_anim: 0.0,
            stand_anim_o: 0.0,
            mouth_anim: 0.0,
            mouth_anim_o: 0.0,
            allow_stand_sliding: false,
            saddle: ItemStack::empty(),
            chest: false,
            inventory: Vec::new(),
            inv_undecoded: Vec::new(),
            body: ItemStack::empty(),
            saddle_drop: 0.085,
            body_drop: 0.085,
            inv_serial: 0,
            pending_equip: Vec::new(),
            type_variant: 0,
            trap: false,
            trap_time: 0,
            strength: 0,
            llama_variant: 0,
            did_spit: false,
            caravan_head: None,
            caravan_tail: None,
            despawn_delay: 47999,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        if self.llama() {
            // (`Llama` replaces `AbstractHorse.registerGoals`; its constructor sets the path length.)
            m.nav.required_path_length = 40.0;
            super::llama::register_goals(m, self.0 == Which::TraderLlama);
            return;
        }
        let g = &mut m.goals;
        g.add(1, Goal::Custom(Box::new(RunAroundLikeCrazyGoal { speed: 1.2, pos: Vec3::ZERO })));
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(4, Goal::FollowParent { speed: 1.0, parent: None, recalc: 0 });
        g.add(6, Goal::RandomStroll { speed: 0.7, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(8, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        g.add(9, Goal::Custom(Box::new(RandomStandGoal { next_stand: -400 })));
        if self.0 == Which::Skeleton {
            // `SkeletonHorse.addBehaviourGoals` is empty: no float, panic or tempt goals.
            return;
        }
        g.add(0, Goal::Float);
        // (`ZombieHorse.addBehaviourGoals`: no panic goal.)
        if self.0 != Which::Zombie {
            g.add(1, Goal::Custom(Box::new(TamableAnimalPanicGoal::named("MountPanicGoal", 1.2, "minecraft:panic_causes"))));
        }
        g.add(3, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
    }

    /// `AbstractHorse.getMaxSpawnClusterSize`.
    fn max_spawn_cluster(&self) -> i32 {
        6
    }

    /// `AbstractHorse.getSoundVolume`.
    fn sound_volume(&self, _m: &MobData) -> f32 {
        0.8
    }

    fn tempted_by(&self, item: i32) -> bool {
        if self.0 == Which::Zombie {
            return item_tag(item, "minecraft:zombie_horse_food");
        }
        item_tag(item, if self.llama() { "minecraft:llama_tempt_items" } else { "minecraft:horse_tempt_items" })
    }

    fn water_slow_down(&self, _m: &MobData) -> f32 {
        if self.0 == Which::Skeleton { 0.96 } else { 0.8 }
    }

    fn is_food(&self, item: i32) -> bool {
        if self.0 == Which::Zombie {
            return item_tag(item, "minecraft:zombie_horse_food");
        }
        item_tag(item, if self.llama() { "minecraft:llama_food" } else { "minecraft:horse_food" })
    }

    fn is_immobile(&self, m: &MobData) -> bool {
        let s = st(m);
        s.eating || s.standing
    }

    fn ai_step_before(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // What was put on or taken off through the screen since the last tick (`onEquipItem`).
        for sound in std::mem::take(&mut st_mut(m).pending_equip) {
            level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        if e.random.next_int_bounded(200) == 0 {
            st_mut(m).tail_counter = 1;
        }
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if self.0 == Which::TraderLlama {
            // `TraderLlama.aiStep` after the horse's.
            self.ai_step_horse(e, m, level);
            self.maybe_despawn(e, m, level);
            return;
        }
        self.ai_step_horse(e, m, level);
    }

    fn custom_server_ai_step(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if self.0 == Which::Skeleton {
            // The trap goal that fired took itself off the selector (`setTrap(false)`).
            super::skeleton_horse::drop_spent_goal(m);
        }
    }

    fn post_tick(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        // `LivingEntity.refreshDirtyAttributes` at the end of its tick: health above a lowered
        // maximum comes down to it.
        let max = m.max_health();
        if m.health > max {
            m.set_health(max);
        }
        self.post_tick_horse(m);
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt && e.random.next_int_bounded(3) == 0 {
            stand(m);
        }
    }

    fn steerable_by(&self, m: &MobData, _rider: &PlayerView) -> bool {
        !st(m).saddle.is_empty()
    }

    fn tick_ridden(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, rider: &PlayerView) {
        // `getRiddenRotation`: the rider's yaw, half its pitch.
        e.y_rot = rider.yaw % 360.0;
        e.x_rot = (rider.pitch * 0.5) % 360.0;
        e.y_rot_o = e.y_rot;
        m.y_body_rot = e.y_rot;
        m.y_head_rot = e.y_rot;
    }

    fn passenger_offset(&self, e: &Entity, m: &MobData) -> Option<Vec3> {
        let (h, z) = self.attach_height(m);
        let base = crate::ride::y_rot(Vec3::new(0.0, h, z), -e.y_rot * 0.017453292);
        let a = st(m).stand_anim_o as f64;
        let stand = crate::ride::y_rot(Vec3::new(0.0, 0.15 * a, -0.7 * a), -e.y_rot * 0.017453292);
        Some(base + stand)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if !m.baby() {
            return base;
        }
        let s = if self.chested() { 0.5 } else { 0.7 };
        (base.0 * s, base.1 * s, base.2 * s)
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        if self.0 == Which::Zombie {
            // `ZombieHorse.finalizeSpawn`: a natural one carries a zombie with an iron spear
            // (finalized for the same reason, no group data).
            if group.natural {
                let mut zombie = crate::mob::new_jockey(e, MobKind::Zombie);
                let mut zombie_group = GroupData::default();
                crate::mob::finalize_spawn(&mut zombie, r, ctx, &mut zombie_group, true);
                if let (Some(zm), Some(spear)) = (crate::mob::data_mut(&mut zombie), ItemStack::of("minecraft:iron_spear", 1)) {
                    zm.equipment[crate::mob::MAINHAND] = spear;
                }
                group.companions.push(crate::mob::Companion { entity: zombie, seat: crate::mob::Seat::OnMob });
                // (A chicken the zombie rode is left behind when it mounts the horse.)
                for c in zombie_group.companions {
                    group.companions.push(crate::mob::Companion { entity: c.entity, seat: crate::mob::Seat::Loose });
                }
            }
            // `randomizeAttributes`: the jump strength, then the speed (three draws each).
            let jump = ((0.5 + r.next_double() * 0.06666666666666667) + r.next_double() * 0.06666666666666667) + r.next_double() * 0.06666666666666667;
            set_base(m, JumpStrength, jump);
            let speed = (((9.0 + r.next_double() * 1.0) + r.next_double() * 1.0) + r.next_double() * 1.0) / 42.15999984741211;
            set_base(m, MovementSpeed, speed);
            ext::ageable_finalize(e, m, r, group, 0.2);
            ext::mob_finalize(m, r);
            return;
        }
        if self.0 == Which::Skeleton {
            // `SkeletonHorse.randomizeAttributes`: only the jump strength.
            let jump = random_jump(r);
            set_base(m, JumpStrength, jump);
            ext::ageable_finalize(e, m, r, group, 0.2);
            ext::mob_finalize(m, r);
            return;
        }
        if self.llama() {
            // `Llama.finalizeSpawn`: strength, then the coat (the group's, or a random one
            // that the group takes on), then `AbstractHorse.finalizeSpawn`.
            super::llama::set_random_strength(m, r);
            let variant = match group.variant {
                Some(v) => v,
                None => {
                    let v = r.next_int_bounded(4);
                    group.variant = Some(v);
                    v
                }
            };
            st_mut(m).llama_variant = variant;
            // (`TraderLlama.finalizeSpawn`: one that came with an event is an adult.)
            if self.0 == Which::TraderLlama && group.event {
                crate::mob::set_age(e, m, 0);
            }
            let health = random_health(r);
            set_base(m, MaxHealth, health as f64);
            ext::ageable_finalize(e, m, r, group, 0.05);
            ext::mob_finalize(m, r);
            return;
        }
        let chance = if self.0 == Which::Horse {
            // `HorseGroupData`: one coat for the group, markings each.
            let variant = match group.variant {
                Some(v) => v,
                None => {
                    let v = r.next_int_bounded(7);
                    group.variant = Some(v);
                    v
                }
            };
            let markings = r.next_int_bounded(5);
            st_mut(m).type_variant = (variant & 255) | ((markings << 8) & 65280);
            0.05
        } else {
            0.2
        };
        // `randomizeAttributes`.
        let health = random_health(r);
        set_base(m, MaxHealth, health as f64);
        if self.0 == Which::Horse {
            let speed = random_speed(r);
            set_base(m, MovementSpeed, speed);
            let jump = random_jump(r);
            set_base(m, JumpStrength, jump);
        }
        ext::ageable_finalize(e, m, r, group, chance);
        ext::mob_finalize(m, r);
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        // `canParent` on both (vehicles are not checked here); mules never breed.
        let parent = |x: &MobData| is_tamed(x) && !x.baby() && x.health >= x.max_health() && x.in_love > 0;
        if self.llama() {
            // `Llama.canMate`: another llama of either kind.
            return super::llama::is_llama(partner.kind) && parent(m) && parent(partner);
        }
        // `Horse.canMate` and `Donkey.canMate`: the partner is a horse or a donkey (mules, skeleton
        // horses and the rest never mate: `AbstractHorse.canMate` is false).
        matches!(self.0, Which::Horse | Which::Donkey) && matches!(partner.kind, MobKind::Horse | MobKind::Donkey) && parent(m) && parent(partner)
    }

    /// `Horse.getBreedOffspring` / `Donkey.getBreedOffspring`: a horse and a donkey have a mule.
    fn offspring_kind(&self, m: &MobData, partner: &MobData) -> MobKind {
        match (self.0, partner.kind) {
            (Which::Horse, MobKind::Donkey) | (Which::Donkey, MobKind::Horse) => MobKind::Mule,
            _ => m.kind,
        }
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        if matches!(self.0, Which::Mule | Which::Skeleton) {
            return;
        }
        if self.llama() {
            // `Llama.getBreedOffspring`: the attributes, then the strength (the larger parent's
            // as a bound, a rare extra point) and the coat of either parent.
            self.offspring_attributes(e, m, partner, child);
            let (mine, theirs) = (st(m).strength, ext::state::<State>(partner).map_or(0, |s| s.strength));
            let mut strength = e.random.next_int_bounded(mine.max(theirs).max(1)) + 1;
            if e.random.next_float() < 0.03 {
                strength += 1;
            }
            super::llama::set_strength(child, strength);
            let variant = if e.random.next_bool() { st(m).llama_variant } else { ext::state::<State>(partner).map_or(0, |s| s.llama_variant) };
            st_mut(child).llama_variant = variant;
            if self.0 == Which::TraderLlama {
                // `TraderLlama.makeNewLlama`: the baby stays.
                child.persistence_required = true;
            }
            return;
        }
        // A mule gets only the attributes of its parents (a horse and a donkey).
        if self.0 == Which::Horse && partner.kind == MobKind::Horse {
            let (mine, theirs) = (st(m).type_variant, ext::state::<State>(partner).map_or(0, |s| s.type_variant));
            let r = e.random.next_int_bounded(9);
            let variant = if r < 4 {
                mine & 255
            } else if r < 8 {
                theirs & 255
            } else {
                e.random.next_int_bounded(7)
            };
            let k = e.random.next_int_bounded(5);
            let markings = if k < 2 {
                (mine & 65280) >> 8
            } else if k < 4 {
                (theirs & 65280) >> 8
            } else {
                e.random.next_int_bounded(5)
            };
            st_mut(child).type_variant = (variant & 255) | ((markings << 8) & 65280);
        }
        self.offspring_attributes(e, m, partner, child);
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if self.0 == Which::Zombie {
            // `ZombieHorse.interact`: whoever touches it makes it stay.
            m.persistence_required = true;
        }
        // `SkeletonHorse.mobInteract`: a wild one ignores everybody.
        if self.0 == Which::Skeleton && !st(m).tamed {
            return Some(Outcome::PASS);
        }
        let vehicle = !e.passengers.is_empty();
        let open_inventory = !m.baby() && st(m).tamed && who.sneaking;
        let dandelion = m.baby() && is(stack, "minecraft:golden_dandelion");
        if !(vehicle || open_inventory || dandelion) && !stack.is_empty() {
            if self.is_food(stack.item()) {
                // `fedFood`.
                if handle_eating(e, m, level, who, stack) {
                    return Some(Outcome::success(HeldChange::Consume(1)));
                }
                return Some(Outcome::PASS);
            }
            if !st(m).tamed {
                make_mad(e, m, level);
                return Some(Outcome::success(HeldChange::None));
            }
            if self.chested() && !st(m).chest && is(stack, "minecraft:chest") {
                // `equipChest`.
                st_mut(m).chest = true;
                let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0;
                play(e, level, sound(m, "chest"), 1.0, pitch);
                self.create_inventory(m);
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
        }
        // `AbstractHorse.mobInteract`.
        if vehicle || m.baby() {
            return Some(crate::mob::interact::animal_interact(e, m, level, who, stack));
        }
        if st(m).tamed && who.sneaking {
            // `openCustomInventoryScreen`: the screen of the tame animal, for whoever rides it
            // or when nobody does.
            let mut out = Outcome::success(HeldChange::None);
            out.open_container = e.passengers.is_empty() || e.passengers.contains(&who.id);
            return Some(out);
        }
        // (A llama is not in `#can_equip_saddle`: the saddle does not go on.)
        if !self.llama() && is(stack, "minecraft:saddle") && st(m).tamed && st(m).saddle.is_empty() && crate::mob::is_alive(e, m) {
            let mut one = stack.clone();
            one.set_count(1);
            st_mut(m).saddle = one;
            play(e, level, "minecraft:entity.horse.saddle", 0.5, 1.0);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        // `equipBodyArmor`: armor from the hand goes on a bare body.
        if !stack.is_empty() && st(m).body.is_empty() && equippable_in_slot(stack, kiln_item::component::EquipmentSlot::Body, e.type_name) {
            let mut one = stack.clone();
            one.set_count(1);
            let s = st_mut(m);
            s.body = one.clone();
            s.body_drop = 2.0;
            equip_sound(e, level, kiln_item::component::EquipmentSlot::Body, &ItemStack::empty(), &one);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        // `TraderLlama.doPlayerRide`: nobody rides a llama a wandering trader leads.
        if m.kind == MobKind::TraderLlama
            && crate::leash::holder_of(e).and_then(|h| level.entity(h)).is_some_and(|h| h.type_name == "minecraft:wandering_trader")
        {
            return Some(Outcome::success(HeldChange::None));
        }
        // `doPlayerRide`.
        st_mut(m).eating = false;
        clear_standing(m);
        let mut out = Outcome::success(HeldChange::None);
        out.ride = true;
        Some(out)
    }

    fn extra_equipment(&self, m: &MobData) -> Vec<(u8, ItemStack)> {
        let s = st(m);
        let mut out = Vec::new();
        if !s.body.is_empty() {
            out.push((6, s.body.clone()));
        }
        if !s.saddle.is_empty() {
            out.push((7, s.saddle.clone()));
        }
        out
    }

    fn take_extra_equipment_for_drop(&self, m: &mut MobData) -> Vec<(ItemStack, f32)> {
        let s = st_mut(m);
        vec![(std::mem::take(&mut s.body), s.body_drop), (std::mem::take(&mut s.saddle), s.saddle_drop)]
    }

    /// `AbstractHorse.dropEquipment` (the inventory) and `AbstractChestedHorse.dropEquipment` (the
    /// chest).
    fn drop_equipment(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let items = std::mem::take(&mut st_mut(m).inventory);
        for stack in items {
            if !stack.is_empty() {
                crate::mob::spawn_at_location(e, level, stack);
            }
        }
        if self.chested() && st(m).chest {
            if let Some(chest) = ItemStack::of("minecraft:chest", 1) {
                crate::mob::spawn_at_location(e, level, chest);
            }
            st_mut(m).chest = false;
        }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let saddle = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "saddle").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let body = match r.get("equipment") {
            Some(Tag::Compound(eq)) => eq.iter().find(|(k, _)| k == "body").and_then(|(_, v)| ItemStack::from_nbt(v).ok()),
            _ => None,
        };
        let drop_chance = |r: &mut Input, key: &str| match r.get("drop_chances") {
            Some(Tag::Compound(dc)) => dc.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.as_f64()).map(|f| f as f32),
            _ => None,
        };
        let (saddle_drop, body_drop) = (drop_chance(r, "saddle"), drop_chance(r, "body"));
        let items = r.get("Items").cloned();
        let eating = r.bool_or("EatingHaystack", false);
        let bred = r.bool_or("Bred", false);
        let temper = r.int_or("Temper", 0);
        let tamed = r.bool_or("Tame", false);
        let owner = r.uuid("Owner");
        let variant = r.int_or("Variant", 0);
        let chest = r.bool_or("ChestedHorse", false);
        let (trap, trap_time) = if self.0 == Which::Skeleton { (r.bool_or("SkeletonTrap", false), r.int_or("SkeletonTrapTime", 0)) } else { (false, 0) };
        let strength = r.int_or("Strength", 0);
        let despawn_delay = r.int_or("DespawnDelay", 47999);
        let s = st_mut(m);
        if self.llama() {
            // `Llama.readAdditionalSaveData`: the strength first (the inventory depends on it),
            // then the coat.
            s.strength = strength.clamp(1, 5);
            s.llama_variant = super::llama::variant_by_id(variant);
            if self.0 == Which::TraderLlama {
                s.despawn_delay = despawn_delay;
            }
        }
        s.eating = eating;
        s.bred = bred;
        s.temper = temper;
        s.tamed = tamed;
        s.owner = owner;
        if let Some(sd) = saddle {
            s.saddle = sd;
        }
        if let Some(b) = body {
            s.body = b;
        }
        if let Some(d) = saddle_drop {
            s.saddle_drop = d;
        }
        if let Some(d) = body_drop {
            s.body_drop = d;
        }
        if self.0 == Which::Horse {
            s.type_variant = variant;
        } else if self.chested() {
            s.chest = chest;
        }
        if self.chested() {
            // `AbstractChestedHorse.readAdditionalSaveData`: `createInventory`, then the `Items`.
            self.create_inventory(m);
            let n = st(m).inventory.len();
            if st(m).chest {
                let list = kiln_inventory::persist::ItemList::load(items.as_ref(), n);
                let s = st_mut(m);
                s.inventory = list.stacks;
                s.inv_undecoded = list.undecoded;
            }
        }
        if self.0 == Which::Skeleton {
            let s = st_mut(m);
            s.trap_time = trap_time;
            super::skeleton_horse::set_trap(m, trap);
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        for (key, stack) in [("body", &s.body), ("saddle", &s.saddle)] {
            if stack.is_empty() {
                continue;
            }
            let entry = (key.to_owned(), stack.to_nbt());
            match o.0.iter_mut().find(|(k, _)| k == "equipment") {
                Some((_, Tag::Compound(eq))) => eq.push(entry),
                _ => o.put("equipment", Tag::Compound(vec![entry])),
            }
        }
        for (key, chance) in [("saddle", s.saddle_drop), ("body", s.body_drop)] {
            if chance != 0.085 {
                let entry = (key.to_owned(), Tag::Float(chance));
                match o.0.iter_mut().find(|(k, _)| k == "drop_chances") {
                    Some((_, Tag::Compound(dc))) => dc.push(entry),
                    _ => o.put("drop_chances", Tag::Compound(vec![entry])),
                }
            }
        }
        o.put("EatingHaystack", Tag::Byte(s.eating as i8));
        o.put("Bred", Tag::Byte(s.bred as i8));
        o.put("Temper", Tag::Int(s.temper));
        o.put("Tame", Tag::Byte(s.tamed as i8));
        if let Some(u) = s.owner {
            o.put("Owner", uuid_to_tag(u));
        }
        if self.llama() {
            o.put("Variant", Tag::Int(s.llama_variant));
            o.put("Strength", Tag::Int(s.strength));
            if self.0 == Which::TraderLlama {
                o.put("DespawnDelay", Tag::Int(s.despawn_delay));
            }
        }
        if self.0 == Which::Horse {
            o.put("Variant", Tag::Int(s.type_variant));
        } else if self.chested() {
            o.put("ChestedHorse", Tag::Byte(s.chest as i8));
            if s.chest {
                o.put("Items", kiln_inventory::persist::ItemList { stacks: s.inventory.clone(), undecoded: s.inv_undecoded.clone() }.save());
            }
        }
        if self.0 == Which::Skeleton {
            o.put("SkeletonTrap", Tag::Byte(s.trap as i8));
            o.put("SkeletonTrapTime", Tag::Int(s.trap_time));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        let flags = (if s.tamed { 2 } else { 0 }) | (if s.bred { 8 } else { 0 }) | (if s.eating { 16 } else { 0 }) | (if s.standing { 32 } else { 0 }) | (if s.open_mouth { 64 } else { 0 });
        d.set(data::abstract_horse::ID_FLAGS, &DataValue::Byte(flags as i8));
        if self.0 == Which::Horse {
            d.set(data::horse::ID_TYPE_VARIANT, &DataValue::Int(s.type_variant));
        } else if self.chested() {
            d.set(data::abstract_chested_horse::ID_CHEST, &DataValue::Boolean(s.chest));
        }
        if self.llama() {
            d.set(data::llama::STRENGTH, &DataValue::Int(s.strength));
            d.set(data::llama::VARIANT, &DataValue::Int(s.llama_variant));
        }
    }
}

impl Equine {
    /// `setOffspringAttributes`: health, jump strength, speed.
    fn offspring_attributes(&self, e: &mut Entity, m: &MobData, partner: &MobData, child: &mut MobData) {
        let min_speed = (0.44999998807907104f64 * 0.25) as f32 as f64;
        let max_speed = ((0.44999998807907104f64 + 0.9) * 0.25) as f32 as f64;
        let min_jump = 0.4000000059604645f32 as f64;
        let max_jump = (0.4000000059604645f64 + 0.6) as f32 as f64;
        for (a, min, max) in [(MaxHealth, 15.0, 30.0), (JumpStrength, min_jump, max_jump), (MovementSpeed, min_speed, max_speed)] {
            let v = offspring_attribute(m.attrs.base(a), partner.attrs.base(a), min, max, &mut e.random);
            set_base(child, a, v);
        }
    }

    /// `TraderLlama.maybeDespawn`: a wild trader llama goes after its despawn delay.
    fn maybe_despawn(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `canDespawn`: not tame, not led (by anyone but a wandering trader), not carrying
        // exactly one player, not kept young, not persistent.
        let one_player = e.passengers.len() == 1 && level.player(e.passengers[0]).is_some();
        // `isLeashedToWanderingTrader` / `isLeashedToSomethingOtherThanTheWanderingTrader`.
        let holder = crate::leash::holder_of(e);
        let trader = holder.and_then(|h| level.entity(h)).filter(|h| h.type_name == "minecraft:wandering_trader");
        let leashed_elsewhere = holder.is_some() && trader.is_none();
        if st(m).tamed || leashed_elsewhere || one_player || m.age_locked || m.persistence_required {
            return;
        }
        // A llama led by a wandering trader lives as long as the trader's delay.
        let delay = match trader.and_then(crate::mob::data) {
            Some(tm) => super::wandering_trader::despawn_delay(tm) - 1,
            None => st(m).despawn_delay - 1,
        };
        st_mut(m).despawn_delay = delay;
        if delay <= 0 {
            crate::leash::remove_leash(e, Some(m), level);
            e.discard();
        }
    }

    fn ai_step_horse(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !crate::mob::is_alive(e, m) {
            return;
        }
        if e.random.next_int_bounded(900) == 0 && m.death_time == 0 {
            heal(m, 1.0);
        }
        // `canEatGrass` (llamas do not graze).
        if !self.llama()
            && !st(m).eating
            && e.passengers.is_empty()
            && e.random.next_int_bounded(300) == 0
            && crate::blocks::block_name(level.block(e.block_position().below())) == "minecraft:grass_block"
        {
            st_mut(m).eating = true;
        }
        let s = st_mut(m);
        if s.eating {
            s.eating_counter += 1;
            if s.eating_counter > 50 {
                s.eating_counter = 0;
                s.eating = false;
            }
        }
        // `followMommy` only creates a path it does not follow (not modelled).
        if self.0 == Which::Skeleton {
            super::skeleton_horse::ai_step(e, m);
        }
    }

    fn post_tick_horse(&self, m: &mut MobData) {
        let s = st_mut(m);
        if s.mouth_counter > 0 {
            s.mouth_counter += 1;
            if s.mouth_counter > 30 {
                s.mouth_counter = 0;
                s.open_mouth = false;
            }
        }
        if s.stand_counter > 0 {
            s.stand_counter -= 1;
            if s.stand_counter <= 0 {
                s.standing = false;
                s.stand_counter = 0;
            }
        }
        if s.tail_counter > 0 {
            s.tail_counter += 1;
            if s.tail_counter > 8 {
                s.tail_counter = 0;
            }
        }
        if s.sprint_counter > 0 {
            s.sprint_counter += 1;
            if s.sprint_counter > 300 {
                s.sprint_counter = 0;
            }
        }
        s.eat_anim_o = s.eat_anim;
        if s.eating {
            s.eat_anim += (1.0 - s.eat_anim) * 0.4 + 0.05;
            if s.eat_anim > 1.0 {
                s.eat_anim = 1.0;
            }
        } else {
            s.eat_anim += (0.0 - s.eat_anim) * 0.4 - 0.05;
            if s.eat_anim < 0.0 {
                s.eat_anim = 0.0;
            }
        }
        s.stand_anim_o = s.stand_anim;
        if s.standing {
            s.eat_anim = 0.0;
            s.eat_anim_o = s.eat_anim;
            s.stand_anim += (1.0 - s.stand_anim) * 0.4 + 0.05;
            if s.stand_anim > 1.0 {
                s.stand_anim = 1.0;
            }
        } else {
            s.allow_stand_sliding = false;
            s.stand_anim += (0.8 * s.stand_anim * s.stand_anim * s.stand_anim - s.stand_anim) * 0.6 - 0.05;
            if s.stand_anim < 0.0 {
                s.stand_anim = 0.0;
            }
        }
        s.mouth_anim_o = s.mouth_anim;
        if s.open_mouth {
            s.mouth_anim += (1.0 - s.mouth_anim) * 0.7 + 0.05;
            if s.mouth_anim > 1.0 {
                s.mouth_anim = 1.0;
            }
        } else {
            s.mouth_anim += (0.0 - s.mouth_anim) * 0.7 - 0.05;
            if s.mouth_anim < 0.0 {
                s.mouth_anim = 0.0;
            }
        }
    }
}

/// `RunAroundLikeCrazyGoal`: an untamed horse with a rider runs about and, now and then, either
/// accepts a player rider (its temper against a roll) or throws the rider off.
#[derive(Clone, Debug)]
pub(super) struct RunAroundLikeCrazyGoal {
    speed: f64,
    pos: Vec3,
}

/// `new RunAroundLikeCrazyGoal(horse, speed)`.
pub(super) fn run_around_like_crazy(speed: f64) -> RunAroundLikeCrazyGoal {
    RunAroundLikeCrazyGoal { speed, pos: Vec3::ZERO }
}

impl CustomGoal for RunAroundLikeCrazyGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RunAroundLikeCrazyGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        // (`!isMobControlled() && !isTamed() && isVehicle()`: a zombie horse's zombie steers it.)
        if st(m).tamed || e.passengers.is_empty() || crate::mob::first_passenger_is_mob(e, &*level) && m.kind == MobKind::ZombieHorse {
            return false;
        }
        match random_pos::default_pos(e, m, level, 5, 4) {
            Some(p) => {
                self.pos = p;
                true
            }
            None => false,
        }
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        !st(m).tamed && !m.nav.is_done() && !e.passengers.is_empty()
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        path::move_to(e, m, level, self.pos.x, self.pos.y, self.pos.z, self.speed);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).tamed || e.random.next_int_bounded(reduced_tick_delay(50)) != 0 {
            return;
        }
        let Some(&first) = e.passengers.first() else { return };
        if let Some(p) = level.player(first) {
            let temper = st(m).temper;
            let max = max_temper(m);
            if e.random.next_int_bounded(max) < temper {
                // `tameWithName`.
                let s = st_mut(m);
                s.owner = Some(p.uuid);
                s.tamed = true;
                level.emit(Event::EntityEvent { entity: e.id, event: 7 });
                let animal = crate::level::Seen::of_mob(e, m);
                level.emit(Event::Criterion { player: p.id, criterion: crate::level::Criterion::TameAnimal { animal } });
                return;
            }
            let s = st_mut(m);
            s.temper = (s.temper + 5).clamp(0, max);
        }
        // `ejectPassengers` (players find out in the level's passenger pass).
        for id in std::mem::take(&mut e.passengers).into_iter().rev() {
            if let Some(o) = level.entity_mut(id) {
                o.vehicle = None;
            }
        }
        make_mad(e, m, level);
        level.emit(Event::EntityEvent { entity: e.id, event: 6 });
    }
}

/// `RandomStandGoal`: now and then the horse rears up with its ambient sound.
#[derive(Clone, Debug)]
struct RandomStandGoal {
    next_stand: i32,
}

impl CustomGoal for RandomStandGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "RandomStandGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        self.next_stand += 1;
        if self.next_stand > 0 && e.random.next_int_bounded(1000) < self.next_stand {
            self.next_stand = -400;
            let s = st(m);
            return !(s.eating || s.standing) && e.random.next_int_bounded(10) == 0;
        }
        false
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        stand(m);
        play(e, level, sound(m, "ambient"), 0.8, 1.0);
    }
}

/// `Equippable.canBeEquippedBy` and the slot: whether `stack` goes in `slot` of a `entity_type`.
pub fn equippable_in_slot(stack: &ItemStack, slot: kiln_item::component::EquipmentSlot, entity_type: &str) -> bool {
    use kiln_item::HolderSet;
    let Some(e) = stack.get(kiln_item::keys::EQUIPPABLE) else { return false };
    if e.slot != slot {
        return false;
    }
    let Some(allowed) = &e.allowed_entities else { return true };
    let Some(id) = kiln_item::registry::ENTITY_TYPE.id(entity_type) else { return false };
    match allowed {
        HolderSet::Direct(ids) => ids.contains(&id),
        HolderSet::Tag(tag) => kiln_inventory::tags::contains("minecraft:entity_type", tag.as_str(), id),
    }
}

/// `onEquipItem` for a mount: the equip sound (the saddle's own for a saddle), with the seed
/// drawn from the animal's random, unless the same item was swapped for itself.
fn equip_sound(e: &mut Entity, level: &mut dyn EntityLevel, slot: kiln_item::component::EquipmentSlot, old: &ItemStack, new: &ItemStack) {
    if let Some(sound) = equip_sound_drawn(e, slot, old, new) {
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch: 1.0 });
    }
}

/// The checks of `onEquipItem` and the draw of the sound's seed; the sound to play, if any.
fn equip_sound_drawn(e: &mut Entity, slot: kiln_item::component::EquipmentSlot, old: &ItemStack, new: &ItemStack) -> Option<&'static str> {
    use kiln_inventory::stack::same_item_same_components as same;
    if (new.is_empty() && old.is_empty()) || same(old, new) || e.first_tick {
        return None;
    }
    let equippable = new.get(kiln_item::keys::EQUIPPABLE)?;
    if e.silent || equippable.slot != slot {
        return None;
    }
    let sound = if slot == kiln_item::component::EquipmentSlot::Saddle {
        "minecraft:entity.horse.saddle"
    } else {
        match &equippable.equip_sound {
            kiln_item::Holder::Reference(id) => kiln_data::builtin_entries("minecraft:sound_event").and_then(|n| n.get(*id as usize).copied()).unwrap_or("minecraft:item.armor.equip_generic"),
            kiln_item::Holder::Direct(_) => "minecraft:item.armor.equip_generic",
        }
    };
    e.random.next_long();
    Some(sound)
}

/// The slots a mount's screen shows: saddle, body armor, then the chest's.
pub fn mount_slots(m: &MobData) -> Option<Vec<ItemStack>> {
    let s = ext::state::<State>(m)?;
    let mut v = vec![s.saddle.clone(), s.body.clone()];
    v.extend(s.inventory.iter().cloned());
    Some(v)
}

/// What the screen's slots now say, taken into the animal. A change of the saddle or the armor
/// is an `onEquipItem` now (its sound's seed comes off the animal's random at once, like the
/// packet that moved the item); the sound itself goes out at the animal's next tick.
pub fn set_mount_slots(e: &mut Entity, items: &[ItemStack]) {
    let mut changes = Vec::new();
    {
        let Some(m) = crate::mob::data_mut(e) else { return };
        let Some(s) = ext::state_mut::<State>(m) else { return };
        if items.len() != 2 + s.inventory.len() {
            return;
        }
        if !same_stack(&s.saddle, &items[0]) {
            changes.push((kiln_item::component::EquipmentSlot::Saddle, s.saddle.clone(), items[0].clone()));
        }
        if !same_stack(&s.body, &items[1]) {
            changes.push((kiln_item::component::EquipmentSlot::Body, s.body.clone(), items[1].clone()));
        }
        s.saddle = items[0].clone();
        s.body = items[1].clone();
        s.inventory.clone_from_slice(&items[2..]);
    }
    for (slot, old, new) in changes {
        if let Some(sound) = equip_sound_drawn(e, slot, &old, &new)
            && let Some(s) = crate::mob::data_mut(e).and_then(ext::state_mut::<State>)
        {
            s.pending_equip.push(sound);
        }
    }
}

fn same_stack(a: &ItemStack, b: &ItemStack) -> bool {
    kiln_inventory::stack::same_item_same_components(a, b) && a.count() == b.count()
}

/// `getInventoryColumns`, whether the saddle slot can be used (`canUseSlot(SADDLE)`: grown, tame
/// and alive) and how often the inventory was made anew, of the mount `e`.
pub fn mount_info(e: &Entity, m: &MobData) -> Option<(usize, bool, u32)> {
    let s = ext::state::<State>(m)?;
    let columns = s.inventory.len() / 3;
    // (`Llama.canUseSlot`: every slot, always.)
    let saddle_usable = super::llama::is_llama(m.kind) || m.kind == MobKind::ZombieHorse || (crate::mob::is_alive(e, m) && !m.baby() && s.tamed);
    Some((columns, saddle_usable, s.inv_serial))
}

/// `handleStartJump` (a rider's jump key on a saddled mount): the horse rears; its jump sound.
pub fn start_jump(m: &mut MobData) -> Option<&'static str> {
    if matches!(m.kind, MobKind::Camel | MobKind::CamelHusk) {
        return super::camel::start_jump(m);
    }
    if matches!(m.kind, MobKind::Nautilus | MobKind::ZombieNautilus) {
        return super::nautilus::start_jump(m);
    }
    let saddled = !ext::state::<State>(m)?.saddle.is_empty();
    if !saddled {
        return None;
    }
    st_mut(m).allow_stand_sliding = true;
    stand(m);
    Some(sound(m, "jump"))
}

/// Whether `kind` is one of the horse family.
pub fn is_equine(kind: MobKind) -> bool {
    matches!(kind, MobKind::Horse | MobKind::Donkey | MobKind::Mule | MobKind::SkeletonHorse | MobKind::ZombieHorse | MobKind::Llama | MobKind::TraderLlama)
}
