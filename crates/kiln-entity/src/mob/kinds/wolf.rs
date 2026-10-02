//! Wolf: tamed with bones (one in three), sits when told, follows and defends its owner, hunts
//! sheep while wild, gets angry at whoever hurts it (`NeutralMob`), shakes itself dry; variants
//! by biome, collar dyes, healing with meat, breeding between tamed wolves.

use super::anger::{Anger, AngryAtPlayerGoal};
use super::tame::{self, FollowOwnerGoal, NonTameRandomTargetGoal, OwnerTargetGoal, SitWhenOrderedToGoal, Tame, TamableAnimalPanicGoal};
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event, PlayerView};
use crate::mob::attributes::Attr::{self, *};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, Living, MeleeKind, Wanted, LOOK};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::{self, reduced_tick_delay};
use crate::mob::path::PathType;
use crate::mob::{DamageSource, GroupData, MobData, MobKind, SpawnContext, item_name, item_tag};
use crate::math::{BlockPos, Vec3};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Wolf;

pub static KIND: Wolf = Wolf;

static INFO: Info = Info::animal("minecraft:wolf", &[(MovementSpeed, 0.30000001192092896), (MaxHealth, 8.0), (AttackDamage, 4.0)]);

/// `DyeColor.RED`.
const DEFAULT_COLLAR: u8 = 14;

#[derive(Clone, Debug)]
pub struct State {
    pub tame: Tame,
    /// `DATA_INTERESTED_ID` (begging).
    pub interested: bool,
    pub collar: u8,
    /// `NeutralMob`: `DATA_ANGER_END_TIME` and the persistent anger target.
    pub anger: Anger,
    pub wet: bool,
    pub shaking: bool,
    pub shake_anim: f32,
    pub shake_anim_o: f32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("wolf state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("wolf state")
}

/// `NeutralMob.isAngry`.
pub fn is_angry(m: &MobData, level: &dyn EntityLevel) -> bool {
    super::anger::is_angry(m, level)
}

/// `Wolf.wantsToAttack(target, owner)`.
pub fn wants_to_attack(level: &dyn EntityLevel, t: &Living, owner: &PlayerView) -> bool {
    if matches!(t.type_name, "minecraft:creeper" | "minecraft:ghast" | "minecraft:armor_stand") {
        return false;
    }
    if t.player {
        // `canHarmPlayer`: no teams, so players can always harm each other.
        return true;
    }
    let Some(om) = level.entity(t.id).and_then(crate::mob::data) else { return true };
    if om.kind == MobKind::Wolf {
        return !tame::is_tame(om) || tame::get(om).and_then(|x| x.owner) != Some(owner.uuid);
    }
    if super::horse::is_equine(om.kind) && super::horse::is_tamed(om) {
        return false;
    }
    !tame::is_tame(om)
}

/// A synchronized registry entry's name.
pub fn synced_name(registry: &str, id: i32) -> Option<&'static str> {
    let (_, entries) = kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry)?;
    entries.get(id as usize).copied()
}

fn synced_len(registry: &str) -> i32 {
    kiln_data::registries::SYNCHRONIZED.iter().find(|(r, _)| *r == registry).map_or(1, |(_, e)| e.len() as i32)
}

/// Whether biome `id` is `spec` (a biome name, or `#tag`).
pub fn biome_is(id: i32, spec: &str) -> bool {
    match spec.strip_prefix('#') {
        Some(tag) => kiln_data::registries::TAGS
            .iter()
            .find(|(r, _)| *r == "minecraft:worldgen/biome")
            .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
            .is_some_and(|(_, ids)| ids.contains(&id)),
        None => synced_name("minecraft:worldgen/biome", id) == Some(spec),
    }
}

/// Whether block state `state` is in the block tag `tag`.
///
/// Sensors ask this for every block of a box (piglins look for repellents in 2601 blocks every
/// 20 ticks), so each thread keeps a bit per block state for the tags it has been asked about.
pub fn block_in_tag(state: u16, tag: &str) -> bool {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(String, Box<[u64]>)>> = const { RefCell::new(Vec::new()) };
    }
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let i = match cache.iter().position(|(t, _)| t == tag) {
            Some(i) => i,
            None => {
                let ids = kiln_data::registries::TAGS
                    .iter()
                    .find(|(r, _)| *r == "minecraft:block")
                    .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
                    .map(|(_, ids)| *ids);
                let count = kiln_data::blocks::STATE_COUNT as usize;
                let mut bits = vec![0u64; count.div_ceil(64)].into_boxed_slice();
                if let Some(ids) = ids {
                    for s in 0..count {
                        let name = crate::blocks::block_name(s as u16);
                        if kiln_data::builtin_id("minecraft:block", name).is_some_and(|id| ids.contains(&id)) {
                            bits[s >> 6] |= 1 << (s & 63);
                        }
                    }
                }
                cache.push((tag.to_owned(), bits));
                cache.len() - 1
            }
        };
        let s = state as usize;
        cache[i].1.get(s >> 6).is_some_and(|w| w >> (s & 63) & 1 == 1)
    })
}

/// `WolfVariants` spawn conditions (priority 1 by biome, else pale).
const VARIANT_BIOMES: &[(&str, &str)] = &[
    ("minecraft:ashen", "minecraft:snowy_taiga"),
    ("minecraft:black", "minecraft:old_growth_pine_taiga"),
    ("minecraft:chestnut", "minecraft:old_growth_spruce_taiga"),
    ("minecraft:rusty", "#minecraft:is_jungle"),
    ("minecraft:snowy", "minecraft:grove"),
    ("minecraft:spotted", "#minecraft:is_savanna"),
    ("minecraft:striped", "#minecraft:is_badlands"),
    ("minecraft:woods", "minecraft:forest"),
];

/// `VariantUtils.selectVariantToSpawn` for wolves: the candidates of the highest matching
/// priority (registry order), one picked with the level's random.
fn spawn_variant(biome: Option<i32>, r: &mut dyn RandomSource) -> i32 {
    let matching: Vec<&str> = VARIANT_BIOMES.iter().filter(|(_, b)| biome.is_some_and(|id| biome_is(id, b))).map(|(v, _)| *v).collect();
    let candidates = if matching.is_empty() { vec!["minecraft:pale"] } else { matching };
    let pick = candidates[r.next_int_bounded(candidates.len() as i32) as usize];
    kiln_data::synced_id("minecraft:wolf_variant", pick).unwrap_or(0)
}

/// The wolf's `WolfSoundSet` sound `what` (ambient, growl, pant, whine, hurt, death).
fn sound(m: &MobData, what: &str) -> &'static str {
    let v = synced_name("minecraft:wolf_sound_variant", m.sound_variant).unwrap_or("minecraft:classic");
    let set = if v == "minecraft:classic" { "wolf".to_owned() } else { format!("wolf_{}", &v[10..]) };
    crate::mob::sound_event(&format!("minecraft:entity.{set}.{what}"))
}

/// `Wolf.applyTamingSideEffects`.
fn apply_taming_side_effects(m: &mut MobData) {
    if tame::is_tame(m) {
        if let Some(i) = m.attrs.get_mut(Attr::MaxHealth) {
            i.base = 40.0;
        }
        m.set_health(40.0);
    } else if let Some(i) = m.attrs.get_mut(Attr::MaxHealth) {
        i.base = 8.0;
    }
}

fn is(stack: &ItemStack, name: &str) -> bool {
    !stack.is_empty() && item_name(stack) == name
}

fn heal(m: &mut MobData, amount: f32) {
    if m.health > 0.0 {
        let h = m.health + amount;
        m.set_health(h);
    }
}

impl Kind for Wolf {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        tame::set_malus(m, PathType::PowderSnow, -1.0);
        tame::set_malus(m, PathType::OnTopOfPowderSnow, -1.0);
        m.variant = kiln_data::synced_id("minecraft:wolf_variant", "minecraft:pale").unwrap_or(0);
        m.sound_variant = kiln_data::synced_id("minecraft:wolf_sound_variant", "minecraft:classic").unwrap_or(0);
        Some(Box::new(State {
            tame: Tame::default(),
            interested: false,
            collar: DEFAULT_COLLAR,
            anger: Anger::default(),
            wet: false,
            shaking: false,
            shake_anim: 0.0,
            shake_anim_o: 0.0,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(1, Goal::Custom(Box::new(TamableAnimalPanicGoal::new(1.5, "minecraft:panic_environmental_causes"))));
        g.add(2, Goal::Custom(Box::new(SitWhenOrderedToGoal)));
        // `WolfAvoidEntityGoal` runs from llamas, which Kiln does not simulate.
        g.add(3, Goal::AvoidEntity);
        g.add(4, Goal::LeapAtTarget { yd: 0.4, target: None });
        g.add(
            5,
            Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: true, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 },
        );
        g.add(6, Goal::Custom(Box::new(FollowOwnerGoal::new(1.0, 10.0, 2.0))));
        g.add(7, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(8, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(9, Goal::Custom(Box::new(BegGoal { look_distance: 8.0, player: None, look_time: 0 })));
        g.add(10, Goal::LookAtPlayer { dist: 8.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(10, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::Custom(Box::new(OwnerTargetGoal::new(true))));
        t.add(2, Goal::Custom(Box::new(OwnerTargetGoal::new(false))));
        t.add(3, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(4, Goal::Custom(Box::new(AngryAtPlayerGoal::default())));
        t.add(5, Goal::Custom(Box::new(NonTameRandomTargetGoal::new(&["minecraft:sheep", "minecraft:rabbit", "minecraft:fox"], false))));
        // Baby turtles on land (turtles are not simulated).
        t.add(6, Goal::Custom(Box::new(NonTameRandomTargetGoal::new(&[], false))));
        t.add(
            7,
            Goal::NearestAttackable {
                wanted: Wanted::Types(&["minecraft:skeleton", "minecraft:stray", "minecraft:wither_skeleton", "minecraft:bogged", "minecraft:parched"]),
                interval: reduced_tick_delay(10),
                must_see: false,
                target: None,
                unseen: 0,
                spider: false,
            },
        );
        // `ResetUniversalAngerTargetGoal`: the `universal_anger` game rule is off.
        t.add(8, Goal::Never);
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !crate::mob::is_alive(e, m) {
            return;
        }
        let wet_now = e.is_in_water() || level.is_raining_at(e.block_position());
        let id = e.id;
        let s = st_mut(m);
        if wet_now {
            s.wet = true;
            if s.shaking {
                level.emit(Event::EntityEvent { entity: id, event: 56 });
                s.shaking = false;
                s.shake_anim = 0.0;
                s.shake_anim_o = 0.0;
            }
        } else if s.shaking {
            if s.shake_anim == 0.0 {
                let pitch = (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0;
                if !e.silent {
                    level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.wolf.shake", source: "neutral", volume: 0.4, pitch });
                }
                level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: e.position(), entity: Some(id) });
            }
            s.shake_anim_o = s.shake_anim;
            s.shake_anim += 0.05;
            if s.shake_anim_o >= 2.0 {
                s.wet = false;
                s.shaking = false;
                s.shake_anim_o = 0.0;
                s.shake_anim = 0.0;
            }
            if s.shake_anim > 0.4 {
                let count = (mth::sin(((s.shake_anim - 0.4) * 3.1415927) as f64) * 7.0) as i32;
                for _ in 0..count {
                    e.random.next_float();
                    e.random.next_float();
                }
            }
        }
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let pathing = !m.nav.is_done();
        let s = st_mut(m);
        if s.wet && !s.shaking && !pathing && e.on_ground {
            s.shaking = true;
            s.shake_anim = 0.0;
            s.shake_anim_o = 0.0;
            level.emit(Event::EntityEvent { entity: e.id, event: 8 });
        }
        super::anger::update_persistent_anger(e, m, level);
    }

    fn ambient_sound(&self, e: &mut Entity, m: &MobData, level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        if is_angry(m, level) {
            return Some(Some(sound(m, "growl")));
        }
        if e.random.next_int_bounded(3) == 0 {
            if tame::is_tame(m) && m.health < 20.0 {
                return Some(Some(sound(m, "whine")));
            }
            return Some(Some(sound(m, "pant")));
        }
        Some(Some(sound(m, "ambient")))
    }

    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        tame::set_ordered_to_sit(m, false);
        None
    }

    fn can_attack(&self, m: &MobData, _level: &dyn EntityLevel, t: &Living) -> bool {
        // `TamableAnimal.canAttack`: never the owner.
        !(t.player && tame::get(m).and_then(|x| x.owner).is_some_and(|u| _level.player(t.id).is_some_and(|p| p.uuid == u)))
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        tame::is_tame(m) && tame::is_tame(partner) && !tame::get(partner).is_some_and(|t| t.sitting)
    }

    fn max_head_x_rot(&self, m: &MobData) -> i32 {
        if tame::get(m).is_some_and(|t| t.sitting) { 20 } else { 40 }
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:wolf_food")
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.3, 0.425, 0.34375) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        let variant = match group.variant {
            Some(v) => v,
            None => {
                let v = spawn_variant(ctx.biome, r);
                group.variant = Some(v);
                v
            }
        };
        m.variant = variant;
        m.sound_variant = r.next_int_bounded(synced_len("minecraft:wolf_sound_variant"));
        // `AgeableMob.finalizeSpawn` with a `WolfPackData` (no babies).
        group.ageable_group_size += 1;
        let _ = e;
        ext::mob_finalize(m, r);
    }

    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(block_in_tag(view.block(pos.below()), "minecraft:wolves_spawnable_on") && view.raw_brightness(pos, 0) > 8)
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let owned = tame::owned_by(m, level, who.id);
        if tame::is_tame(m) {
            if !stack.is_empty() && self.is_food(stack.item()) && m.health < m.max_health() {
                // `feed(player, hand, stack, 2, 2)`.
                let nutrition = stack.get(kiln_item::keys::FOOD).map(|f| f.nutrition);
                heal(m, nutrition.map_or(2.0, |n| 2.0 * n as f32));
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            if !stack.is_empty() && item_tag(stack.item(), "minecraft:wolf_collar_dyes") && owned {
                if let Some(dye) = crate::mob::interact::dye_color(stack)
                    && dye != st(m).collar
                {
                    st_mut(m).collar = dye;
                    return Some(Outcome::success(HeldChange::Consume(1)));
                }
                return Some(crate::mob::interact::animal_interact(e, m, level, who, stack));
            }
            // Wolf armor is not modelled: straight to `Animal.mobInteract`, then sit or stand.
            let out = crate::mob::interact::animal_interact(e, m, level, who, stack);
            if !out.success && owned {
                let sit = !tame::ordered_to_sit(m);
                tame::set_ordered_to_sit(m, sit);
                m.jumping = false;
                m.nav.stop();
                m.target = None;
                return Some(Outcome::success(HeldChange::None));
            }
            return Some(out);
        }
        if is(stack, "minecraft:bone") && !is_angry(m, level) {
            // `tryToTame`.
            if e.random.next_int_bounded(3) == 0 {
                let uuid = level.player(who.id).map_or(0, |p| p.uuid);
                tame::tame(m, uuid);
                apply_taming_side_effects(m);
                m.nav.stop();
                m.target = None;
                tame::set_ordered_to_sit(m, true);
                level.emit(Event::EntityEvent { entity: e.id, event: 7 });
                let animal = crate::level::Seen::of_mob(e, m);
                level.emit(Event::Criterion { player: who.id, criterion: crate::level::Criterion::TameAnimal { animal } });
            } else {
                level.emit(Event::EntityEvent { entity: e.id, event: 6 });
            }
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        None
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, level: &mut dyn EntityLevel) {
        child.variant = if e.random.next_bool() { m.variant } else { partner.variant };
        if tame::is_tame(m) {
            let owner = tame::get(m).and_then(|t| t.owner);
            if let Some(t) = tame::get_mut(child) {
                t.owner = owner;
                t.tame = true;
            }
            apply_taming_side_effects(child);
            let (a, b) = (st(m).collar, ext::state::<State>(partner).map_or(DEFAULT_COLLAR, |s| s.collar));
            // `DyeColor.getMixedColor`: the crafted mix, else one of them (the level's random).
            let mixed = crate::mob::breed::mixed_dye(a, b).unwrap_or_else(|| if level.random().next_bool() { a } else { b });
            st_mut(child).collar = mixed;
        }
        child.sound_variant = e.random.next_int_bounded(synced_len("minecraft:wolf_sound_variant"));
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let side_effects = tame::load(&mut st_mut(m).tame, r);
        if side_effects {
            apply_taming_side_effects(m);
        }
        if let Some(v) = r.get("variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:wolf_variant", v)) {
            m.variant = v;
        }
        st_mut(m).collar = r.byte_or("CollarColor", DEFAULT_COLLAR as i8) as u8 & 15;
        st_mut(m).anger.end = match r.get("anger_end_time") {
            Some(Tag::Long(t)) => *t,
            _ => -1,
        };
        if let Some(v) = r.get("sound_variant").and_then(Tag::as_str).and_then(|v| kiln_data::synced_id("minecraft:wolf_sound_variant", v)) {
            m.sound_variant = v;
        }
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        tame::save(&s.tame, o);
        o.put("CollarColor", Tag::Byte(s.collar as i8));
        if let Some(v) = synced_name("minecraft:wolf_variant", m.variant) {
            o.put("variant", Tag::String(v.to_owned()));
        }
        o.put("anger_end_time", Tag::Long(s.anger.end));
        if let Some(v) = synced_name("minecraft:wolf_sound_variant", m.sound_variant) {
            o.put("sound_variant", Tag::String(v.to_owned()));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        tame::entity_data(&s.tame, d);
        d.set(data::wolf::INTERESTED, &DataValue::Boolean(s.interested));
        d.set(data::wolf::COLLAR_COLOR, &DataValue::Int(s.collar as i32));
        d.set(data::wolf::ANGER_END_TIME, &DataValue::Long(s.anger.end));
        d.set(data::wolf::VARIANT, &DataValue::Holder(m.variant));
        d.set(data::wolf::SOUND_VARIANT, &DataValue::Holder(m.sound_variant));
    }
}

/// `BegGoal`: looks at a nearby player holding a bone or meat.
#[derive(Clone, Debug)]
struct BegGoal {
    look_distance: f32,
    player: Option<i32>,
    look_time: i32,
}

fn holding_interesting(p: &PlayerView) -> bool {
    let bone = kiln_data::builtin_id("minecraft:item", "minecraft:bone");
    [p.main_hand, p.off_hand].iter().any(|&i| i > 0 && (Some(i) == bone || item_tag(i, "minecraft:wolf_food")))
}

impl CustomGoal for BegGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "BegGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        self.player = goals::nearest_player(e, m, level, false, self.look_distance as f64, true, |_| true).map(|p| p.id);
        self.player.and_then(|id| level.player(id)).is_some_and(|p| holding_interesting(&p))
    }
    fn can_continue(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return false };
        if !p.alive {
            return false;
        }
        if e.position().distance_to_sqr(p.pos) > (self.look_distance * self.look_distance) as f64 {
            return false;
        }
        self.look_time > 0 && holding_interesting(&p)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).interested = true;
        self.look_time = reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).interested = false;
        self.player = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.player.and_then(|id| level.player(id)) else { return };
        let max_x = m.max_head_x_rot() as f32;
        m.look.set_look_at(p.pos.x, p.pos.y + p.eye_height as f64, p.pos.z, 10.0, max_x);
        self.look_time -= 1;
    }
}

#[cfg(test)]
mod tag_tests {
    use super::block_in_tag;
    use kiln_data::blocks::default_state as d;

    #[test]
    fn block_tags_answer_for_states_through_the_cache() {
        // (twice: the second answer comes from the cached bits)
        for _ in 0..2 {
            assert!(block_in_tag(d::SOUL_FIRE, "minecraft:piglin_repellents"));
            assert!(!block_in_tag(d::STONE, "minecraft:piglin_repellents"));
            assert!(!block_in_tag(d::SOUL_FIRE, "minecraft:no_such_tag"));
            assert!(!block_in_tag(u16::MAX, "minecraft:piglin_repellents"));
        }
    }
}
