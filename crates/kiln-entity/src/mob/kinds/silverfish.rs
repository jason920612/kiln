//! Silverfish: small monsters that hide in stone (infested blocks). A hurt silverfish wakes the
//! silverfish hidden around it (their blocks break) and idle ones merge into stone next to them.
//! They come out of broken infested blocks and out of mobs with the infested effect.

use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::EntityLevel;
use crate::math::{BlockPos, Direction, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{CustomGoal, Info, Kind};
use crate::mob::goals::{self, Goal, MeleeKind, Wanted, MOVE};
use crate::mob::{self, DamageSource, MobData, mth};
use kiln_javamath::random::RandomSource;

pub struct Silverfish;

pub static KIND: Silverfish = Silverfish;

static INFO: Info = Info::monster("minecraft:silverfish", &[(MaxHealth, 8.0), (MovementSpeed, 0.25), (AttackDamage, 1.0)]);

/// The blocks silverfish hide in (`InfestedBlock`'s hosts) and their infested forms.
const HOSTS: &[(&str, &str)] = &[
    ("minecraft:stone", "minecraft:infested_stone"),
    ("minecraft:cobblestone", "minecraft:infested_cobblestone"),
    ("minecraft:stone_bricks", "minecraft:infested_stone_bricks"),
    ("minecraft:mossy_stone_bricks", "minecraft:infested_mossy_stone_bricks"),
    ("minecraft:cracked_stone_bricks", "minecraft:infested_cracked_stone_bricks"),
    ("minecraft:chiseled_stone_bricks", "minecraft:infested_chiseled_stone_bricks"),
    ("minecraft:deepslate", "minecraft:infested_deepslate"),
];

/// `InfestedBlock.isCompatibleHostBlock`.
pub fn is_host(state: u16) -> bool {
    let name = crate::blocks::block_name(state);
    HOSTS.iter().any(|(h, _)| *h == name)
}

/// Whether `state` is an infested block.
pub fn is_infested(state: u16) -> bool {
    let name = crate::blocks::block_name(state);
    HOSTS.iter().any(|(_, i)| *i == name)
}

/// Moves `state`'s properties (the deepslate axis) onto the block `to`.
fn with_properties(state: u16, to: &str) -> Option<u16> {
    use kiln_data::blocks_types::{block_by_name, block_of};
    let target = block_by_name(to)?;
    let from = block_of(state);
    let mut out = target.default;
    for (p, i) in from.properties.iter().zip(from.property_indices(state)) {
        if let Some(s) = target.with_property(out, p.name, p.values[i]) {
            out = s;
        }
    }
    Some(out)
}

/// `InfestedBlock.infestedStateByHost`.
pub fn infested_of(host: u16) -> Option<u16> {
    let name = crate::blocks::block_name(host);
    HOSTS.iter().find(|(h, _)| *h == name).and_then(|(_, i)| with_properties(host, i))
}

/// `InfestedBlock.hostStateByInfested`.
pub fn host_of(infested: u16) -> Option<u16> {
    let name = crate::blocks::block_name(infested);
    HOSTS.iter().find(|(_, i)| *i == name).and_then(|(h, _)| with_properties(infested, h))
}

impl Kind for Silverfish {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(1, Goal::Custom(Box::new(super::endermite::ClimbOnTopOfPowderSnow)));
        g.add(3, Goal::Custom(Box::new(WakeUpFriends { look_for_friends: 0 })));
        g.add(4, Goal::Melee { kind: MeleeKind::Plain, speed: 1.0, follow_unseen: false, path: None, recalc: 0, next_attack: 0, last_can_use: 0, pathed: Vec3::ZERO, raise_arm: 0 });
        g.add(
            5,
            Goal::Custom(Box::new(MergeWithStone {
                stroll: Goal::RandomStroll { speed: 1.0, interval: 10, check_no_action: true, water_avoiding: None, wanted: Vec3::ZERO, force: false },
                direction: Direction::Down,
                merge: false,
            })),
        );
        let t = &mut m.targets;
        t.add(1, Goal::HurtByTarget { timestamp: 0, alert_others: true, target_mob: None, unseen: 0, unseen_memory: 60 });
        t.add(2, Goal::NearestAttackable { wanted: Wanted::Player, interval: mth::reduced_tick_delay(10), must_see: true, target: None, unseen: 0, spider: false });
    }

    /// `Silverfish.tick`: the body turns with the yaw.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        m.y_body_rot = e.y_rot;
    }

    /// `setYBodyRot` also sets the yaw.
    fn tick_body(&self, e: &mut Entity, m: &mut MobData) -> bool {
        mob::control::tick_body(e, m);
        e.y_rot = m.y_body_rot;
        true
    }

    /// `hurtServer`: a hit from an entity (or a trigger damage type) wakes the friends.
    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32) -> Option<bool> {
        if source.attacker.is_some() || source.kind.is_tag("minecraft:always_triggers_silverfish") {
            for w in m.goals.goals.iter_mut() {
                if let Goal::Custom(c) = &mut w.goal
                    && let Some(f) = c.as_any_mut().downcast_mut::<WakeUpFriends>()
                    && f.look_for_friends == 0
                {
                    f.look_for_friends = mth::reduced_tick_delay(20);
                }
            }
        }
        None
    }

    fn walk_target_value(&self, _m: &MobData, level: &dyn EntityLevel, p: BlockPos) -> Option<f32> {
        if is_host(level.block(p.below())) {
            return Some(10.0);
        }
        Some(-(mob::light_magic_value_at(level, p) - 0.5))
    }

}

/// `SilverfishWakeUpFriendsGoal`: after a hurt, infested blocks around break (or turn back to
/// stone without mob griefing), stopping at random.
#[derive(Clone, Debug)]
struct WakeUpFriends {
    look_for_friends: i32,
}

impl CustomGoal for WakeUpFriends {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SilverfishWakeUpFriendsGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        self.look_for_friends > 0
    }
    fn tick(&mut self, e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        self.look_for_friends -= 1;
        if self.look_for_friends > 0 {
            return;
        }
        let base = e.block_position();
        let next = |o: i32| (if o <= 0 { 1 } else { 0 }) - o;
        let mut dy = 0;
        while (-5..=5).contains(&dy) {
            let mut dx = 0;
            while (-10..=10).contains(&dx) {
                let mut dz = 0;
                while (-10..=10).contains(&dz) {
                    let p = BlockPos::new(base.x + dx, base.y + dy, base.z + dz);
                    let s = level.block(p);
                    if is_infested(s) {
                        if level.mob_griefing() {
                            level.destroy_block(p, true);
                        } else if let Some(h) = host_of(s) {
                            level.set_block(p, h, 3);
                        }
                        if e.random.next_bool() {
                            return;
                        }
                    }
                    dz = next(dz);
                }
                dx = next(dx);
            }
            dy = next(dy);
        }
    }
}

/// `SilverfishMergeWithStoneGoal`: a random stroll (interval 10) that, now and then, instead
/// disappears into the stone block beside it.
#[derive(Clone, Debug)]
struct MergeWithStone {
    stroll: Goal,
    direction: Direction,
    merge: bool,
}

impl MergeWithStone {
    fn merge_pos(e: &Entity, d: Direction) -> BlockPos {
        let p = BlockPos::containing(e.x(), e.y() + 0.5, e.z());
        p.relative(d)
    }
}

impl CustomGoal for MergeWithStone {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "SilverfishMergeWithStoneGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if m.target.is_some() || !m.nav.is_done() {
            return false;
        }
        if level.mob_griefing() && e.random.next_int_bounded(mth::reduced_tick_delay(10)) == 0 {
            self.direction = Direction::ALL[e.random.next_int_bounded(6) as usize];
            if is_host(level.block(Self::merge_pos(e, self.direction))) {
                self.merge = true;
                return true;
            }
        }
        self.merge = false;
        goals::can_use(&mut self.stroll, e, m, level)
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.merge { false } else { goals::can_continue(&mut self.stroll, e, m, level) }
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !self.merge {
            goals::start(&mut self.stroll, e, m, level);
            return;
        }
        let p = Self::merge_pos(e, self.direction);
        let s = level.block(p);
        if let Some(inf) = infested_of(s) {
            level.set_block(p, inf, 3);
            // `spawnAnim`: entity event 20 (poof particles).
            level.emit(crate::level::Event::EntityEvent { entity: e.id, event: 20 });
            e.discard();
        }
    }
    /// `RandomStrollGoal.stop`: the navigation stops (merging or not).
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.stroll, e, m, level);
    }
}
