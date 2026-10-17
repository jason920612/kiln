//! Villager trading against a vanilla 26.3 server, bit for bit (`tools/VillagerVectors.java`, `KILN_VILLAGER_VECTORS`): gossip sequences
//! (`GossipContainer.add`, `decay`, `transferFrom`), offer prices through demand updates, use resets and special prices, `updateSpecialPrices` for a
//! reputation and Hero of the Village, and scripts of `shouldRestock` / `restock` over time (restocks twice a day, 2400 ticks apart, the day roll-over,
//! the 12000-tick reset with its demand catch-up). Skipped without the vectors.

use kiln_entity::memory::MemoryLevel;
use kiln_entity::mob::gossip::{GossipType, Gossips};
use kiln_entity::mob::kinds::villager;
use kiln_entity::mob::{self, MobKind};
use kiln_item::ItemStack;
use kiln_item::trading::{ItemCost, MerchantOffer};
use serde_json::Value;

/// `new UUID(i + 1, i + 1)`.
fn person(i: usize) -> u128 {
    let n = (i + 1) as u128;
    (n << 64) | n
}

fn offer_of(s: &Value) -> MerchantOffer {
    let cost = |c: &Value| ItemCost::new(kiln_data::builtin_id("minecraft:item", c[0].as_str().unwrap()).unwrap(), c[1].as_i64().unwrap() as i32);
    let mult = s["mult"].as_str().map_or_else(|| s["mult"].as_f64().unwrap() as f32, |t| t.parse().unwrap());
    let mut o = MerchantOffer::new(
        cost(&s["a"]),
        (!s["b"].is_null()).then(|| cost(&s["b"])),
        ItemStack::of("minecraft:bread", s["result"].as_i64().unwrap() as i32).unwrap(),
        s["max_uses"].as_i64().unwrap() as i32,
        s["xp"].as_i64().unwrap() as i32,
        mult,
    );
    o.uses = s["uses"].as_i64().unwrap() as i32;
    o.demand = s["demand"].as_i64().unwrap() as i32;
    o.special_price_diff = s["special"].as_i64().unwrap() as i32;
    o
}

fn describe(o: &MerchantOffer) -> Vec<i64> {
    vec![o.cost_a().count() as i64, if o.cost_b.is_some() { o.cost_b().count() as i64 } else { 0 }, o.uses as i64, o.demand as i64, o.special_price_diff as i64, o.needs_restock() as i64]
}

fn nums(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().map(|x| x.as_i64().unwrap()).collect()
}

fn offers_of(v: &Value) -> Vec<Vec<i64>> {
    v.as_array().unwrap().iter().map(nums).collect()
}

/// Per person the five gossip values, then the three reputations (the order of `GossipType.values()` is the one of `GossipType::ALL`).
fn gossip_state(g: &Gossips) -> Vec<i64> {
    let mut out = Vec::new();
    for p in 0..3 {
        let entry = g.entries.iter().find(|(t, _)| *t == person(p));
        for i in 0..GossipType::ALL.len() {
            out.push(entry.map_or(0, |(_, v)| v[i]) as i64);
        }
    }
    for p in 0..3 {
        out.push(g.reputation(person(p)) as i64);
    }
    out
}

fn flat(v: &Value) -> Vec<i64> {
    v.as_array().unwrap().iter().flat_map(nums).collect()
}

#[test]
fn villager_trading_matches_vanilla() {
    let Some(path) = std::env::var_os("KILN_VILLAGER_VECTORS") else {
        eprintln!("skipped: set KILN_VILLAGER_VECTORS (tools/VillagerVectors.java)");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut failures = Vec::new();
    let (mut cases, mut steps_total) = (0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        cases += 1;
        match v["kind"].as_str().unwrap() {
            "gossip" => {
                let (mut g, mut other) = (Gossips::default(), Gossips::default());
                for (k, step) in v["steps"].as_array().unwrap().iter().enumerate() {
                    let op = step["op"].as_array().unwrap();
                    let i = |n: usize| op[n].as_i64().unwrap() as i32;
                    let t = |n: usize| GossipType::ALL[op[n].as_i64().unwrap() as usize];
                    match op[0].as_str().unwrap() {
                        "add" => g.add(person(i(1) as usize), t(2), i(3)),
                        "decay" => g.decay(),
                        "other" => other.add(person(i(1) as usize), t(2), i(3)),
                        "transfer" => {
                            let seed: i64 = op[1].as_str().unwrap().parse().unwrap();
                            let mut r = kiln_javamath::random::LegacyRandom::new(seed);
                            let picked = g.transfer_from(&other, &mut r, i(2));
                            if picked as i64 != op[3].as_i64().unwrap() {
                                failures.push(format!("{name} step {k}: transfer picked {picked}, vanilla {}", op[3]));
                            }
                        }
                        other => panic!("op {other}"),
                    }
                    steps_total += 1;
                    let want = flat(&step["state"]);
                    let got = gossip_state(&g);
                    if got != want {
                        failures.push(format!("{name} step {k} {op:?}: Kiln {got:?}\n    vanilla {want:?}"));
                        break;
                    }
                }
            }
            "offer" => {
                let mut o = offer_of(&v["setup"]);
                for (k, step) in v["steps"].as_array().unwrap().iter().enumerate() {
                    let op = step["op"].as_array().unwrap();
                    match op[0].as_str().unwrap() {
                        "demand" => o.update_demand(),
                        "reset" => o.reset_uses(),
                        "use" => (0..op[1].as_i64().unwrap()).for_each(|_| o.increase_uses()),
                        "special" => o.special_price_diff += op[1].as_i64().unwrap() as i32,
                        "unspecial" => o.special_price_diff = 0,
                        other => panic!("op {other}"),
                    }
                    steps_total += 1;
                    let (got, want) = (describe(&o), nums(&step["state"]));
                    if got != want {
                        failures.push(format!("{name} step {k} {op:?}: Kiln {got:?}\n    vanilla {want:?}"));
                        break;
                    }
                }
            }
            "special" => {
                let e = mob::new(MobKind::Villager, 1, 0, 1);
                let mut m = mob::data(&e).unwrap().clone();
                let st = villager::state_mut(&mut m).unwrap();
                st.offers = Some(v["offers"].as_array().unwrap().iter().map(offer_of).collect());
                let player = person(7);
                for g in v["gossip"].as_array().unwrap() {
                    st.gossips.add(player, GossipType::ALL[g[0].as_i64().unwrap() as usize], g[1].as_i64().unwrap() as i32);
                }
                let hero = v["hero"].as_i64().unwrap();
                villager::update_special_prices(st, player, (hero >= 0).then_some(hero as i32));
                let got: Vec<Vec<i64>> = st.offers.as_ref().unwrap().iter().map(describe).collect();
                let want = offers_of(&v["after"]);
                steps_total += 1;
                if got != want {
                    failures.push(format!("{name} (hero {hero}): Kiln {got:?}\n    vanilla {want:?}"));
                }
            }
            "restock" => {
                let mut level = MemoryLevel::new(-64, 1);
                level.game_time = v["game_time"].as_i64().unwrap();
                level.day_time = v["day_time"].as_i64().unwrap();
                let e = mob::new(MobKind::Villager, 1, 0, 1);
                let mut m = mob::data(&e).unwrap().clone();
                {
                    let st = villager::state_mut(&mut m).unwrap();
                    st.offers = Some(v["offers"].as_array().unwrap().iter().map(offer_of).collect());
                    st.last_restock = v["last_restock"].as_i64().unwrap();
                    st.restocks_today = v["today"].as_i64().unwrap() as i32;
                    st.last_restock_check_day = v["check_day"].as_i64().unwrap();
                }
                for (k, step) in v["steps"].as_array().unwrap().iter().enumerate() {
                    let op = step["op"].as_array().unwrap();
                    match op[0].as_str().unwrap() {
                        "advance" => {
                            level.game_time += op[1].as_i64().unwrap();
                            level.day_time += op[1].as_i64().unwrap();
                        }
                        "use" => {
                            let st = villager::state_mut(&mut m).unwrap();
                            for _ in 0..op[2].as_i64().unwrap() {
                                st.offers.as_mut().unwrap()[op[1].as_i64().unwrap() as usize].increase_uses();
                            }
                        }
                        "should" => {
                            let should = villager::should_restock(&e, &mut m, &mut level);
                            if should != (op[1].as_i64().unwrap() == 1) {
                                failures.push(format!("{name} step {k}: shouldRestock {should}, vanilla {}", op[1]));
                            }
                            if should && op[2].as_i64() == Some(1) {
                                villager::restock(&e, &mut m, &mut level);
                            }
                        }
                        other => panic!("op {other}"),
                    }
                    let st = villager::state(&m).unwrap();
                    steps_total += 1;
                    let got_counters = vec![st.restocks_today as i64, st.last_restock, st.last_restock_check_day];
                    let got_offers: Vec<Vec<i64>> = st.offers.as_ref().unwrap().iter().map(describe).collect();
                    let (want_counters, want_offers) = (nums(&step["counters"]), offers_of(&step["offers"]));
                    if got_counters != want_counters || got_offers != want_offers {
                        failures.push(format!("{name} step {k} {op:?}: Kiln {got_counters:?} {got_offers:?}\n    vanilla {want_counters:?} {want_offers:?}"));
                        break;
                    }
                }
            }
            other => panic!("kind {other}"),
        }
    }
    for f in failures.iter().take(15) {
        eprintln!("FAIL {f}");
    }
    eprintln!("villagers: {cases} cases, {steps_total} steps, {} differ", failures.len());
    assert!(failures.is_empty(), "{} villager cases differ", failures.len());
}
