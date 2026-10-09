//! Property tests of the 1.0 API under changing region layouts (design §11.3): the state the
//! plugins keep must not depend on how the world is cut into regions.
//!
//! - **Claims** (cell-scoped data, copies into neighbouring cells through position tasks):
//!   whatever the regions are, a random sequence of claim placements and break attempts gives
//!   the decisions of a plain model that knows nothing of cells.
//! - **Shop** (player balances in the plugin's global namespace, `try-add` from region
//!   instances): money never appears or vanishes and never goes negative: after every tick
//!   `starting money - balance == prices of what the player was given`, whichever regions the
//!   purchases came from.

use kiln_plugin_host::{
    Actor, CellKey, ClickKind, ContainerClick, EffectKind, GlobalValue, PlayerAt, PlayerInfo, PluginRuntime, Registries, RuntimeConfig, Verdict, World,
    examples,
};
use proptest::prelude::*;
use std::sync::Arc;
use std::time::Duration;

const PLAYERS: usize = 3;
const GOLD: u32 = 1;

fn uuid(i: usize) -> u128 {
    0x2000 + i as u128
}

fn uuid_str(i: usize) -> String {
    uuid::Uuid::from_u128(uuid(i)).hyphenated().to_string()
}

fn names() -> Vec<String> {
    (0..PLAYERS).map(|i| format!("P{i}")).collect()
}

fn registries() -> Arc<Registries> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Arc::new(Registries::new(
        s(&["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]),
        s(&["minecraft:air", "minecraft:stone"]),
        s(&["minecraft:air", "minecraft:gold_block", "minecraft:stick"]),
        s(&["minecraft:cow"]),
    ))
}

/// How cells are cut into regions this step: `k` regions, the cell to region map mixed by `seed`.
#[derive(Clone, Copy, Debug)]
struct Layout {
    k: u64,
    seed: i32,
}

impl Layout {
    fn region_of_cell(&self, x: i32, z: i32) -> u64 {
        1 + ((x.wrapping_mul(7).wrapping_add(z.wrapping_mul(13)).wrapping_add(self.seed)).rem_euclid(self.k as i32)) as u64
    }

    fn region_of_block(&self, x: i32, z: i32) -> u64 {
        self.region_of_cell(x >> 7, z >> 7)
    }
}

struct Cut {
    layout: Layout,
    /// (uuid, player's block column) for the shop's region routing.
    players: Vec<(u128, i32, i32)>,
}

impl World for Cut {
    fn player(&self, who: u128) -> Option<PlayerAt> {
        let (_, x, z) = *self.players.iter().find(|p| p.0 == who)?;
        Some(PlayerAt {
            uuid: who,
            level: 0,
            region: self.layout.region_of_block(x, z),
            name: "P".into(),
            operator: false,
            info: PlayerInfo::default(),
        })
    }
    fn owner(&self, _: u32, x: i32, z: i32) -> Option<u64> {
        Some(self.layout.region_of_block(x, z))
    }
}

fn runtime(plugins: &[&str]) -> PluginRuntime {
    let loaded = plugins.iter().map(|id| examples::load(id, "").unwrap()).collect();
    let cfg = RuntimeConfig {
        registries: registries(),
        call_budget: Duration::from_millis(500),
        tick_budget: Duration::from_secs(3600),
        player_events_per_second: 0,
        pool_instances: 64,
        ..RuntimeConfig::default()
    };
    PluginRuntime::new(loaded, cfg).unwrap()
}

#[derive(Clone, Debug)]
struct ClaimStep {
    layout: Layout,
    /// Claim blocks placed: (player, x, z).
    places: Vec<(usize, i32, i32)>,
    /// Breaks attempted: (player, x, z).
    breaks: Vec<(usize, i32, i32)>,
}

fn claim_step() -> impl Strategy<Value = ClaimStep> {
    // Positions near the borders of cells 0..2 of both axes, so claims reach into neighbours.
    let coord = prop_oneof![-24..24i32, 104..152i32, 232..280i32];
    let at = (0..PLAYERS, coord.clone(), coord);
    (1..5u64, 0..40i32, prop::collection::vec(at.clone(), 0..3), prop::collection::vec(at, 0..8))
        .prop_map(|(k, seed, places, breaks)| ClaimStep { layout: Layout { k, seed }, places, breaks })
}

#[derive(Clone, Copy)]
struct ModelClaim {
    owner: usize,
    x: i32,
    z: i32,
}

impl ModelClaim {
    fn contains(&self, x: i32, z: i32) -> bool {
        (x - self.x).abs() <= 8 && (z - self.z).abs() <= 8
    }
}

fn run_claims(steps: Vec<ClaimStep>) {
    let mut rt = runtime(&["claims"]);
    let names = names();
    let actor = |i: usize| Actor::new(uuid(i), &names[i], false);
    let mut model: Vec<ModelClaim> = Vec::new();
    let mut counts = [0u32; PLAYERS];
    for (n, s) in steps.iter().enumerate() {
        rt.sync_regions(0, 1..=s.layout.k);
        let world = Cut { layout: s.layout, players: Vec::new() };
        for &(p, x, z) in &s.places {
            let r = rt.region_mut(0, s.layout.region_of_block(x, z)).unwrap();
            let got = r.block_place(&actor(p), [x, 64, z], [x, 63, z], Some(GOLD));
            // The model: inside a claim, only its owners may (and nothing new is claimed);
            // elsewhere a claim is made unless the player has used up their three.
            let allowed = if model.iter().any(|c| c.contains(x, z)) {
                model.iter().filter(|c| c.contains(x, z)).all(|c| c.owner == p)
            } else if counts[p] >= 3 {
                false
            } else {
                counts[p] += 1;
                model.push(ModelClaim { owner: p, x, z });
                true
            };
            assert_eq!(got == Verdict::Allow, allowed, "step {n}: placing at ({x}, {z}) by P{p}: {got:?}");
        }
        // The copies into neighbouring cells come a tick later, from the regions owning them.
        rt.begin_tick_in(&world);
        rt.begin_tick_in(&world);
        rt.take_effects();
        for &(p, x, z) in &s.breaks {
            let r = rt.region_mut(0, s.layout.region_of_block(x, z)).unwrap();
            let got = r.block_break(&actor(p), [x, 64, z], 1);
            let allowed = model.iter().filter(|c| c.contains(x, z)).all(|c| c.owner == p);
            assert_eq!(got == Verdict::Allow, allowed, "step {n}: breaking at ({x}, {z}) by P{p} under {} claims: {got:?}", model.len());
        }
        rt.take_effects();
    }
    // Every claim is recorded in each cell it touches, once.
    for c in &model {
        for cx in ((c.x - 8) >> 7)..=((c.x + 8) >> 7) {
            for cz in ((c.z - 8) >> 7)..=((c.z + 8) >> 7) {
                let data = rt.cell_value(CellKey { dim: 0, x: cx, z: cz }, "claims", "claims").unwrap_or_default();
                let copies = data.chunks_exact(32).filter(|r| r[..16] == uuid(c.owner).to_le_bytes() && r[16..20] == (c.x - 8).to_le_bytes() && r[24..28] == (c.z - 8).to_le_bytes()).count();
                assert_eq!(copies, 1, "claim at ({}, {}) in cell ({cx}, {cz})", c.x, c.z);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn claims_do_not_depend_on_the_region_layout(steps in prop::collection::vec(claim_step(), 1..10)) {
        run_claims(steps);
    }
}

#[derive(Clone, Debug)]
struct ShopStep {
    layout: Layout,
    /// (player, goods slot, at which block column the player stands)
    clicks: Vec<(usize, usize, i32, i32)>,
}

const SLOTS: [(i32, i64); 5] = [(10, 8), (12, 40), (14, 100), (16, 25), (22, 60)];

fn shop_step() -> impl Strategy<Value = ShopStep> {
    let coord = -300..300i32;
    let click = (0..PLAYERS, 0..SLOTS.len(), coord.clone(), coord);
    (1..5u64, 0..40i32, prop::collection::vec(click, 0..12)).prop_map(|(k, seed, clicks)| ShopStep { layout: Layout { k, seed }, clicks })
}

fn run_shop(steps: Vec<ShopStep>) {
    let mut rt = runtime(&["shop"]);
    let names = names();
    let actor = |i: usize| Actor::new(uuid(i), &names[i], false);
    for i in 0..PLAYERS {
        rt.player_joined(&actor(i));
    }
    let mut spent = [0i64; PLAYERS];
    let mut positions = vec![(0i32, 0i32); PLAYERS];
    for (n, s) in steps.iter().enumerate() {
        rt.sync_regions(0, 1..=s.layout.k);
        for &(p, slot, x, z) in &s.clicks {
            positions[p] = (x, z);
            let r = rt.region_mut(0, s.layout.region_of_block(x, z)).unwrap();
            let click = ContainerClick {
                menu: Some("shop:main"),
                container: "minecraft:generic_9x3",
                slot: SLOTS[slot].0,
                button: 0,
                kind: ClickKind::Left,
                clicked: None,
            };
            r.container_click(&actor(p), &click);
        }
        let world = Cut { layout: s.layout, players: positions.iter().enumerate().map(|(i, &(x, z))| (uuid(i), x, z)).collect() };
        rt.begin_tick_in(&world);
        for e in rt.take_effects() {
            if let EffectKind::Give { who, item } = &e.kind {
                let p = (0..PLAYERS).find(|&i| uuid(i) == *who).expect("a known player");
                let price = match (item.item.as_str(), item.tag.as_deref()) {
                    ("minecraft:bread", _) => 8,
                    ("minecraft:iron_sword", _) => 40,
                    ("minecraft:diamond", _) => 100,
                    ("minecraft:stick", Some("shop:wand")) => 25,
                    ("minecraft:golden_apple", _) => 60,
                    other => panic!("an unknown good {other:?}"),
                };
                spent[p] += price;
            }
        }
        for p in 0..PLAYERS {
            let bal = match rt.global_value("shop", &format!("bal:{}", uuid_str(p))) {
                Some(GlobalValue::Int(v)) => v,
                other => panic!("step {n}: P{p} has no balance ({other:?})"),
            };
            assert!(bal >= 0, "step {n}: P{p} overdrew to {bal}");
            assert_eq!(100 - bal, spent[p], "step {n}: P{p}: starting money - balance != what they were given");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn purchases_conserve_money_across_region_layouts(steps in prop::collection::vec(shop_step(), 1..10)) {
        run_shop(steps);
    }
}
