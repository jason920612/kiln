//! The 1.0 plugin API in strict mode (design §11.5): a scripted game of six players in three
//! groups, with the shop, the claims, the homes, the HUD, the arena, an NPC and the
//! gatekeeper all running, must come out the same whatever the regions and the workers: one
//! region per level on one worker, or a region per group on four workers under chaos
//! scheduling, with a fuel budget tight enough that some calls run out. The state hash of every
//! tick, every player's packet stream (digest) and the plugins' own state must agree.

use bytes::BytesMut;
use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

const PLAYERS: usize = 6;
const GROUP_SPACING: f64 = 1600.0;

struct Outcome {
    hashes: Vec<u64>,
    digests: Vec<u64>,
    stats: (u64, u64, u64),
    values: Vec<Option<Vec<u8>>>,
    results: u64,
    tasks_run: u64,
}

fn item(name: &str) -> i32 {
    kiln_data::builtin_id("minecraft:item", name).unwrap()
}

fn run(workers: usize, unified: bool, chaos: Option<u64>, fuel: u64) -> Outcome {
    kiln_sim::testing::hash_packets();
    let ids = ["arena", "chat-format", "claims", "gatekeeper", "homes", "npc", "scoreboard-hud", "shop"];
    let dir = kiln_plugin_host::examples::custom_dir("api-determinism", &ids, &[("homes", "warmup = 12")]).expect("example plugins");
    let mut config = SimConfig::new(PLAYERS, 3, None);
    config.keep_alive = false;
    config.pool.workers = workers;
    config.pool.chaos = chaos;
    config.unified_regions = unified;
    config.plugins = Some(kiln_sim::PluginSettings::strict(dir, fuel));
    let mut sim = Sim::new(config);
    let mut clients: Vec<Client> = Vec::new();
    let mut inbox = Vec::new();
    for i in 0..PLAYERS {
        let (msg, stats) = join(i as u64 + 1, &format!("P{i}"), 2);
        inbox.push(msg);
        clients.push(Client::new(i as u64 + 1, stats));
    }
    inbox.push(ToSim::Console("gamerule minecraft:spawn_mobs false".into()));
    for i in 0..PLAYERS {
        inbox.push(ToSim::Console(format!("gamemode creative P{i}")));
        inbox.push(ToSim::Console(format!("tp P{i} {} -60 {}", (i / 2) as f64 * GROUP_SPACING + (i % 2) as f64 * 3.0, 40.0)));
    }
    inbox.push(ToSim::Console("op P0".into()));
    assert!(sim.step(inbox.drain(..)));
    let mut hashes = Vec::new();
    for tick in 0..260usize {
        for c in &mut clients {
            c.tick(None, &mut inbox);
        }
        let conn = |i: usize| i as u64 + 1;
        let pkt = |inbox: &mut Vec<ToSim>, i: usize, p: PlayIn| inbox.push(ToSim::Packet(conn(i), p));
        let cmd = |inbox: &mut Vec<ToSim>, i: usize, c: &str| inbox.push(ToSim::Packet(conn(i), PlayIn::ChatCommand { command: c.into() }));
        match tick {
            // Gold blocks for the claim makers, then everyone survives.
            15 => {
                for i in [0, 2, 4] {
                    let gold = ItemStack { item: item("minecraft:gold_block"), count: 5, added: Vec::new(), removed: Vec::new() };
                    pkt(&mut inbox, i, PlayIn::SetCreativeSlot { slot: 36, item: Some(gold) });
                }
            }
            18 => {
                for i in 0..PLAYERS {
                    inbox.push(ToSim::Console(format!("gamemode survival P{i}")));
                }
            }
            // The shop: two players open it and buy.
            22 => {
                cmd(&mut inbox, 0, "shop");
                cmd(&mut inbox, 3, "shop");
                cmd(&mut inbox, 1, "sethome base");
            }
            26 | 27 | 28 => {
                let slot = [12, 14, 16][tick - 26];
                for i in [0, 3] {
                    let c = ContainerClick { container_id: 1, state_id: 0, slot, button: 0, input: ContainerInput::Pickup, changed: Vec::new(), carried: HashedStack::Empty };
                    let mut body = BytesMut::new();
                    c.write(&mut body);
                    pkt(&mut inbox, i, PlayIn::ContainerClick { body: body.freeze() });
                }
            }
            // Claims: each maker puts a gold block down beside themselves.
            34 => {
                for i in [0, 2, 4] {
                    let p = clients[i].pos;
                    let at = [p[0].floor() as i32 + 1, p[1].floor() as i32 - 1, p[2].floor() as i32];
                    pkt(&mut inbox, i, PlayIn::UseItemOn { hand: 0, pos: at, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 });
                }
            }
            // Neighbours try to dig and to fight on the claims.
            40 | 44 | 48 => {
                for (thief, owner) in [(1usize, 0usize), (3, 2), (5, 4)] {
                    let p = clients[thief].pos;
                    pkt(&mut inbox, thief, PlayIn::PlayerAction { action: 0, pos: [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32], face: 1, sequence: tick as i32 });
                    if let Some(target) = sim.entity_id(conn(owner)) {
                        pkt(&mut inbox, thief, PlayIn::Attack { entity_id: target });
                    }
                }
            }
            // Homes, the arena, an NPC.
            60 => {
                cmd(&mut inbox, 1, "home base");
                cmd(&mut inbox, 5, "arena join");
                cmd(&mut inbox, 4, "npc spawn Guide");
            }
            70 => cmd(&mut inbox, 5, "arena reset"),
            90 => cmd(&mut inbox, 5, "arena out"),
            95 => pkt(&mut inbox, 5, PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn)),
            100 | 140 | 180 => {
                for i in 0..PLAYERS {
                    pkt(&mut inbox, i, PlayIn::Chat { message: format!("hello {tick} from P{i}") });
                }
            }
            120 => cmd(&mut inbox, 0, "balance"),
            _ => {}
        }
        assert!(sim.step(inbox.drain(..)), "simulation stopped");
        hashes.push(sim.state_hash());
    }
    assert_eq!(sim.player_count(), PLAYERS);
    let digests = clients.iter().map(|c| *c.stats.digest.lock().unwrap()).collect();
    let values = (0..PLAYERS)
        .flat_map(|i| {
            let u = uuid::Uuid::from_u64_pair(0x6b69_6c6e, i as u64 + 1);
            [sim.plugin_player_value(u, "scoreboard-hud", "deaths"), sim.plugin_player_value(u, "claims", "claims"), sim.plugin_player_value(u, "shop", "welcomed")]
        })
        .collect();
    Outcome { hashes, digests, stats: sim.plugin_stats(), values, results: sim.plugin_stat("results"), tasks_run: sim.plugin_stat("tasks-run") }
}

#[test]
fn the_api_replays_exactly_on_any_layout_and_worker_count() {
    for fuel in [2_000_000u64, 20_000] {
        let reference = run(1, true, None, fuel);
        let split = run(4, false, Some(7), fuel);
        assert_eq!(split.hashes, reference.hashes, "state hashes, fuel {fuel}");
        assert_eq!(split.digests, reference.digests, "packet streams, fuel {fuel}");
        assert_eq!(split.values, reference.values, "plugin state, fuel {fuel}");
        assert_eq!(split.stats, reference.stats, "calls, traps and timeouts, fuel {fuel}");
        assert_eq!((split.results, split.tasks_run), (reference.results, reference.tasks_run));
        let (calls, traps, timeouts) = split.stats;
        eprintln!("fuel {fuel}: calls {calls}, traps {traps}, timeouts {timeouts}, results {}, tasks {}", split.results, split.tasks_run);
        assert!(calls > 200 && traps == 0, "{:?}", split.stats);
        assert!(split.results > 0 && split.tasks_run > 0, "purchases answered and the HUD ran");
        if fuel == 20_000 {
            assert!(timeouts > 0, "a tight budget: some calls run out of fuel");
        }
    }
}
