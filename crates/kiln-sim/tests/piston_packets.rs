//! The order a piston's packets reach the clients (wp72). The client runs the piston's move itself when the Block Event arrives
//! (`PistonBaseBlock.triggerEvent` on the client builds the moving blocks from the blocks it has). Vanilla sends the event first and the
//! changes it made (the pulled block gone, the moving piston where it goes) after it, because `ServerChunkCache.tick` flushes the chunk
//! changes before `runBlockEvents` and the changes an event makes wait for the next flush. Kiln sent all changes first: the client's own
//! move then found the blocks already moved, and a block a sticky piston pulled was drawn as air for the two ticks of the animation
//! (real clients on Kiln and vanilla, stone 2 blocks in front of a retracting sticky piston: air at the destination against
//! `moving_piston[type=normal]`).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, SinkStats, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;

fn unpack(v: i64) -> [i32; 3] {
    [(v >> 38) as i32, (v << 52 >> 52) as i32, (v << 26 >> 38) as i32]
}

#[derive(Debug, PartialEq)]
enum Seen {
    /// A Block Event: position, b0.
    Event([i32; 3], u8),
    /// A block the client is told about.
    Update([i32; 3]),
}

fn seen(stats: &SinkStats) -> Vec<Seen> {
    use kiln_data::packets::play::clientbound as ids;
    let packets = stats.log.lock().unwrap().replace(Vec::new()).unwrap();
    let mut out = Vec::new();
    for pkt in &packets {
        let mut r = kiln_proto::codec::Reader::new(pkt);
        match r.varint().unwrap() {
            id if id == ids::BLOCK_EVENT => {
                let pos = unpack(r.i64().unwrap());
                out.push(Seen::Event(pos, r.u8().unwrap()));
            }
            id if id == ids::BLOCK_UPDATE => out.push(Seen::Update(unpack(r.i64().unwrap()))),
            id if id == ids::SECTION_BLOCKS_UPDATE => {
                let s = r.i64().unwrap();
                let section = [(s >> 42) as i32, (s << 44 >> 44) as i32, (s << 22 >> 42) as i32];
                for _ in 0..r.varint().unwrap() {
                    let v = r.varlong().unwrap();
                    let (lx, lz, ly) = ((v >> 8) & 15, (v >> 4) & 15, v & 15);
                    out.push(Seen::Update([section[0] * 16 + lx as i32, section[1] * 16 + ly as i32, section[2] * 16 + lz as i32]));
                }
            }
            _ => {}
        }
    }
    out
}

fn joined() -> (Sim, Client, Arc<SinkStats>, [i32; 3]) {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Builder", 3);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let setup = ["gamerule minecraft:spawn_mobs false", "gamerule minecraft:random_tick_speed 0", "gamemode creative Builder"];
    assert!(sim.step(std::iter::once(msg).chain(setup.map(|c| ToSim::Console(c.into())))));
    let mut client = Client::new(1, stats.clone());
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let p = client.pos;
    (sim, client, stats, [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32])
}

#[test]
fn the_block_event_of_a_piston_reaches_the_client_before_the_blocks_it_moved() {
    let (mut sim, mut client, stats, [x, y, z]) = joined();
    // a sticky piston facing west with a stone two blocks in front of it, an observer that pulses next to it (the piston extends, then retracts and pulls the stone)
    let cmds = [
        format!("setblock {} {y} {z} minecraft:sticky_piston[facing=west]", x + 6),
        format!("setblock {} {y} {z} minecraft:stone", x + 4),
        format!("setblock {} {y} {} minecraft:observer[facing=south]", x + 6, z + 1),
    ];
    assert!(sim.step(cmds.map(ToSim::Console)));
    seen(&stats);
    let (stone, head) = ([x + 4, y, z], [x + 5, y, z]);
    let mut retract_checked = false;
    for _ in 0..12 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let tick = seen(&stats);
        let Some(event) = tick.iter().position(|s| matches!(s, Seen::Event(_, 1))) else { continue };
        retract_checked = true;
        for (i, s) in tick.iter().enumerate() {
            if let Seen::Update(p) = s
                && (*p == stone || *p == head)
            {
                assert!(i > event, "the update of {p:?} (index {i}) came before the retract event (index {event}): {tick:?}");
            }
        }
        assert!(tick.iter().any(|s| *s == Seen::Update(head)), "the pulled stone's moving piston is sent: {tick:?}");
    }
    assert!(retract_checked, "the piston retracted");
}
