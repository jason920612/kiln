//! Ocelot: a wild cat of the jungle that avoids players until one feeds it raw fish while it is
//! being tempted (one try in three makes it trusting), hunts chickens (and baby turtles) and
//! sneaks while stalking or tempted, sprinting when it pounces.

use super::fish::AvoidPlayerGoal;
use crate::custom_goal_boilerplate;
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::{Attr::*, Op};
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt, SpawnView};
use crate::mob::goals::{self, Goal, LOOK, MOVE, Wanted};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::mth::reduced_tick_delay;
use crate::mob::{self, GroupData, MobData, SpawnContext, item_tag, path};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::{LegacyRandom, RandomSource};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Ocelot;

pub static KIND: Ocelot = Ocelot;

static INFO: Info = Info {
    ambient_interval: 900,
    ..Info::animal("minecraft:ocelot", &[(MaxHealth, 10.0), (MovementSpeed, 0.30000001192092896), (AttackDamage, 3.0)])
};

#[derive(Clone, Debug, Default)]
pub struct State {
    /// `DATA_TRUSTING`.
    pub trusting: bool,
    crouching: bool,
    sprinting: bool,
    /// `tickCount` (an ocelot is not removed far away in its first two minutes).
    ticks: i32,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("ocelot state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("ocelot state")
}

/// `isTrusting`.
pub fn trusting(m: &MobData) -> bool {
    ext::state::<State>(m).is_some_and(|s| s.trusting)
}

fn is_food(item: i32) -> bool {
    item_tag(item, "minecraft:ocelot_food")
}

impl Kind for Ocelot {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    /// `registerGoals`, then `reassessTrustingGoals` (the avoid goal checks the trust itself).
    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(1, Goal::Float);
        g.add(3, Goal::Custom(Box::new(OcelotTemptGoal { speed: 0.6, player: None, calm_down: 0, px: 0.0, py: 0.0, pz: 0.0, rot_x: 0.0, rot_y: 0.0 })));
        g.add(7, Goal::LeapAtTarget { yd: 0.3, target: None });
        g.add(8, Goal::Custom(Box::new(super::cat::OcelotAttackGoal { target: None, attack_time: 0 })));
        g.add(9, Goal::Breed { speed: 0.8, partner: None, love_time: 0 });
        g.add(10, Goal::RandomStroll { speed: 0.8, interval: 120, check_no_action: true, water_avoiding: Some(1.0000001e-5), wanted: Vec3::ZERO, force: false });
        g.add(11, Goal::LookAtPlayer { dist: 10.0, probability: 0.02, look_at: None, look_time: 0 });
        let t = &mut m.targets;
        t.add(1, Goal::NearestAttackable { wanted: Wanted::Types(&["minecraft:chicken"]), interval: reduced_tick_delay(10), must_see: false, target: None, unseen: 0, spider: false });
        // Baby turtles on land.
        t.add(1, Goal::NearestAttackable { wanted: Wanted::Unsimulated, interval: reduced_tick_delay(10), must_see: false, target: None, unseen: 0, spider: false });
        m.goals.add(4, Goal::Custom(Box::new(AvoidPlayerGoal::new("OcelotAvoidEntityGoal", 16.0, 0.8, 1.33, Some(trusting)))));
    }

    /// `customServerAiStep`: crouching at the sneaking speed, sprinting at the sprinting one.
    fn custom_server_ai_step(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        let (crouch, sprint) = if m.mov.has_wanted() {
            let s = m.mov.speed_modifier;
            (s == 0.6, s == 1.33)
        } else {
            (false, false)
        };
        let s = st_mut(m);
        s.crouching = crouch;
        if s.sprinting != sprint {
            s.sprinting = sprint;
            // `LivingEntity.setSprinting`: the sprint speed bonus.
            if sprint {
                m.attrs.set_modifier(MovementSpeed, "minecraft:sprinting", 0.30000001192092896, Op::AddMultipliedTotal);
            } else {
                m.attrs.remove_modifier(MovementSpeed, "minecraft:sprinting");
            }
        }
    }

    /// `Entity.baseTick`: a sprinting ocelot kicks up block particles (two draws).
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        st_mut(m).ticks = e.tick_count;
        let s = st(m);
        if s.sprinting && !s.crouching && !e.is_in_water() && !e.is_in_lava() && mob::is_alive(e, m) {
            let below = level.block(e.on_pos_legacy(level));
            let name = crate::blocks::block_name(below);
            let invisible = kiln_data::blocks_types::is_air(below) || matches!(name, "minecraft:barrier" | "minecraft:light" | "minecraft:structure_void" | "minecraft:moving_piston");
            if !invisible {
                e.random.next_double();
                e.random.next_double();
            }
        }
    }

    fn is_food(&self, item: i32) -> bool {
        is_food(item)
    }

    fn tempted_by(&self, item: i32) -> bool {
        is_food(item)
    }

    /// `removeWhenFarAway`: an untrusting ocelot, after two minutes.
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        Some(!st(m).trusting && st(m).ticks > 2400)
    }

    /// `mobInteract`: fish from a player it is tempted by, within 3 blocks, wins its trust one
    /// time in three.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        let tempted = m.goals.is_running(|g| matches!(g, Goal::Custom(c) if c.name() == "OcelotTemptGoal"));
        let near = level.player(who.id).is_some_and(|p| p.pos.distance_to_sqr(e.position()) < 9.0);
        if !(tempted && !st(m).trusting && !stack.is_empty() && is_food(stack.item()) && near) {
            return None;
        }
        let trust = e.random.next_int_bounded(3) == 0;
        if trust {
            st_mut(m).trusting = true;
        }
        // `spawnTrustingParticles` (only their draws matter here).
        for _ in 0..7 {
            e.random.next_gaussian();
            e.random.next_gaussian();
            e.random.next_gaussian();
            mob::random_point(e, 1.0);
        }
        level.emit(Event::EntityEvent { entity: e.id, event: if trust { 41 } else { 40 } });
        Some(Outcome::success(HeldChange::Consume(1)))
    }

    /// `checkOcelotSpawnRules`: two tries in three (the obstruction check wants grass or leaves
    /// under it, at sea level or above).
    fn check_spawn_rules(&self, view: &dyn SpawnView, pos: BlockPos, r: &mut LegacyRandom) -> Option<bool> {
        let below = view.block(pos.below());
        let ground = pos.y >= view.sea_level()
            && (crate::blocks::block_name(below) == "minecraft:grass_block" || super::wolf::block_in_tag(below, "minecraft:leaves"));
        Some(r.next_int_bounded(3) != 0 && ground)
    }

    /// `finalizeSpawn` with `AgeableMobGroupData(1.0)`: every ocelot after the first is a baby.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        ext::ageable_finalize(e, m, r, group, 1.0);
        ext::mob_finalize(m, r);
    }

    /// `Ocelot.BABY_DIMENSIONS`.
    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (0.3, 0.35, 0.34375) } else { base }
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let t = r.bool_or("Trusting", false);
        st_mut(m).trusting = t;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        o.put("Trusting", Tag::Byte(st(m).trusting as i8));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        if s.crouching {
            d.set(data::entity::POSE, &DataValue::Pose(kiln_data::entities::pose::CROUCHING));
        }
        if s.sprinting {
            d.set(data::entity::SHARED_FLAGS, &DataValue::Byte(0x08));
        }
        d.set(data::ocelot::TRUSTING, &DataValue::Boolean(s.trusting));
    }
}

/// `Ocelot.OcelotTemptGoal`: a `TemptGoal` (speed 0.6, ocelot food) that a sudden move scares
/// off while the ocelot does not trust players.
#[derive(Clone, Debug)]
struct OcelotTemptGoal {
    speed: f64,
    player: Option<i32>,
    calm_down: i32,
    px: f64,
    py: f64,
    pz: f64,
    rot_x: f64,
    rot_y: f64,
}

impl CustomGoal for OcelotTemptGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "OcelotTemptGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.calm_down > 0 {
            self.calm_down -= 1;
            return false;
        }
        let range = m.attrs.value(TemptRange);
        self.player = goals::nearest_player(e, m, level, false, range, false, |p| is_food(p.main_hand) || is_food(p.off_hand)).map(|p| p.id);
        self.player.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !st(m).trusting
            && let Some(p) = self.player.and_then(|id| level.player(id))
        {
            if e.position().distance_to_sqr(p.pos) < 36.0 {
                if p.pos.distance_to_sqr(Vec3::new(self.px, self.py, self.pz)) > 0.010000000000000002 {
                    return false;
                }
                if (p.pitch as f64 - self.rot_x).abs() > 5.0 || (p.yaw as f64 - self.rot_y).abs() > 5.0 {
                    return false;
                }
            } else {
                (self.px, self.py, self.pz) = (p.pos.x, p.pos.y, p.pos.z);
            }
            self.rot_x = p.pitch as f64;
            self.rot_y = p.yaw as f64;
        }
        self.can_use(e, m, level)
    }
    fn start(&mut self, _e: &mut Entity, _m: &mut MobData, level: &mut dyn EntityLevel) {
        if let Some(p) = self.player.and_then(|id| level.player(id)) {
            (self.px, self.py, self.pz) = (p.pos.x, p.pos.y, p.pos.z);
        }
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.player = None;
        m.nav.stop();
        self.calm_down = reduced_tick_delay(100);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(p) = self.player.and_then(|id| goals::living(level, id)) else { return };
        let (hs, hx) = ((m.kind.max_head_y_rot() + 20) as f32, m.max_head_x_rot() as f32);
        m.look.set_look_at(p.pos.x, p.eye_y, p.pos.z, hs, hx);
        if e.position().distance_to_sqr(p.pos) < 2.5 * 2.5 {
            m.nav.stop();
        } else {
            path::move_to_entity(e, m, level, BlockPos::containing(p.pos.x, p.pos.y, p.pos.z), self.speed);
        }
    }
}
