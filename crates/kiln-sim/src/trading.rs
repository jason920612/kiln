//! Villager trading in the simulation: rolling trade sets for villagers, opening the merchant
//! screen after a click, carrying the menu's trades and closing back to the villager, and
//! closing screens whose villager went away (`MerchantMenu.stillValid`).
//!
//! Approximations: vanilla rolls a trade set from a server-wide random sequence
//! (`minecraft:trade_set/<profession>/level_<n>`, persisted in the world); Kiln seeds each roll
//! from the world seed, the tick and the villager, so offers do not depend on region order but
//! differ from vanilla's. Special prices (gossip, Hero of the Village), demand restocking at job
//! sites and the trade statistics are not simulated.

use crate::entities::{self, Entities, Spawn};
use crate::{Player, blocks::RegionLevel, health};
use kiln_entity::level::TradeMerchant;
use kiln_entity::mob::kinds::villager;
use kiln_inventory::merchant::{MerchantEvent, MerchantState};
use kiln_item::trading::{MerchantOffer, merchant_offers_packet};

/// `addOffersFromTradeSet` for `merchant`: the trade set's offers from the loot data's trades.
pub(crate) fn roll_offers(loot: Option<&kiln_loot::LootData>, seed: i64, game_time: i64, set: &str, merchant: &TradeMerchant) -> Vec<MerchantOffer> {
    let (Some(loot), Some(id)) = (loot, kiln_item::Identifier::parse(set)) else { return Vec::new() };
    let ctx = kiln_loot::trade::TradeContext {
        origin: [merchant.pos.x, merchant.pos.y, merchant.pos.z],
        entity_type: "minecraft:villager",
        villager_type: merchant.villager_type,
    };
    let salt = set.bytes().fold(0x7472_6164u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    let mut rng = kiln_javamath::random::LegacyRandom::new(crate::mobs::loot_seed(seed, game_time, merchant.entity, salt));
    loot.trades.offers(loot, &id, &ctx, &mut rng)
}

/// The villager's screen title (`getDisplayName`: the profession's name).
fn title(profession: &str) -> kiln_proto::nbt::Tag {
    use kiln_proto::nbt::Tag;
    let path = profession.strip_prefix("minecraft:").unwrap_or(profession);
    Tag::Compound(vec![("translate".into(), Tag::String(format!("entity.minecraft.villager.{path}")))])
}

/// After a click on `target`: a villager that started trading with the player opens its screen
/// (`Merchant.openTradingScreen`): Open Screen, the menu's content, then the offers.
pub(crate) fn open_if_requested(entities: &mut Entities, p: &mut Player, target: i32, rules: &kiln_inventory::Rules, spawns: &mut Vec<Spawn>) {
    let Ok(idx) = entities.list.binary_search_by_key(&target, |e| e.id) else { return };
    let Some(phys) = entities.list[idx].phys.as_mut() else { return };
    let Some(m) = kiln_entity::mob::data_mut(phys) else { return };
    let Some(st) = villager::state_mut(m) else { return };
    if st.open_for != Some(p.entity_id) {
        return;
    }
    st.open_for = None;
    let offers = st.offers.clone().unwrap_or_default();
    p.award_stat(*crate::player_stats::stat::TALKED_TO_VILLAGER, 1);
    let (level, xp, profession) = (st.level, st.xp, st.profession);
    // `ServerPlayer.openMenu`: another open screen closes first.
    if let Some(open) = p.open_menu.as_ref() {
        let id = open.container_id;
        p.send(kiln_inventory::effect::container_close(id));
        p.with_menu(rules, spawns, |open, inventory_menu, env| kiln_inventory::click::close_container(open, inventory_menu, env));
        p.open_menu = None;
    }
    let id = kiln_inventory::click::next_container_id(&mut p.containers.counter);
    let menu = kiln_inventory::Menu::merchant(id, MerchantState::new(target, offers.clone()));
    let Some(menu_type) = menu.kind.menu_type_id() else { return };
    p.send(kiln_inventory::effect::open_screen(id, menu_type, &title(profession)));
    p.open_menu = Some(menu);
    p.with_menu(rules, spawns, |open, _, env| open.open(env));
    if !offers.is_empty() {
        p.send(merchant_offers_packet(id, &offers, level, xp, true, true));
    }
}

/// Applies what player `i`'s merchant screen told its villager (trades, preview sounds,
/// closing). A level up adds offers, which the screen gets at once.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_events(
    entities: &mut Entities,
    level: &mut RegionLevel,
    players: &mut [&mut Player],
    i: usize,
    spawns: &mut Vec<Spawn>,
    deaths: &mut Vec<health::Death>,
) {
    let events = std::mem::take(&mut players[i].merchant_events);
    for (n, (target, event)) in events.into_iter().enumerate() {
        let salt = 0x7472_0000 | n as u64;
        match event {
            MerchantEvent::Trade { index } => {
                let r = entities::with_entity(entities, level, players, target, spawns, deaths, salt, |e, lvl| {
                    villager::with_villager(e, |e, m| {
                        let t = villager::notify_trade(e, m, lvl, index);
                        let st = villager::state(m)?;
                        Some((t, st.offers.clone().unwrap_or_default(), st.level, st.xp))
                    })
                    .flatten()
                });
                if let Some(Some((t, offers, lvl, xp))) = r
                    && t.leveled_up
                {
                    let p = &mut *players[i];
                    if let Some(open) = p.open_menu.as_mut()
                        && let Some(st) = open.merchant_state_mut()
                        && st.merchant == target
                    {
                        // The menu's copy counted its own uses; the villager's list is the truth.
                        st.offers = offers.clone();
                        let id = open.container_id;
                        p.send(merchant_offers_packet(id, &offers, lvl, xp, true, true));
                    }
                }
            }
            MerchantEvent::TradeUpdated { has_result } => {
                entities::with_entity(entities, level, players, target, spawns, deaths, salt, |e, lvl| {
                    villager::with_villager(e, |e, m| villager::notify_trade_updated(e, m, lvl, has_result))
                });
            }
            MerchantEvent::Closed => stop_trading(entities, target),
            MerchantEvent::TradedStat => players[i].award_stat(*crate::player_stats::stat::TRADED_WITH_VILLAGER, 1),
        }
    }
}

fn stop_trading(entities: &mut Entities, target: i32) {
    let Ok(idx) = entities.list.binary_search_by_key(&target, |e| e.id) else { return };
    if let Some(m) = entities.list[idx].phys.as_mut().and_then(kiln_entity::mob::data_mut) {
        villager::stop_trading(m);
    }
}

/// `MerchantMenu.stillValid` each tick: the screen closes when its villager died, left, got too
/// far (interaction range + 4) or stopped trading; villagers whose trading player has no
/// screen open for them any more stop trading.
pub(crate) fn check_menus(entities: &mut Entities, players: &mut [&mut Player], rules: &kiln_inventory::Rules, spawns: &mut Vec<Spawn>) {
    for p in players.iter_mut() {
        let Some(target) = p.open_menu.as_ref().and_then(|m| m.merchant_state()).map(|s| s.merchant) else { continue };
        let valid = entities.list.binary_search_by_key(&target, |e| e.id).ok().is_some_and(|idx| {
            let e = &entities.list[idx];
            let Some(phys) = e.phys.as_ref() else { return false };
            let Some(m) = kiln_entity::mob::data(phys) else { return false };
            let trading = villager::state(m).is_some_and(|s| s.trading_player == Some(p.entity_id));
            let bb = phys.bounding_box();
            let eye = p.eye_position();
            let d = |v: f64, lo: f64, hi: f64| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
            let (dx, dy, dz) = (d(eye[0], bb.min_x, bb.max_x), d(eye[1], bb.min_y, bb.max_y), d(eye[2], bb.min_z, bb.max_z));
            let range = p.attribute(crate::combat::ENTITY_INTERACTION_RANGE) + 4.0;
            !e.removed && trading && kiln_entity::mob::is_alive(phys, m) && dx * dx + dy * dy + dz * dz < range * range
        });
        if !valid && !p.dead {
            // `ServerPlayer.closeContainer`.
            if let Some(id) = p.open_menu.as_ref().map(|m| m.container_id) {
                p.send(kiln_inventory::effect::container_close(id));
            }
            p.with_menu(rules, spawns, |open, inventory_menu, env| kiln_inventory::click::close_container(open, inventory_menu, env));
            p.open_menu = None;
            p.merchant_events.clear();
            stop_trading(entities, target);
        }
    }
    for e in entities.list.iter_mut() {
        let Some(m) = e.phys.as_mut().and_then(kiln_entity::mob::data_mut) else { continue };
        let Some(st) = villager::state_mut(m) else { continue };
        let Some(who) = st.trading_player else { continue };
        let id = e.id;
        let screen_open = players.iter().any(|p| p.entity_id == who && p.open_menu.as_ref().and_then(|m| m.merchant_state()).is_some_and(|s| s.merchant == id));
        if !screen_open && st.open_for.is_none() {
            st.trading_player = None;
        }
    }
}
