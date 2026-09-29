//! Mooshroom (`MushroomCow`, an `AbstractCow`): a cow in red or brown that gives mushroom stew
//! for a bowl, turns into a cow when sheared (dropping its mushrooms), swaps color when struck
//! by lightning, and — when brown — takes a flower to give suspicious stew next.

use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::Goal;
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{self, MobData, MobKind};
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_item::component::consume::{StewEffect, SuspiciousStewEffects};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Mooshroom;

pub static KIND: Mooshroom = Mooshroom;

static INFO: Info = Info { sounds: Some("cow"), ..Info::animal("minecraft:mooshroom", &[(MaxHealth, 10.0), (MovementSpeed, 0.20000000298023224)]) };

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `MushroomCow.Variant` (false: red).
    pub brown: bool,
    /// The flower's effects for the next bowl.
    pub stew: Option<Vec<StewEffect>>,
    /// `lastLightningBoltUUID` (the bolt's entity id here).
    last_bolt: Option<i32>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("mooshroom state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("mooshroom state")
}

pub fn is_brown(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.brown)
}

/// `FlowerBlock` suspicious stew effects: (flower item, effect, seconds).
const FLOWERS: [(&str, &str, f32); 17] = [
    ("minecraft:dandelion", "minecraft:saturation", 0.35),
    ("minecraft:golden_dandelion", "minecraft:saturation", 0.35),
    ("minecraft:torchflower", "minecraft:night_vision", 5.0),
    ("minecraft:poppy", "minecraft:night_vision", 5.0),
    ("minecraft:blue_orchid", "minecraft:saturation", 0.35),
    ("minecraft:allium", "minecraft:fire_resistance", 3.0),
    ("minecraft:azure_bluet", "minecraft:blindness", 11.0),
    ("minecraft:red_tulip", "minecraft:weakness", 7.0),
    ("minecraft:orange_tulip", "minecraft:weakness", 7.0),
    ("minecraft:white_tulip", "minecraft:weakness", 7.0),
    ("minecraft:pink_tulip", "minecraft:weakness", 7.0),
    ("minecraft:oxeye_daisy", "minecraft:regeneration", 7.0),
    ("minecraft:cornflower", "minecraft:jump_boost", 5.0),
    ("minecraft:wither_rose", "minecraft:wither", 7.0),
    ("minecraft:lily_of_the_valley", "minecraft:poison", 11.0),
    ("minecraft:open_eyeblossom", "minecraft:blindness", 11.0),
    ("minecraft:closed_eyeblossom", "minecraft:nausea", 7.0),
];

/// `SuspiciousEffectHolder.tryGet(item).getSuspiciousEffects()`.
fn flower_effects(stack: &ItemStack) -> Option<Vec<StewEffect>> {
    let name = mob::item_name(stack);
    let &(_, effect, seconds) = FLOWERS.iter().find(|(f, _, _)| *f == name)?;
    let effect = kiln_data::builtin_id("minecraft:mob_effect", effect)?;
    Some(vec![StewEffect { effect, duration: kiln_javamath::math::floor_f32(seconds * 20.0) }])
}

impl Kind for Mooshroom {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    /// `AbstractCow.registerGoals`.
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Panic { speed: 2.0, pos: Vec3::ZERO });
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(3, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
        g.add(4, Goal::FollowParent { speed: 1.25, parent: None, recalc: 0 });
        g.add(5, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: Some(0.001), wanted: Vec3::ZERO, force: false });
        g.add(6, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
        g.add(7, Goal::RandomLookAround { rel_x: 0.0, rel_z: 0.0, look_time: 0 });
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:cow_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:cow_food")
    }

    fn sound_volume(&self, _m: &MobData) -> f32 {
        0.4
    }

    /// `getWalkTargetValue`: mycelium, else the light.
    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        if crate::blocks::block_name(level.block(p.below())) == "minecraft:mycelium" {
            return Some(10.0);
        }
        Some(mob::light_magic_value_at(level, p) - 0.5)
    }

    /// `checkMushroomSpawnRules`: on mycelium (`#mooshrooms_spawnable_on`) in the light.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        let below = view.block(pos.below());
        let on = kiln_data::builtin_id("minecraft:block", crate::blocks::block_name(below)).is_some_and(|id| block_tag(id, "minecraft:mooshrooms_spawnable_on"));
        Some(on && view.raw_brightness(pos, 0) > 8)
    }

    /// `thunderHit`: red and brown swap once per bolt.
    fn thunder_hit(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, bolt: i32) -> bool {
        let s = st_mut(m);
        if s.last_bolt != Some(bolt) {
            s.brown = !s.brown;
            s.last_bolt = Some(bolt);
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.mooshroom.convert", source: "neutral", volume: 2.0, pitch: 1.0 });
            }
        }
        true
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let item = if stack.is_empty() { "minecraft:air" } else { mob::item_name(stack) };
        let sound = |e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, volume: f32| {
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume, pitch: 1.0 });
            }
        };
        if item == "minecraft:bowl" && !m.baby() {
            let (stew, milk) = match st_mut(m).stew.take() {
                Some(effects) => {
                    let mut s = ItemStack::of("minecraft:suspicious_stew", 1)?;
                    s.insert(kiln_item::keys::SUSPICIOUS_STEW_EFFECTS, SuspiciousStewEffects(effects));
                    (s, "minecraft:entity.mooshroom.suspicious_milk")
                }
                None => (ItemStack::of("minecraft:mushroom_stew", 1)?, "minecraft:entity.mooshroom.milk"),
            };
            sound(e, level, milk, 1.0);
            return Some(Outcome::success(HeldChange::Fill(stew)));
        }
        if item == "minecraft:shears" && !m.baby() {
            // `shear`: the sound, then the cow in its place and the mushrooms.
            if !e.silent {
                level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.mooshroom.shear", source: "players", volume: 1.0, pitch: 1.0 });
            }
            let table = if st(m).brown { "minecraft:shearing/mooshroom/brown" } else { "minecraft:shearing/mooshroom/red" };
            let pos = e.position();
            let height = e.height as f64;
            let converted = crate::mob::convert::convert_to(e, m, level, MobKind::Cow, false, false, |_, _, _| {});
            if converted.is_some() {
                // `spawnAtLocation` at the top of the mooshroom (the drop goes 1 up from `pos`).
                level.emit(Event::ShearLoot { entity: e.id, table: table.into(), pos: Vec3::new(pos.x, pos.y + height - 1.0, pos.z) });
            }
            level.emit(Event::GameEvent { event: "minecraft:shear", pos: e.position(), entity: Some(who.id) });
            return Some(Outcome::success(HeldChange::Damage(1)));
        }
        if st(m).brown && !m.baby()
            && let Some(effects) = flower_effects(stack)
        {
            if st(m).stew.is_some() {
                return Some(Outcome::success(HeldChange::None));
            }
            st_mut(m).stew = Some(effects);
            sound(e, level, "minecraft:entity.mooshroom.eat", 2.0);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        // `AbstractCow.mobInteract`: milk for a bucket.
        if item == "minecraft:bucket" && !m.baby() {
            let mut out = Outcome::success(HeldChange::Fill(ItemStack::of("minecraft:milk_bucket", 1)?));
            out.player_sound = Some("minecraft:entity.cow.milk");
            return Some(out);
        }
        None
    }

    /// `getOffspringVariant`: one in 1024 same-colored pairs have the other color, else a parent's.
    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        let mine = st(m).brown;
        let theirs = is_brown(partner);
        let baby = if mine == theirs && e.random.next_int_bounded(1024) == 0 {
            !mine
        } else if e.random.next_bool() {
            mine
        } else {
            theirs
        };
        st_mut(child).brown = baby;
    }

    /// `MushroomCow.BABY_DIMENSIONS`.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.45, 0.7, 0.69) } else { base }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let brown = r.get("Type").and_then(Tag::as_str) == Some("brown");
        let stew = match r.get("stew_effects") {
            Some(Tag::List(l)) => Some(
                l.iter()
                    .filter_map(|t| {
                        let effect = kiln_data::builtin_id("minecraft:mob_effect", t.get("id")?.as_str()?)?;
                        let duration = t.get("duration").and_then(Tag::as_f64).map_or(160, |d| d as i32);
                        Some(StewEffect { effect, duration })
                    })
                    .collect(),
            ),
            _ => None,
        };
        let s = st_mut(m);
        s.brown = brown;
        s.stew = stew;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("Type", Tag::String(if s.brown { "brown" } else { "red" }.into()));
        if let Some(effects) = &s.stew {
            let list = effects
                .iter()
                .filter_map(|x| {
                    let name = kiln_data::builtin_entries("minecraft:mob_effect")?.get(x.effect as usize)?;
                    Some(Tag::Compound(vec![("id".into(), Tag::String((*name).into())), ("duration".into(), Tag::Int(x.duration))]))
                })
                .collect();
            o.put("stew_effects", Tag::List(list));
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        d.set(kiln_data::entities::data::mushroom_cow::TYPE, &DataValue::Int(st(m).brown as i32));
    }
}

/// Whether block id `id` is in the `minecraft:block` tag `tag`.
fn block_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:block")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}
