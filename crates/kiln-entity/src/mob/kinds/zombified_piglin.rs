//! Zombified piglin: neutral until hurt, then angry with its group (`NeutralMob`).
//!
//! The anger is a persistent end time and a target: while it has a target the timer restarts
//! every tick (`updatePersistentAnger(level, true)`: a random 20 to 39 seconds, drawn each tick),
//! a newly angry piglin draws when to play its first angry sound and when to alert the others,
//! and every 4 to 6 seconds it sets the piglins around it without a target on its own.

use super::zombie::{self, HasZombie, ZombieState};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{Aabb, BlockPos};
use crate::mob::attributes::{Attr, Attr::*, Op};
use crate::mob::ext::{CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, Living, MeleeKind, Wanted};
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::EntityData;

pub struct ZombifiedPiglin;

pub static KIND: ZombifiedPiglin = ZombifiedPiglin;

static INFO: Info = Info {
    breathes_under_water: true,
    fire_immune: true,
    ..Info::monster("minecraft:zombified_piglin", &[(FollowRange, 35.0), (Armor, 2.0), (SpawnReinforcements, 0.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 5.0)])
};

#[derive(Clone, Debug, Default)]
pub struct PiglinState {
    pub zombie: ZombieState,
    pub play_first_anger_sound_in: i32,
    /// `persistentAngerEndTime` (game time; -1 or 0 for none).
    pub anger_end: i64,
    /// `persistentAngerTarget`: (entity id, UUID); the id is -1 until a loaded reference is found.
    pub anger_target: Option<(i32, u128)>,
    pub ticks_until_next_alert: i32,
}

impl HasZombie for PiglinState {
    fn zombie(&self) -> &ZombieState {
        &self.zombie
    }
    fn zombie_mut(&mut self) -> &mut ZombieState {
        &mut self.zombie
    }
}

fn st(m: &MobData) -> &PiglinState {
    crate::mob::ext::state::<PiglinState>(m).expect("zombified piglin state")
}

fn st_mut(m: &mut MobData) -> &mut PiglinState {
    crate::mob::ext::state_mut::<PiglinState>(m).expect("zombified piglin state")
}

/// `NeutralMob.isAngry`.
fn is_angry(m: &MobData, level: &dyn EntityLevel) -> bool {
    let end = st(m).anger_end;
    end > 0 && end - level.game_time() > 0
}

/// `startPersistentAngerTimer`: 20 to 39 seconds from now.
fn start_anger_timer(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let t = e.random.next_int_bounded(381) + 400;
    st_mut(m).anger_end = level.game_time() + t as i64;
}

/// `stopBeingAngry`.
fn stop_being_angry(e: &mut Entity, m: &mut MobData) {
    m.last_hurt_by_mob = None;
    m.last_hurt_by_mob_timestamp = e.tick_count;
    st_mut(m).anger_target = None;
    mob::set_target(e, m, None);
    st_mut(m).anger_end = -1;
}

fn valid_player_target(level: &dyn EntityLevel, t: &Living) -> bool {
    t.player && !t.creative && !t.spectator && level.difficulty() != 0
}

fn uuid_of(level: &dyn EntityLevel, id: i32) -> u128 {
    level.player(id).map(|p| p.uuid).or_else(|| level.entity(id).map(|e| e.uuid)).unwrap_or(0)
}

/// `updatePersistentAnger(level, true)`.
fn update_persistent_anger(e: &mut Entity, m: &mut MobData, level: &dyn EntityLevel) {
    let reference = st(m).anger_target.map(|(id, _)| id);
    if let Some(u) = m.target.and_then(|id| goals::living(level, id))
        && !u.alive
        && reference == Some(u.id)
        && !u.player
    {
        stop_being_angry(e, m);
        return;
    }
    let target = goals::target(m, level);
    if let Some(t) = target {
        let changed = reference != Some(t.id);
        if changed {
            st_mut(m).anger_target = Some((t.id, uuid_of(level, t.id)));
        }
        start_anger_timer(e, m, level);
    }
    if reference.is_some() && !is_angry(m, level) && target.is_none_or(|t| !valid_player_target(level, &t)) {
        stop_being_angry(e, m);
    }
    if let Some(r) = reference.and_then(|id| level.player(id))
        && (r.creative || r.spectator || level.difficulty() == 0)
    {
        stop_being_angry(e, m);
    }
}

/// `NeutralMob.isAngryAt` (universal anger is off: only the remembered target).
fn is_angry_at(m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
    if t.player && level.difficulty() == 0 {
        return false;
    }
    st(m).anger_target.is_some_and(|(id, uuid)| id == t.id || (id == -1 && uuid != 0 && uuid == uuid_of(level, t.id)))
}

/// `ResetUniversalAngerTargetGoal`: only with the `universal_anger` game rule, which is off.
#[derive(Clone, Debug)]
struct ResetUniversalAngerTargetGoal;

impl CustomGoal for ResetUniversalAngerTargetGoal {
    crate::custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "ResetUniversalAngerTargetGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
}

impl Kind for ZombifiedPiglin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(PiglinState::default()))
    }

    /// `Zombie.registerGoals` with `ZombifiedPiglin.addBehaviourGoals`.
    fn register_goals(&self, m: &mut MobData) {
        zombie::register_base_goals(m);
        m.goals.add(1, Goal::Never);
        m.goals.add(
            2,
            Goal::Melee { kind: MeleeKind::Zombie, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: crate::math::Vec3::ZERO, raise_arm: 0 },
        );
        m.goals.add(7, zombie::stroll(1.0, true));
        m.targets.add(1, zombie::hurt_by(true));
        m.targets.add(2, zombie::nearest(Wanted::Player, true));
        m.targets.add(3, Goal::Custom(Box::new(ResetUniversalAngerTargetGoal)));
        m.maluses.push((mob::path::PathType::Lava, 8.0));
    }

    fn on_set_target(&self, e: &mut Entity, m: &mut MobData, target: Option<i32>) {
        if m.target.is_none() && target.is_some() {
            let a = e.random.next_int_bounded(21);
            let b = e.random.next_int_bounded(41) + 80;
            let s = st_mut(m);
            s.play_first_anger_sound_in = a;
            s.ticks_until_next_alert = b;
        }
    }

    fn player_target_ok(&self, _e: &Entity, m: &MobData, level: &dyn EntityLevel, t: &Living) -> bool {
        is_angry_at(m, level, t)
    }

    /// `ZombifiedPiglin.customServerAiStep`.
    fn custom_server_ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // A loaded anger target: find it among the players.
        if let Some((-1, uuid)) = st(m).anger_target
            && let Some(p) = level.players().into_iter().find(|p| p.uuid == uuid)
        {
            st_mut(m).anger_target = Some((p.id, uuid));
            mob::set_target(e, m, Some(p.id));
        }
        let has = m.attrs.get(Attr::MovementSpeed).is_some_and(|i| i.has_modifier("minecraft:attacking"));
        if is_angry(m, level) {
            if !m.baby() && !has {
                m.attrs.set_modifier(Attr::MovementSpeed, "minecraft:attacking", 0.05, Op::AddValue);
            }
            let s = st_mut(m);
            if s.play_first_anger_sound_in > 0 {
                s.play_first_anger_sound_in -= 1;
                if s.play_first_anger_sound_in == 0 {
                    play_anger_sound(e, m, level);
                }
            }
        } else if has {
            m.attrs.remove_modifier(Attr::MovementSpeed, "minecraft:attacking");
        }
        update_persistent_anger(e, m, level);
        if m.target.is_some() {
            maybe_alert_others(e, m, level);
        }
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt {
            zombie::reinforcements(e, m, level, source);
        }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        zombie::finalize(e, m, r, ctx, group, false);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        zombie::load(e, m, r);
        let end = match r.num("anger_end_time") {
            Some(t) => t as i64,
            None => match r.num("AngerTime") {
                // Legacy relative ticks (the load happens at game time 0 as far as Kiln knows).
                Some(t) => t as i64,
                None => -1,
            },
        };
        let uuid = r.uuid("angry_at");
        let s = st_mut(m);
        s.anger_end = end;
        s.anger_target = uuid.map(|u| (-1, u));
    }

    fn save(&self, e: &Entity, m: &MobData, o: &mut Output) {
        zombie::save(e, m, o);
        let s = st(m);
        o.put("anger_end_time", Tag::Long(s.anger_end));
        if let Some((_, uuid)) = s.anger_target {
            o.put("angry_at", crate::persist::uuid_to_tag(uuid));
        }
    }

    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        zombie::entity_data(e, m, d);
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { zombie::baby_dimensions(m.kind) } else { base }
    }

    /// `checkZombifiedPiglinSpawnRules`: not peaceful, not on a nether wart block.
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, _r: &mut LegacyRandom) -> Option<bool> {
        Some(view.difficulty() != 0 && crate::blocks::block_name(view.block(pos.below())) != "minecraft:nether_wart_block")
    }
}

/// `playAngerSound`: twice as loud, pitched up.
fn play_anger_sound(e: &mut Entity, m: &MobData, level: &mut dyn EntityLevel) {
    let voice = if m.baby() {
        (e.random.next_float() - e.random.next_float()) * 0.2 + 1.5
    } else {
        (e.random.next_float() - e.random.next_float()) * 0.2 + 1.0
    };
    if !e.silent {
        level.emit(Event::Sound { pos: e.position(), sound: "minecraft:entity.zombified_piglin.angry", source: "hostile", volume: 2.0, pitch: voice * 1.8 });
    }
}

/// `maybeAlertOthers`.
fn maybe_alert_others(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    if st(m).ticks_until_next_alert > 0 {
        st_mut(m).ticks_until_next_alert -= 1;
        return;
    }
    if let Some(t) = m.target.and_then(|id| goals::living(level, id))
        && mob::has_line_of_sight_cached(e, m, level, &t)
    {
        let r = m.attrs.value(Attr::FollowRange);
        let p = e.position();
        let area = Aabb::new(p.x, p.y, p.z, p.x + 1.0, p.y + 1.0, p.z + 1.0).inflate(r, 10.0, r);
        for id in level.entities_in(&area, crate::level::EntityFilter::Living, e.id) {
            let idle = matches!(level.entity(id).map(|o| &o.kind), Some(crate::entity::EntityKind::Mob(om)) if om.kind == MobKind::ZombifiedPiglin && om.target.is_none());
            if idle {
                mob::set_target_of(level, id, Some(t.id));
            }
        }
    }
    let n = e.random.next_int_bounded(41) + 80;
    st_mut(m).ticks_until_next_alert = n;
}
