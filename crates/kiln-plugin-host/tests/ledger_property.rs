//! Design §11.3 property test: three kinds of context (region instances running in parallel
//! on their own threads, and the global instance) move money between players at random,
//! while regions split and merge at random, players move between regions and levels, and
//! players leave and come back (their namespaces go to disk and back). Money lives in
//! player-scoped balances and global escrow keys; the ledger plugin counts everything it
//! minted, so `sum(balances) + sum(escrows) == minted` after every tick, some calls trap
//! half-way (nothing of theirs may stick), and no player's balance may ever go missing.

use kiln_plugin_host::{Actor, GlobalValue, PluginRuntime, RuntimeConfig, examples};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::time::Duration;

const PLAYERS: usize = 6;

#[derive(Clone, Debug)]
enum Topology {
    None,
    /// A new region takes the players whose bit is set (a split).
    Split(u8),
    /// A region disappears into another (a merge).
    Merge(usize, usize),
}

#[derive(Clone, Debug)]
struct Tx {
    from: usize,
    to: usize,
    amount: i64,
    /// 0 pay, 1 pay then trap, 2 claim.
    kind: u8,
}

#[derive(Clone, Debug)]
struct Step {
    topology: Topology,
    /// (player, region index modulo the region count, level)
    moves: Vec<(usize, usize, u32)>,
    txs: Vec<Tx>,
    /// Players running `/sweep` in the global instance.
    sweeps: Vec<usize>,
    /// A player that leaves and joins again.
    relog: Option<usize>,
}

fn step() -> impl Strategy<Value = Step> {
    let topology = prop_oneof![
        2 => Just(Topology::None),
        1 => any::<u8>().prop_map(Topology::Split),
        1 => (0..8usize, 0..8usize).prop_map(|(a, b)| Topology::Merge(a, b)),
    ];
    let tx = (0..PLAYERS, 0..PLAYERS, 1..60i64, 0..3u8).prop_map(|(from, to, amount, kind)| Tx { from, to, amount, kind });
    (
        topology,
        prop::collection::vec((0..PLAYERS, 0..8usize, 0..2u32), 0..3),
        prop::collection::vec(tx, 0..24),
        prop::collection::vec(0..PLAYERS, 0..3),
        prop::option::of(0..PLAYERS),
    )
        .prop_map(|(topology, moves, txs, sweeps, relog)| Step { topology, moves, txs, sweeps, relog })
}

fn uuid(i: usize) -> u128 {
    0x1000 + i as u128
}

fn uuid_str(i: usize) -> String {
    uuid::Uuid::from_u128(uuid(i)).hyphenated().to_string()
}

fn names() -> Vec<String> {
    (0..PLAYERS).map(|i| format!("P{i}")).collect()
}

fn actor(names: &[String], i: usize) -> Actor<'_> {
    Actor::new(uuid(i), &names[i], false)
}

fn int(v: &Option<Vec<u8>>) -> Option<i64> {
    v.as_ref().map(|b| i64::from_le_bytes(b.as_slice().try_into().expect("8 bytes")))
}

/// Checks conservation and that every player's balance is there; returns the total.
fn check(rt: &PluginRuntime) -> i64 {
    let mut sum = 0;
    for i in 0..PLAYERS {
        let bal = int(&rt.player_value(uuid(i), "ledger", "balance"));
        sum += bal.unwrap_or_else(|| panic!("player {i} lost its balance"));
    }
    let mut minted = 0;
    for (k, v) in rt.global_entries("ledger") {
        let GlobalValue::Int(v) = v else { panic!("{k} is not an integer") };
        if k == "minted" {
            minted = v;
        } else if k.starts_with("escrow:") {
            sum += v;
        }
    }
    assert_eq!(sum, minted, "money appeared or vanished");
    sum
}

fn run(steps: Vec<Step>, case: u64) {
    let dir = std::env::temp_dir().join(format!("kiln-ledger-{}-{case}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = RuntimeConfig { data_dir: Some(dir.clone()), call_budget: Duration::from_millis(200), pool_instances: 64, ..RuntimeConfig::default() };
    let mut rt = PluginRuntime::new(vec![examples::load("ledger", "").unwrap()], cfg).unwrap();
    let names = names();
    // Region ids by level; players' (level, region).
    let mut regions: BTreeMap<u32, Vec<u64>> = BTreeMap::from([(0, vec![1]), (1, vec![2])]);
    let mut next_region = 3u64;
    let mut at: Vec<(u32, u64)> = (0..PLAYERS).map(|_| (0, 1)).collect();
    for (&dim, ids) in &regions {
        rt.sync_regions(dim, ids.iter().copied());
    }
    for i in 0..PLAYERS {
        rt.player_joined(&actor(&names, i));
    }
    rt.begin_tick();
    assert_eq!(check(&rt), 100 * PLAYERS as i64);

    for s in steps {
        // B0: topology, membership, operations.
        match s.topology {
            Topology::None => {}
            Topology::Split(mask) => {
                let dim = at[0].0;
                let id = next_region;
                next_region += 1;
                regions.get_mut(&dim).unwrap().push(id);
                for (i, a) in at.iter_mut().enumerate() {
                    if a.0 == dim && mask & (1 << i) != 0 {
                        a.1 = id;
                    }
                }
            }
            Topology::Merge(x, y) => {
                let ids = regions.get_mut(&0).unwrap();
                if ids.len() > 1 {
                    let from = ids[x % ids.len()];
                    let into = ids[y % ids.len()];
                    if from != into {
                        ids.retain(|r| *r != from);
                        for a in at.iter_mut().filter(|a| **a == (0, from)) {
                            a.1 = into;
                        }
                    }
                }
            }
        }
        for (p, r, dim) in s.moves {
            let ids = &regions[&dim];
            at[p] = (dim, ids[r % ids.len()]);
        }
        for (&dim, ids) in &regions {
            rt.sync_regions(dim, ids.iter().copied());
        }
        rt.begin_tick();
        check(&rt);

        // P: every region runs its players' transactions on its own thread.
        let mut work: Vec<((u32, u64), &mut kiln_plugin_host::RegionPlugins, Vec<&Tx>)> =
            rt.regions_mut().map(|(k, r)| (k, r, Vec::new())).collect();
        for tx in &s.txs {
            if let Some(w) = work.iter_mut().find(|w| w.0 == at[tx.from]) {
                w.2.push(tx);
            }
        }
        std::thread::scope(|scope| {
            for (_, region, txs) in work {
                let names = &names;
                scope.spawn(move || {
                    for tx in txs {
                        let msg = match tx.kind {
                            0 => format!("pay {} {}", tx.amount, uuid_str(tx.to)),
                            1 => format!("paytrap {} {}", tx.amount, uuid_str(tx.to)),
                            _ => "claim".to_owned(),
                        };
                        region.chat(&actor(names, tx.from), &msg);
                    }
                });
            }
        });

        // PX/G: the global instance sweeps escrows; a player relogs.
        for p in s.sweeps {
            rt.run_command(0, Some(&actor(&names, p)), "sweep", "");
        }
        if let Some(p) = s.relog {
            rt.player_left(&actor(&names, p));
            assert!(rt.player_value(uuid(p), "ledger", "balance").is_none(), "unloaded on leave");
            rt.save();
            rt.player_joined(&actor(&names, p));
        }
    }
    rt.begin_tick();
    let total = check(&rt);
    assert_eq!(total, 100 * PLAYERS as i64, "nothing is minted twice (rejoining players keep their balance)");
    // And everything survives a restart.
    rt.save();
    drop(rt);
    let cfg = RuntimeConfig { data_dir: Some(dir.clone()), pool_instances: 16, ..RuntimeConfig::default() };
    let mut rt = PluginRuntime::new(vec![examples::load("ledger", "").unwrap()], cfg).unwrap();
    for i in 0..PLAYERS {
        rt.player_joined(&actor(&names, i));
    }
    assert_eq!(check(&rt), total);
    let _ = std::fs::remove_dir_all(&dir);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 12, ..ProptestConfig::default() })]

    #[test]
    fn transfers_conserve_money_across_splits_merges_and_moves(steps in prop::collection::vec(step(), 1..25), case in any::<u64>()) {
        run(steps, case);
    }
}

#[test]
fn money_moves_between_regions_through_escrow() {
    // A fixed scenario: P0 (region 1) pays P1 (region 2), P1 claims next tick.
    let names = names();
    let mut rt = PluginRuntime::new(vec![examples::load("ledger", "").unwrap()], RuntimeConfig::default()).unwrap();
    rt.sync_regions(0, [1, 2]);
    for i in 0..2 {
        rt.player_joined(&actor(&names, i));
    }
    rt.region_mut(0, 1).unwrap().chat(&actor(&names, 0), &format!("pay 30 {}", uuid_str(1)));
    // The claim sees the snapshot: nothing yet.
    rt.region_mut(0, 2).unwrap().chat(&actor(&names, 1), "claim");
    assert_eq!(int(&rt.player_value(uuid(1), "ledger", "balance")), Some(100));
    rt.begin_tick();
    rt.region_mut(0, 2).unwrap().chat(&actor(&names, 1), "claim");
    assert_eq!(int(&rt.player_value(uuid(1), "ledger", "balance")), Some(130));
    assert_eq!(int(&rt.player_value(uuid(0), "ledger", "balance")), Some(70));
    rt.begin_tick();
    assert_eq!(rt.global_value("ledger", &format!("escrow:{}", uuid_str(1))), Some(GlobalValue::Int(0)));
}
