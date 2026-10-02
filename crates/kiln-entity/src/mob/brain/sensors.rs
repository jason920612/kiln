//! The shared sensors: `NearestLivingEntitySensor`, `PlayerSensor`, `HurtBySensor`,
//! `IsInWaterSensor`, `AdultSensor`, `TemptingSensor`, `MobSensor`.

use super::memory::{NearestVisible, Val};
use super::util::{self, Targeting};
use super::{Cx, Mem, Sensor};
use crate::math::Aabb;
use crate::mob::goals::{self, Living};
use crate::sensor_boilerplate;

/// `NearestLivingEntitySensor`: the living entities (and players) within the follow range,
/// nearest first, and the same as a `NearestVisibleLivingEntities`.
#[derive(Clone, Debug)]
pub struct NearestLivingEntities;

impl Sensor for NearestLivingEntities {
    fn name(&self) -> &'static str {
        "NearestLivingEntitySensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestLivingEntities, Mem::NearestVisibleLivingEntities]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let r = util::follow_range(cx);
        let area = cx.e.bounding_box().inflate(r, r, r);
        let typed = util::living_in_box_typed(cx, &area);
        let list: Vec<i32> = typed.iter().map(|&(id, _)| id).collect();
        let kinds: Vec<&'static str> = typed.iter().map(|&(_, t)| t).collect();
        cx.b.mem.set(Mem::NearestLivingEntities, Val::Entities(list.clone()));
        cx.b.mem.set(Mem::NearestVisibleLivingEntities, Val::Visible(NearestVisible::new(list, kinds)));
    }
    sensor_boilerplate!();
}

/// `PlayerSensor`: players within the follow range, the visible ones, the attackable ones.
#[derive(Clone, Debug)]
pub struct Players;

impl Sensor for Players {
    fn name(&self) -> &'static str {
        "PlayerSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestPlayers, Mem::NearestVisiblePlayer, Mem::NearestVisibleAttackablePlayer, Mem::NearestVisibleAttackablePlayers]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let range = util::follow_range(cx);
        let mut players: Vec<(f64, i32)> = Vec::new();
        for p in goals::players_around(cx.e, &*cx.level, range) {
            if p.spectator {
                continue;
            }
            // `closerThan(player, followRange)`.
            let d = cx.e.position().distance_to_sqr(p.pos);
            if d < range * range {
                players.push((d, p.id));
            }
        }
        players.sort_by(|a, b| a.0.total_cmp(&b.0));
        let players: Vec<i32> = players.into_iter().map(|(_, id)| id).collect();
        cx.b.mem.set(Mem::NearestPlayers, Val::Entities(players.clone()));
        let mut visible = Vec::new();
        for &id in &players {
            if let Some(l) = util::living(cx, id)
                && util::is_entity_targetable(cx, &l)
            {
                visible.push(id);
            }
        }
        cx.b.mem.set_opt(Mem::NearestVisiblePlayer, visible.first().map(|&id| Val::Entity(id)));
        let mut attackable = Vec::new();
        for &id in &visible {
            if let Some(l) = util::living(cx, id)
                && util::is_entity_attackable(cx, &l)
            {
                attackable.push(id);
            }
        }
        cx.b.mem.set(Mem::NearestVisibleAttackablePlayers, Val::Entities(attackable.clone()));
        cx.b.mem.set_opt(Mem::NearestVisibleAttackablePlayer, attackable.first().map(|&id| Val::Entity(id)));
    }
    sensor_boilerplate!();
}

/// `HurtBySensor`.
#[derive(Clone, Debug)]
pub struct HurtBy;

impl Sensor for HurtBy {
    fn name(&self) -> &'static str {
        "HurtBySensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::HurtBy, Mem::HurtByEntity]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        match cx.m.last_damage_source(cx.time) {
            Some(src) => {
                cx.b.mem.set(Mem::HurtBy, Val::Damage(src));
                if let Some(a) = src.attacker
                    && util::living(cx, a).is_some()
                {
                    cx.b.mem.set(Mem::HurtByEntity, Val::Entity(a));
                }
            }
            None => cx.b.mem.erase(Mem::HurtBy),
        }
        if let Some(id) = cx.b.mem.entity(Mem::HurtByEntity)
            && !util::living(cx, id).is_some_and(|l| l.alive)
        {
            cx.b.mem.erase(Mem::HurtByEntity);
        }
    }
    sensor_boilerplate!();
}

/// `IsInWaterSensor`.
#[derive(Clone, Debug)]
pub struct IsInWater;

impl Sensor for IsInWater {
    fn name(&self) -> &'static str {
        "IsInWaterSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::IsInWater]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if cx.e.is_in_water() {
            cx.b.mem.set(Mem::IsInWater, Val::Unit);
        } else {
            cx.b.mem.erase(Mem::IsInWater);
        }
    }
    sensor_boilerplate!();
}

/// `AdultSensor` (`NEAREST_ADULT`): the closest visible adult of the same type.
#[derive(Clone, Debug)]
pub struct Adult {
    /// `AdultSensorAnyType`: any type.
    pub any_type: bool,
}

impl Sensor for Adult {
    fn name(&self) -> &'static str {
        "AdultSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestVisibleAdult, Mem::NearestVisibleLivingEntities]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if !cx.b.mem.has(Mem::NearestVisibleLivingEntities) {
            return;
        }
        let ty = cx.e.type_name;
        let any = self.any_type;
        let found = util::find_closest_visible(cx, |cx, id| {
            let Some(other) = cx.level.entity(id) else { return false };
            (any || other.type_name == ty) && crate::mob::data(other).is_some_and(|d| !d.baby())
        });
        cx.b.mem.set_opt(Mem::NearestVisibleAdult, found.map(Val::Entity));
    }
    sensor_boilerplate!();
}

/// `TemptingSensor`: the nearest player holding something the mob is tempted by.
#[derive(Clone, Debug)]
pub struct Tempting {
    /// `TemptingSensor.forAnimal`: `Animal.isFood`; otherwise these item ids.
    pub items: Option<&'static [&'static str]>,
}

impl Tempting {
    pub fn for_animal() -> Tempting {
        Tempting { items: None }
    }
}

impl Sensor for Tempting {
    fn name(&self) -> &'static str {
        "TemptingSensor"
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::TemptingPlayer]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        let range = cx.m.attrs.value(crate::mob::attributes::Attr::TemptRange) as f32 as f64;
        let cond = Targeting::non_combat().ignore_line_of_sight().range(range);
        let mut found: Vec<(f64, i32)> = Vec::new();
        for p in goals::players_around(cx.e, &*cx.level, range) {
            if p.spectator {
                continue;
            }
            let l = goals::living_player(&p);
            if !cond.test(cx, &l) {
                continue;
            }
            if !(self.tempts(cx, p.main_hand) || self.tempts(cx, p.off_hand)) {
                continue;
            }
            if cx.e.passengers.contains(&p.id) {
                continue;
            }
            found.push((cx.e.position().distance_to_sqr(p.pos), p.id));
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        match found.first() {
            Some(&(_, id)) => cx.b.mem.set(Mem::TemptingPlayer, Val::Entity(id)),
            None => cx.b.mem.erase(Mem::TemptingPlayer),
        }
    }
    sensor_boilerplate!();
}

impl Tempting {
    fn tempts(&self, cx: &Cx, item: i32) -> bool {
        if item == 0 {
            return false;
        }
        match self.items {
            Some(list) => list.iter().any(|n| kiln_data::builtin_id("minecraft:item", n) == Some(item)),
            None => cx.m.kind.ext().is_some_and(|k| k.is_food(item)),
        }
    }
}

/// `MobSensor`: sets `to_set` (for `ttl` ticks) when one of the nearest living entities passes the
/// test and the mob is ready, clears it when it is not.
#[derive(Clone, Debug)]
pub struct MobSensor {
    pub scan_rate: i32,
    pub mob_test: fn(&mut Cx, &Living) -> bool,
    pub ready_test: fn(&Cx) -> bool,
    pub to_set: Mem,
    pub ttl: i64,
}

impl Sensor for MobSensor {
    fn name(&self) -> &'static str {
        "MobSensor"
    }
    fn scan_rate(&self) -> i32 {
        self.scan_rate
    }
    fn requires(&self) -> &'static [Mem] {
        &[Mem::NearestLivingEntities]
    }
    fn do_tick(&mut self, cx: &mut Cx) {
        if !(self.ready_test)(cx) {
            cx.b.mem.erase(self.to_set);
            return;
        }
        let ids = cx.b.mem.entities(Mem::NearestLivingEntities).to_vec();
        if !cx.b.mem.has(Mem::NearestLivingEntities) {
            return;
        }
        for id in ids {
            if let Some(l) = util::living(cx, id)
                && (self.mob_test)(cx, &l)
            {
                cx.b.mem.set_expiring(self.to_set, Val::Bool(true), self.ttl);
                break;
            }
        }
    }
    sensor_boilerplate!();
}

/// A box around the mob for the sensors that look at a fixed range (`getBoundingBox().inflate`).
pub fn around(cx: &Cx, h: f64, v: f64) -> Aabb {
    cx.e.bounding_box().inflate(h, v, h)
}
