//! A crowd's packets are a function of its inputs: what the optimisations of the serial phases
//! (the locator bar, tracking) leave out must not change a byte a player receives. Each
//! scenario plays scripted walkers that crouch, go spectator, leave and rejoin, and compares
//! the state hash and one hash of every player's packet stream (keep-alives left out) with
//! the values the straightforward implementation produced (constants below, recorded before
//! the locator bar was reorganised; `cargo run --release -p kiln-sim --example sim_load`
//! with `--churn` and `KILN_SINK_DIGEST=1` prints the same numbers for the same arguments).
//! The last scenario's constants come from the same implementation with its two `HashMap`
//! iteration orders (the untracks a player changing dimension gets, and the untracks when the
//! game rule turns off) put in connection order, which made its streams reproducible.

use kiln_sim::testing::{Churn, Client, Walker, group_offset, join};
use kiln_sim::{Sim, SimConfig};

const SURFACE_Y: f64 = -60.0;

struct Scenario {
    players: usize,
    groups: usize,
    spacing: f64,
    walk: bool,
    ticks: usize,
    /// Console commands for measured tick `k`, after the churn's.
    events: fn(usize, &mut Vec<kiln_link::ToSim>),
}

fn no_events(_: usize, _: &mut Vec<kiln_link::ToSim>) {}

/// Same script as `sim_load --churn` (5 joins per tick, then `ticks` measured ticks).
fn run(s: &Scenario) -> (u64, u64, usize) {
    kiln_sim::testing::hash_packets();
    kiln_sim::testing::verify_locator_bar();
    let mut config = SimConfig::new(s.players, 10, None);
    config.keep_alive = false;
    config.pool.workers = 3;
    let mut sim = Sim::new(config);
    let mut walkers: Vec<Walker> = Vec::new();
    let mut inbox = Vec::new();
    let mut churn = Churn::new(s.players);
    let mut tick = 0usize;
    let mut since = None;
    loop {
        for _ in 0..5 {
            let i = walkers.len();
            if i == s.players {
                break;
            }
            let name = format!("W{i}");
            let (msg, stats) = join(i as u64 + 1, &name, 2);
            inbox.push(msg);
            let [ox, oz] = group_offset(i % s.groups, s.groups, s.spacing);
            let center = [8.5 + ox, 8.5 + oz];
            inbox.push(kiln_link::ToSim::Console(format!("tp {name} {} {SURFACE_Y} {}", center[0], center[1])));
            walkers.push(Walker::new(Client::new(i as u64 + 1, stats), center, i as u64 + 1));
        }
        for w in &mut walkers {
            w.tick(6.0, s.walk, &mut inbox);
        }
        if let Some(since) = since {
            churn.tick(tick - since, &mut walkers, &mut inbox, s.groups, s.spacing, 2, SURFACE_Y);
            (s.events)(tick - since, &mut inbox);
        }
        assert!(sim.step(inbox.drain(..)));
        tick += 1;
        match since {
            None if walkers.len() == s.players && walkers.iter().all(|w| w.client.settled()) => since = Some(tick),
            Some(t) if tick - t >= s.ticks => break,
            None => assert!(tick < 20 * 600, "players did not settle"),
            _ => {}
        }
    }
    (sim.state_hash(), churn.stream_digest(&walkers), sim.region_count())
}

fn check(s: Scenario, hash: u64, digest: u64, regions: usize) {
    // The constants are for the built-in data. A datapack (`KILN_DATAPACK`) brings natural
    // spawning and advancements, whose Update Advancements packets carry the wall-clock time a
    // criterion was obtained (vanilla's `Instant.now()`): no stream repeats from run to run.
    if std::env::var_os("KILN_DATAPACK").is_some() {
        eprintln!("crowd_golden: skipped with KILN_DATAPACK (advancement dates follow the clock)");
        return;
    }
    let got = run(&s);
    assert_eq!(got.2, regions, "regions");
    assert_eq!(
        (format!("{:016x}", got.0), format!("{:016x}", got.1)),
        (format!("{hash:016x}"), format!("{digest:016x}")),
        "state hash and packet stream digest"
    );
}

/// One region, everyone close: block links.
#[test]
fn crowd_in_one_region() {
    check(Scenario { players: 150, groups: 4, spacing: 48.0, walk: false, ticks: 150, events: no_events }, 0xe94274c461b259ab, 0x92401cec36661511, 1);
}

/// Groups far enough apart for chunk links, walking.
#[test]
fn groups_with_chunk_links() {
    check(Scenario { players: 80, groups: 4, spacing: 200.0, walk: true, ticks: 150, events: no_events }, 0x9d96bd81102791fd, 0x2e2f7e87a454c5de, 1);
}

/// Groups past the locator bar's 332 blocks: azimuth links, one region each.
#[test]
fn groups_with_azimuth_links() {
    check(Scenario { players: 60, groups: 3, spacing: 1500.0, walk: true, ticks: 150, events: no_events }, 0x16e1ef1b0c272be9, 0x537a8b0853c53dac, 3);
}

/// The locator bar's other paths: team colors (connections keep the color they were made
/// with), `/waypoint modify`, the `locator_bar` game rule off and on, and players changing
/// dimension and coming back.
fn locator_events(k: usize, inbox: &mut Vec<kiln_link::ToSim>) {
    let cmds: &[&str] = match k {
        5 => &["team add red", "team modify red color red", "team join red W1", "team join red W2", "team join red W3"],
        10 => &["waypoint modify W4 color blue"],
        12 => &["waypoint modify W5 style set minecraft:bowtie", "waypoint modify W1 color hex 00ff7f"],
        36 => &["team modify red color green"],
        30 => &["gamerule locator_bar false"],
        33 => &["gamerule locator_bar true"],
        40 => &["execute in minecraft:the_nether run tp W6 0 100 0", "execute in minecraft:the_nether run tp W7 30 100 0"],
        45 => &["waypoint modify W4 color reset", "waypoint modify W5 style reset", "team leave W2"],
        70 => &["execute in minecraft:overworld run tp W6 8.5 -60 8.5"],
        _ => &[],
    };
    inbox.extend(cmds.iter().map(|c| kiln_link::ToSim::Console(c.to_string())));
}

/// Two groups with chunk links, and the events above.
///
/// The packet digest is the first recording; the state hash was re-recorded on Linux after the
/// walkers (`testing::Walker`) stopped using the platform's `hypot`/`cos`/`sin`: the first
/// recording came from a platform whose `hypot` rounded a few of this scenario's long nether
/// walks one ulp differently (players' positions are hashed bit for bit, the packets quantize
/// them). The other three scenarios kept their recorded constants.
#[test]
fn locator_bar_commands_and_dimensions() {
    check(Scenario { players: 40, groups: 2, spacing: 120.0, walk: true, ticks: 120, events: locator_events }, 0xff6bf67d90731153, 0xc1f7b7558f5a5e4f, 2);
}
