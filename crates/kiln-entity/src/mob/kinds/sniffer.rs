//! Sniffer: an ancient animal that sniffs the air, walks to a spot of diggable soil, digs up a
//! seed (torchflower seeds or a pitcher pod, from `gameplay/sniffer_digging`), rises happy and
//! rests for 8 minutes before sniffing again; bred with torchflower seeds it lays an egg.
//!
//! Approximation: vanilla drives it with a `Brain` (`SnifferAi`); here one goal runs the same
//! states and timings (scenting 50-80 ticks, sniffing 80-100, searching up to 600, digging
//! 160-180 with the seed at 120, rising 20, feeling happy 20-30) next to the usual animal goals.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{Goal, JUMP, LOOK, MOVE};
use crate::mob::{self, MobData, mth, path, random_pos};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Sniffer;

pub static KIND: Sniffer = Sniffer;

static INFO: Info = Info { head: (50, 40, 10), ..Info::animal("minecraft:sniffer", &[(MovementSpeed, 0.10000000149011612), (MaxHealth, 14.0)]) };

/// `Sniffer.State` ids.
pub const IDLING: i32 = 0;
pub const FEELING_HAPPY: i32 = 1;
pub const SCENTING: i32 = 2;
pub const SNIFFING: i32 = 3;
pub const SEARCHING: i32 = 4;
pub const DIGGING: i32 = 5;
pub const RISING: i32 = 6;

#[derive(Clone, Debug, Default)]
pub struct State {
    pub state: i32,
    drop_seed_at: i32,
    /// `SNIFF_COOLDOWN` (game time it ends) and `SNIFFER_EXPLORED_POSITIONS`.
    sniff_cooldown_until: i64,
    explored: Vec<BlockPos>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("sniffer state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("sniffer state")
}

fn play(e: &Entity, level: &mut dyn EntityLevel, sound: &'static str, pitch: f32) {
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch });
    }
}

/// `transitionTo`: the state with its sound (and the digging's seed time).
fn transition(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, to: i32) {
    match to {
        FEELING_HAPPY => play(e, level, "minecraft:entity.sniffer.happy", 1.0),
        SCENTING => play(e, level, "minecraft:entity.sniffer.scenting", if m.baby() { 1.3 } else { 1.0 }),
        SNIFFING => play(e, level, "minecraft:entity.sniffer.sniffing", 1.0),
        DIGGING => {
            st_mut(m).drop_seed_at = e.tick_count + 120;
            level.emit(Event::EntityEvent { entity: e.id, event: 63 });
        }
        RISING => play(e, level, "minecraft:entity.sniffer.digging_stop", 1.0),
        _ => {}
    }
    let was_digging = st(m).state == DIGGING;
    st_mut(m).state = to;
    if was_digging || to == DIGGING {
        mob::refresh_dimensions_in(e, m, &*level);
    }
}

/// `getHeadBlock`: 2.25 ahead, 0.2 up.
fn head_block(e: &Entity) -> BlockPos {
    let f = crate::ext_entity::fireball::view_vector(0.0, e.y_rot);
    BlockPos::containing(e.x() + f.x * 2.25, e.y() + 0.20000000298023224, e.z() + f.z * 2.25)
}

/// `canSniff`.
fn can_sniff(e: &Entity, m: &MobData) -> bool {
    let panicking = m.goals.is_running(|g| matches!(g, Goal::Panic { .. }));
    let tempted = m.goals.is_running(|g| matches!(g, Goal::Tempt { .. }));
    !tempted && !panicking && !e.is_in_water() && m.in_love <= 0 && e.on_ground && e.vehicle.is_none()
}

/// `canDig(pos)`: diggable soil not dug before.
fn diggable(m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> bool {
    super::wolf::block_in_tag(level.block(p), "minecraft:sniffer_diggable_block") && !st(m).explored.contains(&p)
}

impl Kind for Sniffer {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.nav.can_float = true;
        m.maluses.push((path::PathType::Water, -1.0));
        m.maluses.push((path::PathType::OnTopOfPowderSnow, -1.0));
        m.maluses.push((path::PathType::DamageCautious, -1.0));
        Some(Box::new(State::default()))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(1, Goal::Panic { speed: 2.0, pos: Vec3::ZERO });
        g.add(2, Goal::Breed { speed: 1.0, partner: None, love_time: 0 });
        g.add(3, Goal::Tempt { speed: 1.25, calm_down: 0, player: None });
        g.add(4, Goal::Custom(Box::new(SnifferDig { ticks: 0, target: None })));
        g.add(5, Goal::FollowParent { speed: 1.25, parent: None, recalc: 0 });
        g.add(6, Goal::RandomStroll { speed: 1.0, interval: 120, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false });
        g.add(7, Goal::LookAtPlayer { dist: 6.0, probability: 0.02, look_at: None, look_time: 0 });
    }

    fn is_food(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:sniffer_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        mob::item_tag(item, "minecraft:sniffer_food")
    }

    /// `Sniffer.tick`: the seed comes up at its time.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).state == DIGGING && st(m).drop_seed_at == e.tick_count {
            let h = head_block(e);
            level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/sniffer_digging", pos: Vec3::new(h.x as f64, h.y as f64, h.z as f64) });
            play(e, level, "minecraft:entity.sniffer.drop_seed", 1.0);
        }
        if st(m).state == DIGGING && e.tick_count % 10 == 0 {
            let h = head_block(e);
            level.emit(Event::GameEvent { event: "minecraft:entity_action", pos: Vec3::new(h.x as f64 + 0.5, h.y as f64 + 0.5, h.z as f64 + 0.5), entity: Some(e.id) });
        }
    }

    /// `jumpFromGround`: a nudge forward when jumping from a standstill.
    fn jump_from_ground(&self, e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) -> bool {
        let power = m.attrs.value(JumpStrength) as f32 * e.block_jump_factor(level) + mob::effects::jump_boost_power(m);
        if power > 1.0e-5 {
            e.delta = Vec3::new(e.delta.x, (power as f64).max(e.delta.y), e.delta.z);
            e.needs_sync = true;
        }
        if m.mov.speed_modifier > 0.0 && e.delta.horizontal_distance_sqr() < 0.01 {
            mob::move_relative(e, 0.1, Vec3::new(0.0, 0.0, 1.0));
        }
        true
    }

    fn can_mate(&self, m: &MobData, partner: &MobData) -> bool {
        let ok = |x: &MobData| ext::state::<State>(x).is_some_and(|s| matches!(s.state, IDLING | SCENTING | FEELING_HAPPY));
        ok(m) && ok(partner)
    }

    /// `spawnChildFromBreeding`: an egg, not a baby.
    fn breed_as_item(&self) -> Option<&'static str> {
        Some("minecraft:sniffer_egg")
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        Some(if matches!(st(m).state, DIGGING | SEARCHING) { None } else { Some("minecraft:entity.sniffer.idle") })
    }

    fn die(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &mob::DamageSource) {
        st_mut(m).state = IDLING;
    }

    /// `DIGGING_DIMENSIONS`: 0.4 lower, eyes at 0.81.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        let (w, h, eye) = if st(m).state == DIGGING { (base.0, base.1 - 0.4, 0.81) } else { base };
        if m.baby() { (w * 0.5, h * 0.5, eye * 0.5) } else { (w, h, eye) }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let explored = match r.get("Brain").and_then(|b| b.get("memories")).and_then(|mm| mm.get("minecraft:sniffer_explored_positions")).and_then(|v| v.get("value")) {
            Some(Tag::List(l)) => l
                .iter()
                .filter_map(|g| match g.get("pos") {
                    Some(Tag::IntArray(a)) if a.len() == 3 => Some(BlockPos::new(a[0], a[1], a[2])),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        st_mut(m).explored = explored;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        let mut memories = Vec::new();
        if !s.explored.is_empty() {
            let list = s
                .explored
                .iter()
                .map(|p| Tag::Compound(vec![("dimension".into(), Tag::String("minecraft:overworld".into())), ("pos".into(), Tag::IntArray(vec![p.x, p.y, p.z]))]))
                .collect();
            memories.push(("minecraft:sniffer_explored_positions".to_owned(), Tag::Compound(vec![("value".into(), Tag::List(list))])));
        }
        o.put("Brain", Tag::Compound(vec![("memories".into(), Tag::Compound(memories))]));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::sniffer as f;
        let s = st(m);
        d.set(f::STATE, &DataValue::Enum(s.state));
        d.set(f::DROP_SEED_AT_TICK, &DataValue::Int(s.drop_seed_at));
    }
}

/// The sniff activity as one goal: scent or sniff, walk to diggable soil, dig, rise, be happy.
#[derive(Clone, Debug)]
struct SnifferDig {
    ticks: i32,
    target: Option<BlockPos>,
}

impl CustomGoal for SnifferDig {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SnifferDig"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.baby() || !can_sniff(e, m) || st(m).sniff_cooldown_until > level.game_time() {
            return false;
        }
        if e.random.next_int_bounded(mth::reduced_tick_delay(200)) != 0 {
            return false;
        }
        // Scenting (idle) or sniffing (the dig search), one of two.
        let (state, t) = if e.random.next_bool() { (SCENTING, mth::next_int_between(&mut e.random, 50, 80)) } else { (SNIFFING, mth::next_int_between(&mut e.random, 80, 100)) };
        m.nav.stop();
        transition(e, m, level, state);
        self.ticks = t;
        self.target = None;
        true
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        st(m).state != IDLING && (st(m).state != SEARCHING || can_sniff(e, m))
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if st(m).state != IDLING {
            transition(e, m, level, IDLING);
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        self.ticks -= 1;
        let state = st(m).state;
        match state {
            SCENTING | FEELING_HAPPY if self.ticks <= 0 => transition(e, m, level, IDLING),
            SNIFFING if self.ticks <= 0 => {
                // `calculateDigPosition`: land spots 10 to 18 away whose block below can be dug.
                let found = (0..5).find_map(|i| {
                    let p = random_pos::land_pos(e, m, &*level, 10 + 2 * i, 3)?;
                    let b = BlockPos::containing(p.x, p.y, p.z).below();
                    diggable(m, &*level, b).then_some(b)
                });
                match found {
                    Some(b) => {
                        self.target = Some(b);
                        self.ticks = 600;
                        path::move_to(e, m, level, b.x as f64 + 0.5, b.y as f64 + 1.0, b.z as f64 + 0.5, 1.25);
                        transition(e, m, level, SEARCHING);
                    }
                    None => transition(e, m, level, IDLING),
                }
            }
            SEARCHING => {
                if self.ticks <= 0 {
                    transition(e, m, level, IDLING);
                } else if m.nav.is_done() {
                    let h = head_block(e).below();
                    if diggable(m, &*level, h) && can_sniff(e, m) {
                        self.ticks = mth::next_int_between(&mut e.random, 160, 180);
                        transition(e, m, level, DIGGING);
                    } else {
                        transition(e, m, level, IDLING);
                    }
                }
            }
            DIGGING if self.ticks <= 0 => {
                st_mut(m).sniff_cooldown_until = level.game_time() + 9600;
                self.ticks = 20;
                transition(e, m, level, RISING);
            }
            RISING if self.ticks <= 0 => {
                // `onDiggingComplete`: the spot is remembered (the last 20).
                let at = e.on_pos(&*level, 0.2);
                let s = st_mut(m);
                s.explored.insert(0, at);
                s.explored.truncate(20);
                self.ticks = mth::next_int_between(&mut e.random, 20, 30);
                transition(e, m, level, FEELING_HAPPY);
            }
            _ => {}
        }
    }
}
