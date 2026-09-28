//! The simulation is a function of its inputs: the same scripted players, block edits and
//! console commands give the same state hashes, tick for tick, however the world is split
//! into regions and however many workers tick them in whatever order (design DT-R1). Block
//! behaviour takes part: fences reshape their neighbours, water spreads through scheduled
//! ticks, and random ticks run in every chunk near a player. Players of a group hit each
//! other: damage, hurt cooldowns and knockback are part of the state; so are mob effects
//! (poison, regeneration, speed) and burning in the fire lit in each group. Every group gets
//! the eight first mob types Kiln simulated (they wander, path, chase and hit the survival
//! players, and the skeletons shoot) and a row of every other mob type.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};

const PLAYERS: usize = 24;
const GROUPS: usize = 3;
/// Far enough apart that each group gets its own region.
const GROUP_SPACING: f64 = 1024.0;
const SURFACE_Y: f64 = -60.0;

struct Run {
    hashes: Vec<u64>,
    /// Packets and bytes each player received, at every hash point: what the players see
    /// must not depend on the topology either.
    traffic: Vec<Vec<(u64, u64)>>,
    max_regions: usize,
    /// With plugins: blocks the counter plugin counted, and edits of group 1 (inside the
    /// protected area) that left a block.
    counted: i64,
    protected_built: usize,
    /// Plugin calls, traps, timeouts.
    plugin_stats: (u64, u64, u64),
}

fn run(ticks: usize, workers: usize, unified: bool, chaos: Option<u64>) -> Run {
    run_with(ticks, workers, unified, chaos, None)
}

fn run_with(ticks: usize, workers: usize, unified: bool, chaos: Option<u64>, plugins: Option<kiln_sim::PluginSettings>) -> Run {
    let with_plugins = plugins.is_some();
    let mut config = SimConfig::new(PLAYERS, 4, None);
    config.plugins = plugins;
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    config.unified_regions = unified;
    let mut sim = Sim::new(config);
    let items = ["minecraft:stone", "minecraft:oak_fence", "minecraft:redstone_torch"].map(|n| kiln_data::builtin_id("minecraft:item", n).unwrap());
    let mut walkers = Vec::new();
    let mut inbox = Vec::new();
    let mut hashes = Vec::new();
    let mut placed = Vec::new();
    let mut traffic = Vec::new();
    let mut max_regions = 0;
    let mut hits = 0;
    let (mut effects, mut burning) = (0, 0);
    let mut mob_ticks = 0;
    // Players in odd rows of the groups can be hurt; the ones in even rows hit them.
    let victim = |i: usize| (i / GROUPS) % 2 == 1;
    for tick in 0..ticks {
        if walkers.len() < PLAYERS {
            let i = walkers.len();
            let conn = i as u64 + 1;
            let name = format!("P{i}");
            let (msg, stats) = join(conn, &name, 2);
            let [ox, oz] = group_offset(i % GROUPS, GROUPS, GROUP_SPACING);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(msg);
            inbox.push(ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
            let item = ItemStack { item: items[i % items.len()], count: 64, added: Vec::new(), removed: Vec::new() };
            inbox.push(ToSim::Packet(conn, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }));
            walkers.push(Walker::new(Client::new(conn, stats), center, conn));
        }
        for (i, w) in walkers.iter_mut().enumerate() {
            w.tick(5.0, i % 2 == 0, &mut inbox);
            // Every so often a player builds next to itself, and later breaks what it built.
            if w.client.settled() && (tick + i) % 40 == 0 {
                let [x, y, z] = w.client.pos.map(|c| c.floor() as i32);
                let (conn, sequence) = (w.client.conn, tick as i32);
                let ground = [x + 2, y - 1, z];
                placed.push(([x + 2, y, z], i % GROUPS));
                inbox.push(ToSim::Packet(conn, if (tick / 40) % 3 == 2 {
                    PlayIn::PlayerAction { action: 0, pos: [x + 2, y, z], face: 1, sequence }
                } else {
                    PlayIn::UseItemOn { hand: 0, pos: ground, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence }
                }));
            }
        }
        // With plugins, everyone chats (PX, the formatter rewrites it) mid-way.
        if with_plugins && tick == 160 {
            for i in 0..PLAYERS {
                inbox.push(ToSim::Packet(i as u64 + 1, PlayIn::Chat { message: format!("hello from P{i}") }));
            }
        }
        if tick == 150 {
            inbox.push(ToSim::Console("time set 15000".into()));
        }
        if tick == 120 {
            for i in (0..PLAYERS).filter(|&i| victim(i)) {
                inbox.push(ToSim::Console(format!("gamemode survival P{i}")));
            }
        }
        // Every so often each attacker hits the player one row further in its group (the
        // swing follows, as from the real client), sometimes sprinting.
        if tick > 130 && tick % 15 == 0 {
            for i in (0..PLAYERS).filter(|&i| !victim(i) && i + GROUPS < PLAYERS) {
                let (conn, target) = (i as u64 + 1, sim.entity_id((i + GROUPS) as u64 + 1).unwrap());
                if (tick / 15) % 3 == 0 {
                    inbox.push(ToSim::Packet(conn, PlayIn::PlayerCommand { action: 1 }));
                }
                inbox.push(ToSim::Packet(conn, PlayIn::Attack { entity_id: target }));
                inbox.push(ToSim::Packet(conn, PlayIn::Punch));
            }
        }
        // Effects for everyone, poison and regeneration for the victims, then a fire in the
        // middle of each group that the walkers pass through.
        if tick == 125 {
            inbox.push(ToSim::Console("effect give @a minecraft:speed 30 1".into()));
            for i in (0..PLAYERS).filter(|&i| victim(i)) {
                inbox.push(ToSim::Console(format!("effect give P{i} minecraft:poison 6 0")));
                inbox.push(ToSim::Console(format!("effect give P{i} minecraft:regeneration 12 1")));
            }
        }
        if tick == 140 {
            for g in 0..GROUPS {
                let [ox, oz] = group_offset(g, GROUPS, GROUP_SPACING);
                for (dx, dz) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)] {
                    let (x, z) = (8.0 + ox + dx, 8.0 + oz + dz);
                    inbox.push(ToSim::Console(format!("setblock {x} {SURFACE_Y} {z} minecraft:fire")));
                }
            }
        }
        // The mobs of each group, at night so the undead do not burn.
        if tick == 100 {
            inbox.push(ToSim::Console("time set 14000".into()));
            for g in 0..GROUPS {
                let [ox, oz] = group_offset(g, GROUPS, GROUP_SPACING);
                let mobs = ["zombie", "skeleton", "creeper", "spider", "pig", "cow", "sheep", "chicken"];
                for (k, m) in mobs.iter().enumerate() {
                    let (x, z) = (8.5 + ox + 6.0 * (k as f64 - 3.5), 8.5 + oz + 7.0);
                    inbox.push(ToSim::Console(format!("summon minecraft:{m} {x} {SURFACE_Y} {z}")));
                }
                // Every other mob type in a second row behind them.
                for (k, kind) in kiln_entity::mob::ALL_KINDS[8..].iter().enumerate() {
                    let (x, z) = (8.5 + ox + 2.0 * (k as f64 - 12.0), 8.5 + oz - 9.0);
                    inbox.push(ToSim::Console(format!("summon {} {x} {SURFACE_Y} {z}", kind.type_name())));
                }
            }
        }
        // Beside each group, a chest emptying through a hopper chain into another chest, and
        // a furnace with ore and fuel: block entities tick in their regions.
        if tick == 70 {
            for g in 0..GROUPS {
                let [ox, oz] = group_offset(g, GROUPS, GROUP_SPACING);
                let (x, y, z) = ((2.0 + ox) as i32, SURFACE_Y as i32, (14.0 + oz) as i32);
                inbox.push(ToSim::Console(format!("setblock {x} {y} {z} minecraft:chest")));
                inbox.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:hopper[facing=down]", y + 1)));
                inbox.push(ToSim::Console(format!("setblock {x} {} {z} minecraft:hopper[facing=down]", y + 2)));
                inbox.push(ToSim::Console(format!(
                    "setblock {x} {} {z} minecraft:chest{{Items:[{{Slot:0b,id:\"minecraft:oak_log\",count:9}},{{Slot:4b,id:\"minecraft:dirt\",count:3}}]}}",
                    y + 3
                )));
                inbox.push(ToSim::Console(format!(
                    "setblock {} {y} {z} minecraft:furnace{{Items:[{{Slot:0b,id:\"minecraft:raw_iron\",count:2}},{{Slot:1b,id:\"minecraft:coal\",count:1}}]}}",
                    x + 2
                )));
            }
        }
        // A spring beside each group: water spreads over the next ticks.
        if tick == 60 {
            for g in 0..GROUPS {
                let [ox, oz] = group_offset(g, GROUPS, GROUP_SPACING);
                inbox.push(ToSim::Console(format!("setblock {} {} {} minecraft:water", 14.0 + ox, SURFACE_Y, 3.0 + oz)));
            }
        }
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        hits += (0..PLAYERS).filter(|&i| victim(i) && sim.health(i as u64 + 1).is_some_and(|(h, _)| h < 20.0)).count();
        effects += (0..PLAYERS).filter(|&i| sim.effects(i as u64 + 1).is_some_and(|e| !e.is_empty())).count();
        burning += (0..PLAYERS).filter(|&i| sim.fire_and_air(i as u64 + 1).is_some_and(|(f, _)| f > -20)).count();
        max_regions = max_regions.max(sim.region_count());
        mob_ticks += sim.mobs().len();
        if tick % 100 == 99 {
            hashes.push(sim.state_hash());
            let load = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
            traffic.push(walkers.iter().map(|w| (load(&w.client.stats.packets), load(&w.client.stats.bytes))).collect());
        }
    }
    assert_eq!(sim.player_count(), PLAYERS);
    let air = kiln_data::blocks::default_state::AIR;
    let built = placed.iter().filter(|(p, _)| sim.block_at(p[0], p[1], p[2]).is_some_and(|s| s != air)).count();
    // (The group's spring may have flowed there.)
    let protected_built = placed
        .iter()
        .filter(|(p, g)| *g == 1 && sim.block_at(p[0], p[1], p[2]).is_some_and(|s| s != air && !kiln_data::blocks_types::has_fluid(s)))
        .count();
    let counted = (0..PLAYERS as u64)
        .filter_map(|c| sim.plugin_player_value(uuid::Uuid::from_u64_pair(0x6b69_6c6e, c + 1), "counter", "broken"))
        .map(|v| i64::from_le_bytes(v.try_into().unwrap()))
        .sum();
    assert!(built > 10, "only {built} of {} edits left a block", placed.len());
    let [ox, oz] = group_offset(0, GROUPS, GROUP_SPACING);
    let (wx, wz) = ((14.0 + ox) as i32, (3.0 + oz) as i32);
    let flowing = (-3..=3).flat_map(|dx| (-3..=3).map(move |dz| (dx, dz))).filter(|(dx, dz)| {
        sim.block_at(wx + dx, SURFACE_Y as i32, wz + dz).is_some_and(|s| kiln_data::blocks_types::has_fluid(s))
    });
    assert!(flowing.count() > 9, "the water spread");
    let chest = [(2.0 + ox) as i32, SURFACE_Y as i32, (14.0 + oz) as i32];
    assert!(sim.container_at(chest).is_some_and(|(items, _)| !items.is_empty()), "the hoppers moved items");
    assert!(walkers.iter().all(|w| !w.client.stats.disconnected.load(std::sync::atomic::Ordering::Relaxed)));
    assert!(hits > 0, "some attacks landed");
    assert!(effects > 0, "players had effects");
    assert!(burning > 0, "someone walked into the fire");
    assert!(mob_ticks > 0, "mobs took part");
    Run { hashes, traffic, max_regions, counted, protected_built, plugin_stats: sim.plugin_stats() }
}

#[test]
fn same_inputs_give_the_same_states() {
    let a = run(400, 1, false, None);
    let b = run(400, 1, false, None);
    assert_eq!(a.hashes, b.hashes);
    assert_eq!(a.traffic, b.traffic);
    assert!(a.hashes.windows(2).all(|w| w[0] != w[1]), "the state should keep changing: {:x?}", a.hashes);
}

#[test]
fn regions_and_workers_do_not_change_the_result() {
    let unified = run(400, 1, true, None);
    assert_eq!(unified.max_regions, 1);
    let split = run(400, 1, false, None);
    assert!(split.max_regions >= GROUPS, "groups should tick as separate regions ({} regions)", split.max_regions);
    assert_eq!(split.hashes, unified.hashes, "one region vs one per group");
    assert_eq!(split.traffic, unified.traffic, "players must receive the same packets");
    for (workers, seed) in [(4, 1), (7, 99)] {
        let parallel = run(400, workers, false, Some(seed));
        assert_eq!(parallel.hashes, unified.hashes, "{workers} workers, chaos seed {seed}");
        assert_eq!(parallel.traffic, unified.traffic, "{workers} workers, chaos seed {seed}");
    }
}

/// WASM plugins loaded: spawn protection around group 1's centre (fail-closed, cell data),
/// the chat formatter (everyone chats once), the counter (observe batches, global adds) and
/// the ledger. Their calls run in the parallel regions and in PX; the result must still not
/// depend on the regions or the workers.
#[test]
fn plugins_do_not_change_the_result() {
    let dir = kiln_plugin_host::examples::custom_dir("determinism", &[], &[("spawn-protection", "center = \"520,-504\"")]).expect("example plugins");
    // Wall-clock timeouts are not deterministic (a preempted call runs out of its budget):
    // the budget here is one no call reaches.
    let settings_dir = dir.clone();
    let settings = kiln_sim::PluginSettings { call_budget: std::time::Duration::from_secs(1), ..kiln_sim::PluginSettings::new(dir) };
    let unified = run_with(400, 1, true, None, Some(settings.clone()));
    assert!(unified.counted > 0, "the counter saw breaks");
    assert_eq!(unified.protected_built, 0, "nothing was built in the protected area");
    let split = run_with(400, 4, false, Some(5), Some(settings));
    assert!(split.max_regions >= GROUPS);
    assert_eq!(split.hashes, unified.hashes);
    assert_eq!(split.traffic, unified.traffic);
    assert_eq!(split.counted, unified.counted);
    let (calls, traps, timeouts) = split.plugin_stats;
    assert!(calls > 100 && traps == 0 && timeouts == 0, "{:?}", split.plugin_stats);
    // With the default 500 µs budget under chaos scheduling, some calls may time out (and,
    // fail-closed, deny): report how many.
    let tight = run_with(400, 4, false, Some(5), Some(kiln_sim::PluginSettings::new(settings_dir)));
    eprintln!("plugin calls, traps, timeouts at the default budget: {:?}", tight.plugin_stats);
    // Plugins take part in the state: without them the hashes differ.
    assert_ne!(run(400, 1, true, None).hashes, unified.hashes);
}

/// Players spread over the three levels: some are sent to the nether and the End with
/// `/execute in`, some walk through a nether portal lit in the overworld; they walk and build
/// there. Regions of different levels tick in parallel; the result must not depend on it.
fn run_levels(ticks: usize, workers: usize, unified: bool, chaos: Option<u64>) -> (Vec<u64>, Vec<Vec<(u64, u64)>>) {
    const N: usize = 9;
    let mut config = SimConfig::new(N, 4, None);
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    config.unified_regions = unified;
    let mut sim = Sim::new(config);
    let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
    let (mut walkers, mut inbox, mut hashes, mut traffic) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for tick in 0..ticks {
        if walkers.len() < N {
            let i = walkers.len();
            let conn = i as u64 + 1;
            let name = format!("L{i}");
            let (msg, stats) = join(conn, &name, 2);
            inbox.push(msg);
            let (level, y) = match i % 3 {
                0 => ("minecraft:overworld", SURFACE_Y),
                1 => ("minecraft:the_nether", 4.0),
                _ => ("minecraft:the_end", 4.0),
            };
            let center = [8.5 + (i / 3) as f64 * 300.0, 8.5];
            inbox.push(ToSim::Console(format!("execute in {level} run tp {name} {} {y} {}", center[0], center[1])));
            let item = ItemStack { item: stone, count: 64, added: Vec::new(), removed: Vec::new() };
            inbox.push(ToSim::Packet(conn, PlayIn::SetCreativeSlot { slot: 36, item: Some(item) }));
            walkers.push(Walker::new(Client::new(conn, stats), center, conn));
        }
        // A portal frame east of the spawn, lit with fire.
        if tick == 20 {
            inbox.push(ToSim::Console("fill 30 -61 8 33 -57 8 minecraft:obsidian".into()));
            inbox.push(ToSim::Console("fill 31 -60 8 32 -58 8 minecraft:air".into()));
            inbox.push(ToSim::Console("setblock 31 -60 8 minecraft:fire".into()));
        }
        // The overworld players walk into the portal and keep walking in the nether.
        if tick == 60 {
            for i in (0..N).step_by(3) {
                inbox.push(ToSim::Console(format!("tp L{i} 32.0 -60 8.5")));
            }
        }
        for (i, w) in walkers.iter_mut().enumerate() {
            if tick >= 60 && tick < 90 && i % 3 == 0 {
                w.client.tick(None, &mut inbox);
                continue;
            }
            if tick == 90 && i % 3 == 0 {
                w.center = [w.client.pos[0], w.client.pos[2]];
            }
            w.tick(4.0, true, &mut inbox);
            if w.client.settled() && (tick + i) % 30 == 0 {
                let [x, y, z] = w.client.pos.map(|c| c.floor() as i32);
                let pkt = PlayIn::UseItemOn { hand: 0, pos: [x + 2, y - 1, z], face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: tick as i32 };
                inbox.push(ToSim::Packet(w.client.conn, pkt));
            }
        }
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        if tick % 50 == 49 {
            hashes.push(sim.state_hash());
            let load = |a: &std::sync::atomic::AtomicU64| a.load(std::sync::atomic::Ordering::Relaxed);
            traffic.push(walkers.iter().map(|w| (load(&w.client.stats.packets), load(&w.client.stats.bytes))).collect());
        }
    }
    let levels: Vec<&str> = (1..=N as u64).map(|c| sim.player_level(c).unwrap().0).collect();
    assert_eq!(levels.iter().filter(|l| **l == "minecraft:the_nether").count(), 6, "{levels:?}");
    assert_eq!(levels.iter().filter(|l| **l == "minecraft:the_end").count(), 3, "{levels:?}");
    assert!(sim.loaded_chunks().iter().all(|&n| n > 0));
    (hashes, traffic)
}

#[test]
fn levels_tick_in_parallel_with_the_same_result() {
    let reference = run_levels(200, 1, true, None);
    assert!(reference.0.windows(2).all(|w| w[0] != w[1]));
    assert_eq!(run_levels(200, 1, false, None), reference, "one region per level vs split regions");
    for (workers, seed) in [(4, 3), (7, 42)] {
        assert_eq!(run_levels(200, workers, false, Some(seed)), reference, "{workers} workers, chaos seed {seed}");
    }
}
