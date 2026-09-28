//! Zombie villager: a zombie that remembers its villager data (type, profession, level) and, once
//! curing has started, turns back into a villager.
//!
//! Curing is started by a golden apple on a zombie villager with weakness (the weakness goes,
//! strength comes for the conversion time); the conversion runs faster near iron bars and beds
//! and ends in a villager with the zombie villager's data, experience and offers.

use super::zombie::{self, HasZombie, ZombieState};
use crate::entity::Entity;
use crate::level::{EntityLevel, Event};
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{Info, Kind, MobExt};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::mob::goals::Living;
use crate::mob::{self, DamageSource, GroupData, MobData, MobKind, SpawnContext};
use crate::persist::{Input, Output};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct ZombieVillager;

pub static KIND: ZombieVillager = ZombieVillager;

static INFO: Info = Info {
    burns_in_daylight: true,
    breathes_under_water: true,
    ..Info::monster("minecraft:zombie_villager", &[(FollowRange, 35.0), (MovementSpeed, 0.23000000417232513), (AttackDamage, 3.0), (Armor, 2.0), (SpawnReinforcements, 0.0)])
};

#[derive(Clone, Debug)]
pub struct ZombieVillagerState {
    pub zombie: ZombieState,
    /// `VillagerData`: `minecraft:villager_type` and `minecraft:villager_profession` names, level.
    pub villager_type: String,
    pub profession: String,
    pub level: i32,
    pub finalized: bool,
    pub xp: i32,
    /// `villagerConversionTime` (-1: not converting).
    pub conversion_time: i32,
    pub conversion_player: Option<u128>,
    /// `tradeOffers` kept from the villager it was (given to the cured villager).
    pub offers: Option<Vec<kiln_item::trading::MerchantOffer>>,
}

impl HasZombie for ZombieVillagerState {
    fn zombie(&self) -> &ZombieState {
        &self.zombie
    }
    fn zombie_mut(&mut self) -> &mut ZombieState {
        &mut self.zombie
    }
}

fn st(m: &MobData) -> &ZombieVillagerState {
    crate::mob::ext::state::<ZombieVillagerState>(m).expect("zombie villager state")
}

fn st_mut(m: &mut MobData) -> &mut ZombieVillagerState {
    crate::mob::ext::state_mut::<ZombieVillagerState>(m).expect("zombie villager state")
}

fn professions() -> &'static [&'static str] {
    kiln_data::builtin_entries("minecraft:villager_profession").unwrap_or(&[])
}

/// `initializeZombieVillagerData`: plains, a random profession (any, from the entity's random).
fn random_profession(random: &mut dyn RandomSource) -> String {
    let p = professions();
    if p.is_empty() {
        return "minecraft:none".into();
    }
    p[random.next_int_bounded(p.len() as i32) as usize].to_owned()
}

/// `VillagerType.byBiome`.
fn villager_type_for_biome(biome: Option<i32>) -> &'static str {
    let name = biome.and_then(|b| kiln_data::builtin_entries("minecraft:worldgen/biome").and_then(|e| e.get(b as usize).copied())).unwrap_or("");
    match name {
        "minecraft:desert" | "minecraft:badlands" | "minecraft:eroded_badlands" | "minecraft:wooded_badlands" => "minecraft:desert",
        "minecraft:jungle" | "minecraft:sparse_jungle" | "minecraft:bamboo_jungle" => "minecraft:jungle",
        "minecraft:savanna" | "minecraft:savanna_plateau" | "minecraft:windswept_savanna" => "minecraft:savanna",
        "minecraft:snowy_plains" | "minecraft:ice_spikes" | "minecraft:deep_frozen_ocean" | "minecraft:frozen_ocean" | "minecraft:frozen_river"
        | "minecraft:grove" | "minecraft:jagged_peaks" | "minecraft:snowy_beach" | "minecraft:snowy_slopes" | "minecraft:snowy_taiga"
        | "minecraft:frozen_peaks" => "minecraft:snow",
        "minecraft:mangrove_swamp" | "minecraft:swamp" => "minecraft:swamp",
        "minecraft:old_growth_spruce_taiga" | "minecraft:old_growth_pine_taiga" | "minecraft:taiga" | "minecraft:windswept_forest"
        | "minecraft:windswept_gravelly_hills" | "minecraft:windswept_hills" => "minecraft:taiga",
        _ => "minecraft:plains",
    }
}

impl Kind for ZombieVillager {
    fn info(&self) -> &'static Info {
        &INFO
    }

    /// `defineSynchedData` draws the profession from the entity's random at construction.
    fn new_state(&self, _m: &mut MobData, random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(ZombieVillagerState {
            zombie: ZombieState::default(),
            villager_type: "minecraft:plains".into(),
            profession: random_profession(random),
            level: 1,
            finalized: false,
            xp: 0,
            conversion_time: -1,
            conversion_player: None,
            offers: None,
        }))
    }

    fn register_goals(&self, m: &mut MobData) {
        zombie::register_goals(m);
    }

    /// `ZombieVillager.tick` before `super.tick()`: the curing countdown.
    fn pre_tick(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
        if !mob::is_alive(e, m) || st(m).conversion_time < 0 {
            return;
        }
        let progress = conversion_progress(e, level);
        let s = st_mut(m);
        s.conversion_time -= progress;
        if s.conversion_time <= 0 {
            finish_conversion(e, m, level);
        }
    }

    /// `mobInteract`: a golden apple starts the cure of a weakened zombie villager.
    fn interact(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, who: &Interactor, stack: &kiln_item::ItemStack) -> Option<Outcome> {
        if stack.is_empty() || mob::item_name(stack) != "minecraft:golden_apple" {
            return None;
        }
        if !mob::effects::has(m, crate::effect::ids::weakness()) {
            // `CONSUME`: the click is taken, nothing happens.
            return Some(Outcome::success(HeldChange::None));
        }
        let player = level.player(who.id).map(|p| p.uuid);
        let time = e.random.next_int_bounded(2401) + 3600;
        start_converting(m, player, time);
        level.emit(Event::EntityEvent { entity: e.id, event: 16 });
        Some(Outcome::success(HeldChange::Consume(1)))
    }

    fn after_hurt(&self, e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel, source: &DamageSource, _amount: f32, hurt: bool) {
        if hurt {
            zombie::reinforcements(e, m, level, source);
        }
    }

    fn after_hurt_target(&self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn EntityLevel, _t: &Living) {}

    /// `finalizeVillagerType` (from the biome, unless finalized), then the zombie's.
    fn finalize_spawn(&self, e: &mut Entity, m: &mut MobData, r: &mut dyn RandomSource, ctx: &SpawnContext, group: &mut GroupData) {
        if !st(m).finalized {
            st_mut(m).villager_type = villager_type_for_biome(ctx.biome).into();
        }
        zombie::finalize(e, m, r, ctx, group, false);
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        zombie::load(e, m, r);
        let data = r.get("VillagerData").cloned();
        let finalized = r.bool_or("VillagerDataFinalized", false);
        if finalized || data.is_some() {
            let s = st_mut(m);
            s.finalized = true;
            match data {
                Some(d) => {
                    if let Some(t) = d.get("type").and_then(Tag::as_str) {
                        s.villager_type = t.to_owned();
                    }
                    if let Some(p) = d.get("profession").and_then(Tag::as_str) {
                        s.profession = p.to_owned();
                    }
                    s.level = d.get("level").and_then(Tag::as_f64).map_or(1, |l| l as i32);
                }
                None => s.profession = random_profession(&mut e.random),
            }
        }
        let offers = r.get("Offers").map(kiln_item::trading::offers_from_nbt);
        st_mut(m).offers = offers;
        let t = r.int_or("ConversionTime", -1);
        let player = r.uuid("ConversionPlayer");
        let xp = r.int_or("Xp", 0);
        if t != -1 {
            start_converting(m, player, t);
        } else {
            let s = st_mut(m);
            s.conversion_time = -1;
            s.conversion_player = None;
        }
        st_mut(m).xp = xp;
    }

    fn save(&self, e: &Entity, m: &MobData, o: &mut Output) {
        zombie::save(e, m, o);
        let s = st(m);
        o.put(
            "VillagerData",
            Tag::Compound(vec![
                ("level".into(), Tag::Int(s.level)),
                ("profession".into(), Tag::String(s.profession.clone())),
                ("type".into(), Tag::String(s.villager_type.clone())),
            ]),
        );
        o.put("VillagerDataFinalized", Tag::Byte(s.finalized as i8));
        if let Some(offers) = &s.offers {
            o.put("Offers", kiln_item::trading::offers_to_nbt(offers));
        }
        o.put("ConversionTime", Tag::Int(s.conversion_time));
        if let Some(p) = s.conversion_player {
            o.put("ConversionPlayer", crate::persist::uuid_to_tag(p));
        }
        o.put("Xp", Tag::Int(s.xp));
    }

    fn entity_data(&self, e: &Entity, m: &MobData, d: &mut EntityData) {
        use kiln_data::entities::data::zombie_villager as f;
        zombie::entity_data(e, m, d);
        let s = st(m);
        if s.conversion_time >= 0 {
            d.set(f::CONVERTING, &DataValue::Boolean(true));
        }
        let kind = kiln_data::builtin_id("minecraft:villager_type", &s.villager_type).unwrap_or(0);
        let profession = kiln_data::builtin_id("minecraft:villager_profession", &s.profession).unwrap_or(0);
        d.set(f::VILLAGER_DATA, &DataValue::VillagerData { kind, profession, level: s.level });
        if s.finalized {
            d.set(f::VILLAGER_DATA_FINALIZED, &DataValue::Boolean(true));
        }
    }

    fn dimensions(&self, m: &MobData, base: (f32, f32, f32)) -> (f32, f32, f32) {
        if m.baby() { zombie::baby_dimensions(m.kind) } else { base }
    }

    /// `removeWhenFarAway`: not while curing or after trading.
    fn remove_when_far_away(&self, m: &MobData) -> Option<bool> {
        let s = st(m);
        Some(s.conversion_time < 0 && s.xp == 0)
    }
}

/// `startConverting`: the countdown and the player who started it; the weakness goes and
/// strength (level 0: `min(difficulty - 1, 0)` clamped) lasts the conversion. The entity event
/// (the cure sound) is the caller's.
fn start_converting(m: &mut MobData, player: Option<u128>, time: i32) {
    let s = st_mut(m);
    s.conversion_player = player;
    s.conversion_time = time;
    mob::effects::remove(m, crate::effect::ids::weakness());
    mob::effects::add_quiet(m, crate::effect::Effect::simple(crate::effect::ids::strength(), time, 0));
}

/// `getConversionProgress`: 1, and 1% of the time a chance per iron bar or bed nearby.
fn conversion_progress(e: &mut Entity, level: &dyn EntityLevel) -> i32 {
    let mut progress = 1;
    if e.random.next_float() < 0.01 {
        let mut count = 0;
        let (bx, by, bz) = (e.x() as i32, e.y() as i32, e.z() as i32);
        let mut x = bx - 4;
        while x < bx + 4 && count < 14 {
            let mut y = by - 4;
            while y < by + 4 && count < 14 {
                let mut z = bz - 4;
                while z < bz + 4 && count < 14 {
                    let s = level.block(crate::math::BlockPos::new(x, y, z));
                    let name = crate::blocks::block_name(s);
                    if name == "minecraft:iron_bars" || name.ends_with("_bed") {
                        if e.random.next_float() < 0.3 {
                            progress += 1;
                        }
                        count += 1;
                    }
                    z += 1;
                }
                y += 1;
            }
            x += 1;
        }
    }
    progress
}

/// `finishConversion`: a villager with the zombie villager's data takes its place.
fn finish_conversion(e: &mut Entity, m: &mut MobData, level: &mut dyn EntityLevel) {
    let zombie = crate::level::Seen::of_mob(e, m);
    let starter = st(m).conversion_player.and_then(|u| level.players().iter().find(|p| p.uuid == u).map(|p| p.id));
    let (vtype, profession, vlevel, finalized, xp) = {
        let s = st(m);
        (s.villager_type.clone(), s.profession.clone(), s.level, s.finalized, s.xp)
    };
    let offers = st_mut(m).offers.take();
    mob::convert::convert_to(e, m, level, MobKind::Villager, false, false, |ne, nm, level| {
        // `setVillagerDataFinalized`, `setVillagerData`, `setOffers`, `setVillagerXp`.
        if let Some(v) = super::villager::state_mut(nm) {
            let name = |reg: &str, n: &str| kiln_data::builtin_entries(reg).and_then(|e| e.iter().find(|x| **x == n).copied());
            v.finalized = finalized;
            v.villager_type = name("minecraft:villager_type", &vtype).unwrap_or("minecraft:plains");
            v.set_profession(name("minecraft:villager_profession", &profession).unwrap_or("minecraft:none"));
            v.level = vlevel;
            v.offers = offers;
            v.xp = xp;
        }
        // `CuredZombieVillagerTrigger` for the player who started the cure.
        if let Some(player) = starter {
            let villager = crate::level::Seen::of_mob(ne, nm);
            level.emit(Event::Criterion { player, criterion: crate::level::Criterion::CuredZombieVillager { zombie: zombie.clone(), villager } });
        }
        let eff = level.effective_difficulty(ne.block_position());
        let ctx = SpawnContext {
            biome: None,
            moon_brightness: 1.0,
            special_multiplier: zombie::special_multiplier(eff),
            effective_difficulty: eff,
            hard: level.difficulty() == 3,
            halloween: false,
        };
        mob::put(ne, Box::new(std::mem::replace(nm, MobData::new(MobKind::Villager, &mut kiln_javamath::random::LegacyRandom::new(0)))));
        mob::finalize_spawn(ne, level.random(), &ctx, &mut GroupData::default(), false);
        *nm = *mob::take(ne);
        if let Some(fx) = crate::effect::Effect::named("minecraft:nausea", 200, 0) {
            mob::effects::add(ne, nm, level, fx, None);
        }
        if !ne.silent {
            level.emit(Event::LevelEvent { event: 1027, pos: ne.block_position(), data: 0 });
        }
    });
}
