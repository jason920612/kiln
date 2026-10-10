//! Panda: two genes (main and hidden; brown and weak are recessive) give its personality —
//! lazy pandas lie on their backs and move slowly, worried ones flee players and monsters,
//! playful ones and cubs roll about, weak cubs sneeze more (a slimeball with
//! `gameplay/panda_sneeze`, startling the adults into a jump), aggressive ones join a fight.
//! Pandas sit down to eat bamboo and cake they pick up or are handed, only breed near bamboo
//! (sulking otherwise), and bite back once when hurt.

use super::common_a::{self, Avoid, AvoidEntityGoal, MeleeGoal, Named};
use crate::custom_goal_boilerplate;
use crate::entity::{Entity, EntityKind};
use crate::level::{EntityFilter, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, CustomGoal, Info, Kind, MobExt};
use crate::mob::goals::{self, Goal, JUMP, LOOK, Living, MOVE};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::{DamageSource, GroupData, MAINHAND, MobData, MobKind, SpawnContext, item_tag, mth};
use crate::persist::{Input, Output};
use kiln_data::entities::data;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Panda;

pub static KIND: Panda = Panda;

static INFO: Info = Info::animal("minecraft:panda", &[(MovementSpeed, 0.15000000596046448), (AttackDamage, 6.0)]);

/// `Panda.Gene` ids.
pub const NORMAL: u8 = 0;
pub const LAZY: u8 = 1;
pub const WORRIED: u8 = 2;
pub const PLAYFUL: u8 = 3;
pub const BROWN: u8 = 4;
pub const WEAK: u8 = 5;
pub const AGGRESSIVE: u8 = 6;
const GENES: [&str; 7] = ["normal", "lazy", "worried", "playful", "brown", "weak", "aggressive"];

const SNEEZE: u8 = 2;
const ROLL: u8 = 4;
const SIT: u8 = 8;
const ON_BACK: u8 = 16;

#[derive(Clone, Debug)]
pub struct State {
    pub main_gene: u8,
    pub hidden_gene: u8,
    pub flags: u8,
    unhappy: i32,
    sneeze_counter: i32,
    eat_counter: i32,
    got_bamboo: bool,
    did_bite: bool,
    roll_counter: i32,
    roll_delta: Vec3,
    /// `PandaLookAtPlayerGoal.lookAt` (the breed goal points it at a player).
    look_at: Option<i32>,
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("panda state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("panda state")
}

fn flag(m: &MobData, f: u8) -> bool {
    st(m).flags & f != 0
}

fn set_flag(m: &mut MobData, f: u8, on: bool) {
    let s = st_mut(m);
    if on { s.flags |= f } else { s.flags &= !f }
}

/// `Gene.getRandom`.
fn random_gene(r: &mut dyn RandomSource) -> u8 {
    match r.next_int_bounded(16) {
        0 => LAZY,
        1 => WORRIED,
        2 => PLAYFUL,
        4 => AGGRESSIVE,
        n if n < 9 => WEAK,
        n if n < 11 => BROWN,
        _ => NORMAL,
    }
}

fn recessive(g: u8) -> bool {
    g == BROWN || g == WEAK
}

/// `getVariant`: a recessive main gene shows only when both genes carry it.
pub fn variant(m: &MobData) -> u8 {
    let s = st(m);
    if recessive(s.main_gene) {
        if s.main_gene == s.hidden_gene { s.main_gene } else { NORMAL }
    } else {
        s.main_gene
    }
}

fn is_eating(m: &MobData) -> bool {
    st(m).eat_counter > 0
}

fn eat(m: &mut MobData, on: bool) {
    st_mut(m).eat_counter = on as i32;
}

fn sit(m: &mut MobData, on: bool) {
    set_flag(m, SIT, on);
}

/// `isScared` (worried in a thunderstorm: storms are not visible here).
fn is_scared(_m: &MobData) -> bool {
    false
}

/// `canPerformAction`.
fn can_perform_action(m: &MobData) -> bool {
    !flag(m, ON_BACK) && !is_scared(m) && !is_eating(m) && !flag(m, ROLL) && !flag(m, SIT)
}

/// `setAttributes`: weak pandas have 10 health, lazy ones crawl.
fn set_attributes(m: &mut MobData) {
    let v = variant(m);
    if v == WEAK
        && let Some(a) = m.attrs.get_mut(MaxHealth)
    {
        a.base = 10.0;
    }
    if v == LAZY
        && let Some(a) = m.attrs.get_mut(MovementSpeed)
    {
        a.base = 0.07000000029802322;
    }
}

fn eats_from_ground(stack: &ItemStack) -> bool {
    !stack.is_empty() && item_tag(stack.item(), "minecraft:panda_eats_from_ground")
}

/// `tryToSit`.
fn try_to_sit(e: &Entity, m: &mut MobData) {
    if !e.is_in_water() {
        m.zza = 0.0;
        m.nav.stop();
        sit(m, true);
    }
}

/// Items a panda would pick up and eat within `r` blocks, first found first.
fn edible_items(e: &Entity, level: &dyn EntityLevel, r: f64) -> Vec<(i32, BlockPos)> {
    let area = e.bounding_box().inflate(r, r, r);
    level
        .entities_in(&area, EntityFilter::Item, e.id)
        .into_iter()
        .filter_map(|id| {
            let o = level.entity(id)?;
            let EntityKind::Item(d) = &o.kind else { return None };
            (eats_from_ground(&d.stack) && o.is_alive() && d.pickup_delay == 0).then(|| (id, o.block_position()))
        })
        .collect()
}

impl Kind for Panda {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        m.can_pick_up_loot = true;
        Some(Box::new(State {
            main_gene: NORMAL,
            hidden_gene: NORMAL,
            flags: 0,
            unhappy: 0,
            sneeze_counter: 0,
            eat_counter: 0,
            got_bamboo: false,
            did_bite: false,
            roll_counter: 0,
            roll_delta: Vec3::ZERO,
            look_at: None,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        let g = &mut m.goals;
        g.add(0, Goal::Float);
        g.add(
            2,
            Named::new("PandaPanicGoal", Goal::Custom(Box::new(common_a::PanicGoal::new("PanicGoal", 2.0, |_| "minecraft:panic_environmental_causes"))))
                .keep(|_, m, _| !flag(m, SIT))
                .boxed(),
        );
        g.add(2, Goal::Custom(Box::new(PandaBreedGoal { inner: common_a::breed(1.0), unhappy_cooldown: 0 })));
        let mut attack = MeleeGoal::new("PandaAttackGoal", 1.2000000476837158, true, common_a::plain_attack);
        attack.gate = Some(can_perform_action);
        g.add(3, Goal::Custom(Box::new(attack)));
        g.add(4, common_a::tempt(1.0));
        let worried = |m: &MobData, _: &dyn EntityLevel| variant(m) == WORRIED && can_perform_action(m);
        g.add(6, Goal::Custom(Box::new(AvoidEntityGoal::new("PandaAvoidGoal", Avoid::Players, 8.0, 2.0, 2.0).gate(worried))));
        g.add(6, Goal::Custom(Box::new(AvoidEntityGoal::new("PandaAvoidGoal", Avoid::Monsters, 4.0, 2.0, 2.0).gate(worried))));
        g.add(7, Goal::Custom(Box::new(PandaSitGoal { cooldown: 0 })));
        g.add(8, Goal::Custom(Box::new(PandaLieOnBackGoal { cooldown: 0 })));
        g.add(8, Goal::Custom(Box::new(PandaSneezeGoal)));
        g.add(9, Goal::Custom(Box::new(PandaLookAtPlayerGoal { look_time: 0 })));
        g.add(10, common_a::look_around());
        g.add(12, Goal::Custom(Box::new(PandaRollGoal)));
        g.add(13, common_a::follow_parent(1.25));
        g.add(14, common_a::stroll(1.0));
        m.targets.add(
            1,
            Named::new("PandaHurtByTargetGoal", common_a::hurt_by(false))
                .after_start(|_, e, m, level| common_a::alert_others(e, m, level, |o| variant(o) == AGGRESSIVE))
                .keep(|_, m, _| !st(m).got_bamboo && !st(m).did_bite)
                .boxed(),
        );
    }

    fn post_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if variant(m) == WORRIED && !is_eating(m) {
            sit(m, false);
        }
        let target = goals::living(level, m.target.unwrap_or(i32::MIN));
        if m.target.is_none() {
            let s = st_mut(m);
            s.got_bamboo = false;
            s.did_bite = false;
        }
        if st(m).unhappy > 0 {
            if let Some(t) = &target {
                crate::mob::mob_look_at(e, t, 90.0, 90.0);
            }
            let u = st(m).unhappy;
            if u == 29 || u == 14 {
                common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.panda.cant_breed"), 1.0, 1.0);
            }
            st_mut(m).unhappy -= 1;
        }
        if flag(m, SNEEZE) {
            st_mut(m).sneeze_counter += 1;
            let c = st(m).sneeze_counter;
            if c > 20 {
                set_flag(m, SNEEZE, false);
                st_mut(m).sneeze_counter = 0;
                after_sneeze(e, m, level);
            } else if c == 1 {
                common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.panda.pre_sneeze"), 1.0, 1.0);
            }
        }
        if flag(m, ROLL) {
            handle_roll(e, m);
        } else {
            st_mut(m).roll_counter = 0;
        }
        if flag(m, SIT) {
            e.x_rot = 0.0;
        }
        handle_eating(e, m, level);
    }

    fn ai_step(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        // `Mob.aiStep`'s pickup with `Panda.pickUpItem`: a bamboo (or cake) stack into the paw.
        if !m.can_pick_up_loot || !crate::mob::is_alive(e, m) || !level.mob_griefing() {
            return;
        }
        let area = e.bounding_box().inflate(1.0, 0.0, 1.0);
        for id in level.entities_in(&area, EntityFilter::Item, e.id) {
            if !m.equipment[MAINHAND].is_empty() {
                break;
            }
            let Some(item) = level.entity_mut(id) else { continue };
            let removed = item.is_removed();
            let EntityKind::Item(d) = &mut item.kind else { continue };
            if removed || d.stack.is_empty() || d.pickup_delay > 0 || !eats_from_ground(&d.stack) {
                continue;
            }
            let thrower = d.thrower;
            m.equipment[MAINHAND] = std::mem::replace(&mut d.stack, ItemStack::empty());
            m.drop_chances[MAINHAND] = 2.0;
            item.discard();
            let taken = m.equipment[MAINHAND].clone();
            crate::mob::on_item_pickup(e, m, level, thrower, &taken);
        }
    }

    fn tick_move(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if can_perform_action(m) {
            crate::mob::control::tick_move(e, m, level);
        }
        true
    }

    fn hurt(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _source: &DamageSource, _amount: f32) -> Option<bool> {
        sit(m, false);
        None
    }

    fn do_hurt_target(&self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel, _t: &Living) -> Option<bool> {
        if variant(m) != AGGRESSIVE {
            st_mut(m).did_bite = true;
        }
        None
    }

    fn after_hurt_target(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, _t: &Living) {
        common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.panda.bite"), 1.0, 1.0);
    }

    fn ambient_sound(&self, _e: &mut Entity, m: &MobData, _level: &dyn EntityLevel) -> Option<Option<&'static str>> {
        let s = match variant(m) {
            AGGRESSIVE => "minecraft:entity.panda.aggressive_ambient",
            WORRIED => "minecraft:entity.panda.worried_ambient",
            _ => "minecraft:entity.panda.ambient",
        };
        Some(Some(crate::mob::sound_event(s)))
    }

    fn is_food(&self, item: i32) -> bool {
        item_tag(item, "minecraft:panda_food")
    }

    fn tempted_by(&self, item: i32) -> bool {
        self.is_food(item)
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { (base.0 * 0.5, base.1 * 0.5, 0.28125) } else { base }
    }

    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, _ctx: &SpawnContext, group: &mut GroupData) {
        let main = random_gene(r);
        let hidden = random_gene(r);
        let s = st_mut(m);
        s.main_gene = main;
        s.hidden_gene = hidden;
        set_attributes(m);
        ext::ageable_finalize(e, m, r, group, 0.2);
        ext::mob_finalize(m, r);
    }

    fn breed_offspring(&self, e: &mut Entity, m: &mut MobData, partner: &MobData, child: &mut MobData, _level: &mut dyn EntityLevel) {
        // `setGeneFromParents` draws from the baby's own random (a fresh one: approximated by
        // the parent's).
        let r = &mut e.random;
        let pick = |r: &mut kiln_javamath::random::LegacyRandom, s: &State| if r.next_bool() { s.main_gene } else { s.hidden_gene };
        let a = st(m).clone();
        let b = ext::state::<State>(partner).cloned().unwrap_or_else(|| a.clone());
        let (main, hidden) = if r.next_bool() { (pick(r, &a), pick(r, &b)) } else { (pick(r, &b), pick(r, &a)) };
        let mut main = main;
        let mut hidden = hidden;
        if r.next_int_bounded(32) == 0 {
            main = random_gene(r);
        }
        if r.next_int_bounded(32) == 0 {
            hidden = random_gene(r);
        }
        let s = st_mut(child);
        s.main_gene = main;
        s.hidden_gene = hidden;
        set_attributes(child);
        child.health = child.max_health();
    }

    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if is_scared(m) {
            return Some(Outcome::PASS);
        }
        if flag(m, ON_BACK) {
            set_flag(m, ON_BACK, false);
            return Some(Outcome::success(HeldChange::None));
        }
        if stack.is_empty() || !self.is_food(stack.item()) {
            // Babies still take the golden dandelion (`Animal.mobInteract`); otherwise nothing.
            return (!m.baby() || crate::mob::item_name(stack) != "minecraft:golden_dandelion").then_some(Outcome::PASS);
        }
        if m.target.is_some() {
            st_mut(m).got_bamboo = true;
        }
        if m.baby() && !m.age_locked {
            let age = m.age;
            crate::mob::age_up(e, m, ((-age / 20) as f32 * 0.1) as i32, true);
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if m.baby() {
            return Some(Outcome::PASS);
        }
        if m.age == 0 && m.in_love <= 0 {
            crate::mob::breed::set_in_love(e, m, level, Some(who.id));
            return Some(Outcome::success(HeldChange::Consume(1)));
        }
        if flag(m, SIT) || e.is_in_water() {
            return Some(Outcome::PASS);
        }
        try_to_sit(e, m);
        eat(m, true);
        let current = std::mem::replace(&mut m.equipment[MAINHAND], ItemStack::empty());
        if !current.is_empty() && !who.creative {
            crate::mob::spawn_at_location(e, level, current);
        }
        m.equipment[MAINHAND] = ItemStack::new(stack.item(), 1);
        Some(Outcome::success(HeldChange::Consume(1)))
    }

    fn load(&self, _e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let gene = |t: Option<&Tag>| t.and_then(Tag::as_str).and_then(|n| GENES.iter().position(|g| *g == n)).map_or(NORMAL, |i| i as u8);
        let main = gene(r.get("MainGene"));
        let hidden = gene(r.get("HiddenGene"));
        let s = st_mut(m);
        s.main_gene = main;
        s.hidden_gene = hidden;
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("MainGene", Tag::String(GENES[s.main_gene as usize].into()));
        o.put("HiddenGene", Tag::String(GENES[s.hidden_gene as usize].into()));
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        d.set(data::panda::UNHAPPY_COUNTER, &DataValue::Int(s.unhappy));
        d.set(data::panda::SNEEZE_COUNTER, &DataValue::Int(s.sneeze_counter));
        d.set(data::panda::EAT_COUNTER, &DataValue::Int(s.eat_counter));
        d.set(data::panda::MAIN_GENE, &DataValue::Byte(s.main_gene as i8));
        d.set(data::panda::HIDDEN_GENE, &DataValue::Byte(s.hidden_gene as i8));
        d.set(data::panda::ID_FLAGS, &DataValue::Byte(s.flags as i8));
    }
}

/// `handleEating`: sitting with food, starts eating now and then; eats a bamboo up after
/// about five seconds.
fn handle_eating(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let held = !m.equipment[MAINHAND].is_empty();
    if !is_eating(m) && flag(m, SIT) && !is_scared(m) && held && e.random.next_int_bounded(80) == 1 {
        eat(m, true);
    } else if !held || !flag(m, SIT) {
        eat(m, false);
    }
    if !is_eating(m) {
        return;
    }
    // `addEatingParticles`.
    if st(m).eat_counter % 5 == 0 {
        let volume = 0.5 + 0.5 * e.random.next_int_bounded(2) as f32;
        let pitch = common_a::voice(e);
        common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.panda.eat"), volume, pitch);
        if !m.equipment[MAINHAND].is_empty() {
            for _ in 0..36 {
                e.random.next_float();
            }
        }
    }
    if st(m).eat_counter > 80 && e.random.next_int_bounded(20) == 1 {
        if st(m).eat_counter > 100 && eats_from_ground(&m.equipment[MAINHAND]) {
            m.equipment[MAINHAND] = ItemStack::empty();
            level.emit(Event::GameEvent { event: "minecraft:eat", pos: e.position(), entity: Some(e.id) });
            sit(m, false);
        }
        eat(m, false);
        return;
    }
    st_mut(m).eat_counter += 1;
}

/// `handleRoll`: a 32-tick tumble forward with three hops.
fn handle_roll(e: &mut Entity, m: &mut MobData) {
    st_mut(m).roll_counter += 1;
    let c = st(m).roll_counter;
    if c > 32 {
        set_flag(m, ROLL, false);
        return;
    }
    let v = e.delta;
    if c == 1 {
        let angle = e.y_rot * 0.017453292;
        let mult = if m.baby() { 0.1f32 } else { 0.2 };
        let d = Vec3::new(v.x + (-mth::sin(angle as f64) * mult) as f64, 0.0, v.z + (mth::cos(angle as f64) * mult) as f64);
        st_mut(m).roll_delta = d;
        e.delta = d.add(0.0, 0.27, 0.0);
    } else if c != 7 && c != 15 && c != 23 {
        let d = st(m).roll_delta;
        e.delta = Vec3::new(d.x, v.y, d.z);
    } else {
        e.delta = Vec3::new(0.0, if e.on_ground { 0.27 } else { v.y }, 0.0);
    }
}

/// `afterSneeze`: the sound, adults nearby jump, the sneeze gift (a slimeball).
fn after_sneeze(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    common_a::play(e, m, level, crate::mob::sound_event("minecraft:entity.panda.sneeze"), 1.0, 1.0);
    let area = e.bounding_box().inflate(10.0, 10.0, 10.0);
    for id in level.entities_in(&area, EntityFilter::Living, e.id) {
        let Some(o) = level.entity_mut(id) else { continue };
        let ok = matches!(&o.kind, EntityKind::Mob(om) if om.kind == MobKind::Panda && !om.baby() && can_perform_action(om));
        if ok && o.on_ground && !o.is_in_water() {
            // `jumpFromGround` (jump strength 0.42, no block factor lookup from here).
            let power = crate::mob::data(o).map_or(0.41999998688697815, |om| om.attrs.value(JumpStrength)) as f32;
            o.delta = Vec3::new(o.delta.x, (power as f64).max(o.delta.y), o.delta.z);
            o.needs_sync = true;
        }
    }
    if level.mob_drops() {
        level.emit(Event::GiftLoot { entity: e.id, table: "minecraft:gameplay/panda_sneeze", pos: e.position() });
    }
}

/// `PandaBreedGoal`: in love, but only near bamboo; otherwise sulks at the nearest player.
#[derive(Clone, Debug)]
struct PandaBreedGoal {
    inner: Goal,
    unhappy_cooldown: i32,
}

fn can_find_bamboo(e: &Entity, level: &dyn EntityLevel) -> bool {
    let o = e.block_position();
    for y in 0..3 {
        for r in 0..8 {
            let mut x = 0;
            while x <= r {
                let mut z = if x < r && x > -r { r } else { 0 };
                while z <= r {
                    if crate::blocks::block_name(level.block(o.offset(x, y, z))) == "minecraft:bamboo" {
                        return true;
                    }
                    z = if z > 0 { -z } else { 1 - z };
                }
                x = if x > 0 { -x } else { 1 - x };
            }
        }
    }
    false
}

impl CustomGoal for PandaBreedGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaBreedGoal"
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if !goals::can_use(&mut self.inner, e, m, level) || st(m).unhappy != 0 {
            return false;
        }
        if can_find_bamboo(e, level) {
            return true;
        }
        if self.unhappy_cooldown <= e.tick_count {
            st_mut(m).unhappy = 32;
            self.unhappy_cooldown = e.tick_count + 600;
            if !m.no_ai {
                let p = goals::nearest_player(e, m, level, false, 8.0, true, |_| true).map(|p| p.id);
                st_mut(m).look_at = p;
            }
        }
        false
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        goals::can_continue(&mut self.inner, e, m, level)
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::start(&mut self.inner, e, m, level);
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::stop(&mut self.inner, e, m, level);
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        goals::tick_goal(&mut self.inner, e, m, level);
    }
}

/// `PandaSitGoal`: walks to bamboo on the ground, sits down to eat what it holds.
#[derive(Clone, Debug)]
struct PandaSitGoal {
    cooldown: i32,
}

impl CustomGoal for PandaSitGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaSitGoal"
    }
    fn flags(&self) -> u8 {
        MOVE
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if self.cooldown > e.tick_count || m.baby() || e.is_in_water() || !can_perform_action(m) || st(m).unhappy > 0 {
            return false;
        }
        !m.equipment[MAINHAND].is_empty() || !edible_items(e, level, 6.0).is_empty()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !e.is_in_water() && (variant(m) == LAZY || e.random.next_int_bounded(mth::reduced_tick_delay(600)) != 1) {
            e.random.next_int_bounded(mth::reduced_tick_delay(2000)) != 1
        } else {
            false
        }
    }
    fn tick(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        if !flag(m, SIT) && !m.equipment[MAINHAND].is_empty() {
            try_to_sit(e, m);
        }
    }
    fn start(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if m.equipment[MAINHAND].is_empty() {
            if let Some(&(_, p)) = edible_items(e, level, 8.0).first() {
                crate::mob::path::move_to_entity(e, m, level, p, 1.2000000476837158);
            }
        } else {
            try_to_sit(e, m);
        }
        self.cooldown = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let held = std::mem::replace(&mut m.equipment[MAINHAND], ItemStack::empty());
        if !held.is_empty() {
            crate::mob::spawn_at_location(e, level, held);
            let wait = if variant(m) == LAZY { e.random.next_int_bounded(50) + 10 } else { e.random.next_int_bounded(150) + 10 };
            self.cooldown = e.tick_count + wait * 20;
        }
        sit(m, false);
    }
}

/// `PandaLieOnBackGoal`: lazy pandas roll onto their backs for a while.
#[derive(Clone, Debug)]
struct PandaLieOnBackGoal {
    cooldown: i32,
}

impl CustomGoal for PandaLieOnBackGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaLieOnBackGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        self.cooldown < e.tick_count && variant(m) == LAZY && can_perform_action(m) && e.random.next_int_bounded(mth::reduced_tick_delay(400)) == 1
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !e.is_in_water() && (variant(m) == LAZY || e.random.next_int_bounded(mth::reduced_tick_delay(600)) != 1) {
            e.random.next_int_bounded(mth::reduced_tick_delay(2000)) != 1
        } else {
            false
        }
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, ON_BACK, true);
        self.cooldown = 0;
    }
    fn stop(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, ON_BACK, false);
        self.cooldown = e.tick_count + 200;
    }
}

/// `PandaSneezeGoal`: cubs sneeze now and then (weak ones more often).
#[derive(Clone, Debug)]
struct PandaSneezeGoal;

impl CustomGoal for PandaSneezeGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaSneezeGoal"
    }
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        if !(m.baby() && can_perform_action(m)) {
            return false;
        }
        if variant(m) == WEAK && e.random.next_int_bounded(mth::reduced_tick_delay(500)) == 1 {
            return true;
        }
        e.random.next_int_bounded(mth::reduced_tick_delay(6000)) == 1
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, SNEEZE, true);
    }
}

/// `PandaLookAtPlayerGoal`: a `LookAtPlayerGoal` (6 blocks) that keeps its player and can be
/// pointed at one.
#[derive(Clone, Debug)]
struct PandaLookAtPlayerGoal {
    look_time: i32,
}

impl CustomGoal for PandaLookAtPlayerGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaLookAtPlayerGoal"
    }
    fn flags(&self) -> u8 {
        LOOK
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        if e.random.next_float() >= 0.02 {
            return false;
        }
        if st(m).look_at.is_none() {
            let p = goals::nearest_player(e, m, level, false, 6.0, true, |_| true).map(|p| p.id);
            st_mut(m).look_at = p;
        }
        can_perform_action(m) && st(m).look_at.is_some()
    }
    fn can_continue(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let Some(t) = st(m).look_at.and_then(|id| goals::living(level, id)) else { return false };
        t.alive && e.position().distance_to_sqr(t.pos) <= 36.0 && self.look_time > 0
    }
    fn start(&mut self, e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) {
        self.look_time = mth::reduced_tick_delay(40 + e.random.next_int_bounded(40));
    }
    fn stop(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        st_mut(m).look_at = None;
    }
    fn tick(&mut self, _e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        let Some(t) = st(m).look_at.and_then(|id| goals::living(level, id)) else { return };
        if t.alive {
            crate::mob::control::look_at(m, t.pos.x, t.eye_y, t.pos.z);
            self.look_time -= 1;
        }
    }
}

/// `PandaRollGoal`: cubs and playful pandas tumble (always at a drop ahead).
#[derive(Clone, Debug)]
struct PandaRollGoal;

impl CustomGoal for PandaRollGoal {
    custom_goal_boilerplate!();
    fn name(&self) -> &'static str {
        "PandaRollGoal"
    }
    fn flags(&self) -> u8 {
        MOVE | LOOK | JUMP
    }
    fn interruptable(&self) -> bool {
        false
    }
    fn can_use(&mut self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) -> bool {
        let playful = variant(m) == PLAYFUL;
        if !((m.baby() || playful) && e.on_ground) || !can_perform_action(m) {
            return false;
        }
        let angle = e.y_rot * 0.017453292;
        let xd = -mth::sin(angle as f64);
        let zd = mth::cos(angle as f64);
        let sign = |v: f32| if v > 0.0 { 1 } else if v < 0.0 { -1 } else { 0 };
        let xs = if xd.abs() as f64 > 0.5 { sign(xd) } else { 0 };
        let zs = if zd.abs() as f64 > 0.5 { sign(zd) } else { 0 };
        if kiln_data::blocks_types::is_air(level.block(e.block_position().offset(xs, -1, zs))) {
            return true;
        }
        if playful && e.random.next_int_bounded(mth::reduced_tick_delay(60)) == 1 {
            return true;
        }
        e.random.next_int_bounded(mth::reduced_tick_delay(500)) == 1
    }
    fn can_continue(&mut self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel) -> bool {
        false
    }
    fn start(&mut self, _e: &mut Entity, m: &mut MobData, _level: &mut dyn EntityLevel) {
        set_flag(m, ROLL, true);
    }
}
