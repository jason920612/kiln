//! Block commands edit in one batch (wp70): the clients get a command's changes when it is done,
//! a Section Blocks Update per section (vanilla's `ChunkHolder.broadcastChanges`), and the light
//! of the changed blocks is worked out once at the end; what the blocks end up as (states, block
//! entities, light, neighbour reactions of each mode) stays what one block at a time gives.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, SinkStats, join};
use kiln_sim::{Sim, SimConfig};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

fn settle(sim: &mut Sim, client: &mut Client, ticks: usize) {
    for _ in 0..ticks {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
}

/// A player standing in a flat world, the packets it gets logged; the block it stands on.
fn joined() -> (Sim, Client, Arc<SinkStats>, [i32; 3]) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Builder", 3);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let setup = ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:random_tick_speed 0", "gamemode creative Builder"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats.clone());
    settle(&mut sim, &mut client, 5);
    let p = client.pos;
    (sim, client, stats, [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32])
}

fn run(sim: &mut Sim, client: &mut Client, commands: &[String]) {
    assert!(sim.step(commands.iter().map(|c| ToSim::Console(c.clone()))));
    settle(sim, client, 1);
}

/// `x`, `y`, `z` from a packed block position.
fn unpack(v: i64) -> [i32; 3] {
    [(v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32]
}

/// The block changes in the logged packets: (single updates, section updates by section), and
/// the states they leave on the client.
fn block_packets(stats: &SinkStats) -> (Vec<[i32; 3]>, Vec<[i32; 3]>, HashMap<[i32; 3], u16>) {
    let packets = stats.log.lock().unwrap().replace(Vec::new()).unwrap();
    let (mut singles, mut sections, mut seen) = (Vec::new(), Vec::new(), HashMap::new());
    for pkt in &packets {
        let mut r = kiln_proto::codec::Reader::new(pkt);
        match r.varint().unwrap() {
            id if id == kiln_data::packets::play::clientbound::BLOCK_UPDATE => {
                let pos = unpack(r.i64().unwrap());
                seen.insert(pos, r.varint().unwrap() as u16);
                singles.push(pos);
            }
            id if id == kiln_data::packets::play::clientbound::SECTION_BLOCKS_UPDATE => {
                let s = r.i64().unwrap();
                let section = [(s >> 42) as i32, (s << 44 >> 44) as i32, (s << 22 >> 42) as i32];
                sections.push(section);
                for _ in 0..r.varint().unwrap() {
                    let v = r.varlong().unwrap();
                    let (lx, lz, ly) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
                    let pos = [section[0] * 16 + lx as i32, section[1] * 16 + ly as i32, section[2] * 16 + lz as i32];
                    seen.insert(pos, (v >> 12) as u16);
                }
            }
            _ => {}
        }
    }
    (singles, sections, seen)
}

fn cube(min: [i32; 3], max: [i32; 3]) -> impl Iterator<Item = [i32; 3]> {
    (min[2]..=max[2]).flat_map(move |z| (min[1]..=max[1]).flat_map(move |y| (min[0]..=max[0]).map(move |x| [x, y, z])))
}

fn state(name: &str) -> u16 {
    kiln_data::blocks_types::block_by_name(name).unwrap().default
}

#[test]
fn a_fill_reaches_the_client_once_per_section() {
    let (mut sim, mut client, stats, [x, y, z]) = joined();
    let (min, max) = ([x - 10, y + 3, z - 9], [x + 9, y + 22, z + 8]);
    block_packets(&stats);
    run(&mut sim, &mut client, &[format!("fill {} {} {} {} {} {} stone", min[0], min[1], min[2], max[0], max[1], max[2])]);
    let (singles, sections, seen) = block_packets(&stats);
    for p in cube(min, max) {
        assert_eq!(seen.get(&p).copied(), Some(state("minecraft:stone")), "the client's block at {p:?}");
        assert!(!singles.contains(&p), "no single update for {p:?}");
    }
    let touched: BTreeSet<[i32; 3]> = cube(min, max).map(|p| [p[0] >> 4, p[1] >> 4, p[2] >> 4]).collect();
    let mut sent: BTreeMap<[i32; 3], usize> = BTreeMap::new();
    for s in sections {
        *sent.entry(s).or_default() += 1;
    }
    for s in &touched {
        assert_eq!(sent.get(s), Some(&1), "section {s:?} once");
    }
}

#[test]
fn the_light_of_a_fill_is_the_light_of_its_blocks_set_one_by_one() {
    // The same blocks: a hollow stone box with a glowstone strip in a wall and a glass roof, made
    // by three fills (each lit at its end, many blocks together) and block by block (one
    // setblock command each, lit alone).
    let (mut together, mut client_a, _, [x, y, z]) = joined();
    let (mut one, mut client_b, _, at) = joined();
    assert_eq!([x, y, z], at);
    let (min, max) = ([x - 12, y + 1, z - 7], [x + 5, y + 9, z + 10]);
    let strip = |p: [i32; 3]| p[0] == min[0] && p[2] == min[2] + 3 && (min[1] + 2..=max[1] - 2).contains(&p[1]);
    let blocks = |p: [i32; 3]| match () {
        _ if p[1] == max[1] => Some("glass"),
        _ if strip(p) => Some("glowstone"),
        _ if (0..3).any(|i| p[i] == min[i] || p[i] == max[i]) => Some("stone"),
        _ => None,
    };
    let fills = [
        format!("fill {} {} {} {} {} {} stone hollow", min[0], min[1], min[2], max[0], max[1], max[2]),
        format!("fill {} {} {} {} {} {} glowstone", min[0], min[1] + 2, min[2] + 3, min[0], max[1] - 2, min[2] + 3),
        format!("fill {} {} {} {} {} {} glass", min[0], max[1], min[2], max[0], max[1], max[2]),
    ];
    for c in &fills {
        run(&mut together, &mut client_a, std::slice::from_ref(c));
    }
    let singles: Vec<String> = cube(min, max).filter_map(|p| blocks(p).map(|b| format!("setblock {} {} {} {b}", p[0], p[1], p[2]))).collect();
    for c in singles.chunks(64) {
        run(&mut one, &mut client_b, c);
    }
    for p in cube([min[0] - 16, min[1] - 2, min[2] - 16], [max[0] + 16, max[1] + 16, max[2] + 16]) {
        assert_eq!(together.block_at(p[0], p[1], p[2]), one.block_at(p[0], p[1], p[2]), "block at {p:?}");
        assert_eq!(together.light_at(p[0], p[1], p[2]), one.light_at(p[0], p[1], p[2]), "light at {p:?}");
    }
    // Light is there when the command is done: beside the strip inside the box (under glass,
    // which lets the sky through).
    assert_eq!(together.light_at(min[0] + 1, min[1] + 3, min[2] + 3), Some((14, 15)), "inside the box beside the strip");
}

#[test]
fn modes_place_what_vanilla_places() {
    let (mut sim, mut client, _, [x, y, z]) = joined();
    sim.capture_console();
    let b = |sim: &Sim, p: [i32; 3]| sim.block_at(p[0], p[1], p[2]).unwrap();
    let (stone, glass, air, sand) = (state("minecraft:stone"), state("minecraft:glass"), state("minecraft:air"), state("minecraft:sand"));
    let (lo, hi) = ([x + 2, y + 3, z + 2], [x + 6, y + 7, z + 6]);
    let area = format!("{} {} {} {} {} {}", lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]);
    run(&mut sim, &mut client, &[format!("fill {area} stone hollow")]);
    assert_eq!((b(&sim, lo), b(&sim, [x + 4, y + 5, z + 4])), (stone, air), "hollow: a shell");
    run(&mut sim, &mut client, &[format!("fill {area} glass outline")]);
    assert_eq!((b(&sim, lo), b(&sim, [x + 4, y + 5, z + 4])), (glass, air), "outline leaves the inside");
    run(&mut sim, &mut client, &[format!("fill {area} stone keep")]);
    assert_eq!((b(&sim, lo), b(&sim, [x + 4, y + 5, z + 4])), (glass, stone), "keep fills only air");
    run(&mut sim, &mut client, &[format!("fill {area} dirt replace glass")]);
    assert_eq!((b(&sim, lo), b(&sim, [x + 4, y + 5, z + 4])), (state("minecraft:dirt"), stone), "the filter");
    run(&mut sim, &mut client, &[format!("fill {area} air destroy")]);
    assert_eq!(b(&sim, lo), air);
    let lines = sim.take_console();
    let counts: Vec<&String> = lines.iter().filter(|l| l.contains("commands.fill.success") || l.contains("Successfully filled")).collect();
    assert_eq!(counts.len(), 5, "{lines:?}");
    // (Vanilla counts the blocks that changed: the hollow box's inside was air already.)
    let n: Vec<&str> = counts.iter().map(|l| l.trim_end_matches(']').rsplit('[').next().unwrap()).collect();
    assert_eq!(n, ["98", "98", "27", "98", "125"], "{counts:?}");
    // strict: no neighbour reactions and no onPlace, so sand stays up; without it, it falls.
    let s1 = [x + 2, y + 12, z + 2];
    let s2 = [x + 6, y + 12, z + 2];
    run(&mut sim, &mut client, &[format!("fill {} {} {} {} {} {} sand strict", s1[0], s1[1], s1[2], s1[0], s1[1], s1[2])]);
    run(&mut sim, &mut client, &[format!("fill {} {} {} {} {} {} sand", s2[0], s2[1], s2[2], s2[0], s2[1], s2[2])]);
    settle(&mut sim, &mut client, 40);
    assert_eq!(b(&sim, s1), sand, "strict sand stays");
    assert_eq!(b(&sim, s2), air, "sand falls");
}

#[test]
fn clone_copies_block_entities_and_move_leaves_air() {
    let (mut sim, mut client, _, [x, y, z]) = joined();
    let chest = [x + 3, y + 2, z + 3];
    run(
        &mut sim,
        &mut client,
        &[
            format!("fill {} {} {} {} {} {} stone", x + 2, y + 1, z + 2, x + 5, y + 1, z + 5),
            format!("setblock {} {} {} chest{{Items:[{{Slot:3b,id:\"minecraft:diamond\",count:5}}]}}", chest[0], chest[1], chest[2]),
        ],
    );
    let src = format!("{} {} {} {} {} {}", x + 2, y + 1, z + 2, x + 5, y + 3, z + 5);
    run(&mut sim, &mut client, &[format!("clone {src} {} {} {}", x + 10, y + 1, z + 2)]);
    let copy = [chest[0] + 8, chest[1], chest[2]];
    let items = |sim: &mut Sim, p: [i32; 3]| sim.block_entity_data("minecraft:overworld", p[0], p[1], p[2]).and_then(|t| t.get("Items").cloned());
    let original = items(&mut sim, chest).expect("the chest's items");
    assert_eq!(items(&mut sim, copy), Some(original.clone()), "the copy has the items");
    run(&mut sim, &mut client, &[format!("clone {src} {} {} {} replace move", x + 2, y + 6, z + 2)]);
    assert_eq!(sim.block_at(chest[0], chest[1], chest[2]), Some(state("minecraft:air")), "moved away");
    assert_eq!(items(&mut sim, [chest[0], chest[1] + 5, chest[2]]), Some(original), "moved with its items");
}

#[test]
fn many_setblocks_of_one_command_go_out_by_section() {
    // `execute` runs a setblock per player position: one command, one batch.
    let (mut sim, mut client, stats, [x, y, z]) = joined();
    block_packets(&stats);
    let cmds: Vec<String> = (0..8).map(|i| format!("execute positioned {} {} {} run setblock ~ ~ ~ glowstone", x + i, y + 4, z)).collect();
    run(&mut sim, &mut client, &cmds);
    let (singles, _, seen) = block_packets(&stats);
    for i in 0..8 {
        assert_eq!(seen.get(&[x + i, y + 4, z]).copied(), Some(state("minecraft:glowstone")));
    }
    // (Separate console lines: a single update each.)
    assert_eq!(singles.len(), 8, "{singles:?}");
    assert_eq!(sim.light_at(x, y + 4, z).map(|l| l.0), Some(15));
    assert_eq!(sim.light_at(x, y + 5, z).map(|l| l.0), Some(14));
}
