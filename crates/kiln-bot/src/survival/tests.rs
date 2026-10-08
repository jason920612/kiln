//! A survival agent against a stone floor, its packets decoded by the server's own decoder.

use super::steps::PlaceSpec;
use super::*;
use crate::metrics::Shared;
use bytes::BytesMut;
use kiln_data::blocks::default_state as d;
use kiln_proto::packets::{self as server, PlayIn};

fn agent(role: Role) -> Agent {
    let cfg = Settings { role, site: [0.0, 0.0], view_distance: 2, seed: 7, chat_interval: None, stay: true };
    let mut a = Agent::new(cfg, Arc::new(Shared::default()), [0.5, 0.0, 0.5], 0.0, 0.0, 1);
    a.world = crate::world::test_world();
    a.phase = Phase::Active;
    a.survival_set = true;
    a
}

/// Decodes what the agent sent, with the server's decoder (packets it ignores are skipped).
fn decode(out: &mut Out) -> Vec<PlayIn> {
    let mut framed = out.take_framed();
    let mut got = Vec::new();
    while !framed.is_empty() {
        let mut r = Reader::new(&framed);
        let len = r.varint().unwrap() as usize;
        let header = framed.len() - r.remaining();
        let payload = framed[header..header + len].to_vec();
        let mut r = Reader::new(&payload);
        let id = r.varint().unwrap();
        if let Some(p) = server::decode_play(id, &mut r).unwrap() {
            got.push(p);
        }
        let _ = framed.split_to(header + len);
    }
    got
}

fn run(a: &mut Agent, out: &mut Out, ticks: usize) -> Vec<(usize, PlayIn)> {
    let mut all = Vec::new();
    for t in 0..ticks {
        a.tick(out);
        all.extend(decode(out).into_iter().map(|p| (t, p)));
    }
    all
}

fn actions(sent: &[(usize, PlayIn)]) -> Vec<(usize, i32, [i32; 3])> {
    sent.iter()
        .filter_map(|(t, p)| match p {
            PlayIn::PlayerAction { action, pos, .. } => Some((*t, *action, *pos)),
            _ => None,
        })
        .collect()
}

#[test]
fn breaks_stone_in_the_vanilla_time_and_counts_the_ack() {
    let mut a = agent(Role::Miner);
    let mut out = Out::new(None);
    a.hot[0] = Slot { item: "iron_pickaxe", count: 1 };
    a.queue.push_back(Step::dig([0, -1, 0]));
    let sent = run(&mut a, &mut out, 40);
    let acts = actions(&sent);
    // Iron pickaxe on stone: 6 / 1.5 / 30 per tick, so 8 ticks between start and stop.
    assert_eq!(acts.len(), 2, "{acts:?}");
    assert_eq!((acts[0].1, acts[0].2), (0, [0, -1, 0]), "start destroy");
    assert_eq!((acts[1].1, acts[1].2), (3, [0, -1, 0]), "stop destroy");
    assert_eq!(acts[1].0 - acts[0].0, 8);
    // Swings every tick in between, and the pickaxe was selected.
    let swings = sent.iter().filter(|(t, p)| matches!(p, PlayIn::Punch) && *t > acts[0].0 && *t <= acts[1].0).count();
    assert_eq!(swings, 8);
    // The pickaxe is in slot 0, which is held already; no slot change was sent.
    assert!(!sent.iter().any(|(_, p)| matches!(p, PlayIn::SetCarriedItem { .. })));
    assert_eq!(a.counts.dig_started, 1);
    // The server breaks it and acknowledges the sequence of the stop packet.
    a.on_block_update([0, -1, 0], d::AIR);
    a.on_ack(a.seq);
    assert_eq!((a.counts.dig_done, a.counts.dig_rejected), (1, 0));
}

#[test]
fn a_block_the_server_keeps_counts_as_rejected() {
    let mut a = agent(Role::Miner);
    let mut out = Out::new(None);
    a.hot[0] = Slot { item: "iron_pickaxe", count: 1 };
    a.queue.push_back(Step::dig([0, -1, 0]));
    run(&mut a, &mut out, 20);
    a.on_ack(a.seq);
    assert_eq!((a.counts.dig_done, a.counts.dig_rejected), (0, 1));
}

#[test]
fn places_against_the_floor_after_looking_and_sneaks_on_containers() {
    let mut a = agent(Role::Builder);
    let mut out = Out::new(None);
    a.hot[4] = Slot { item: "oak_planks", count: 8 };
    let spec = PlaceSpec { pos: [0, 0, 2], slot: 4, expect: "minecraft:oak_planks", look: None, from: None, facing: None };
    a.queue.push_back(Step::place(spec));
    let sent = run(&mut a, &mut out, 12);
    let used: Vec<_> = sent.iter().filter(|(_, p)| matches!(p, PlayIn::UseItemOn { .. })).collect();
    assert_eq!(used.len(), 1, "{sent:?}");
    match &used[0].1 {
        PlayIn::UseItemOn { pos, face, cursor, .. } => {
            assert_eq!(*pos, [0, -1, 2]);
            assert_eq!(*face, 1, "up");
            assert_eq!(*cursor, [0.5, 1.0, 0.5]);
        }
        _ => unreachable!(),
    }
    // The rotation toward the block was sent a tick before the click.
    let rot_tick = sent.iter().find(|(_, p)| matches!(p, PlayIn::Move { rot: Some(_), .. })).map(|x| x.0).unwrap();
    assert!(rot_tick < used[0].0);
    a.on_block_update([0, 0, 2], d::OAK_PLANKS);
    a.on_ack(a.seq);
    assert_eq!((a.counts.placed, a.counts.place_rejected), (1, 0));
    assert_eq!(a.hot[4].count, 7);

    // Against a chest the bot sneaks first (the click would open it otherwise).
    let mut a = agent(Role::Redstone);
    a.world.set(1, 0, 2, d::CHEST);
    a.hot[4] = Slot { item: "hopper", count: 8 };
    let spec = PlaceSpec { pos: [0, 0, 2], slot: 4, expect: "minecraft:hopper", look: None, from: Some(Dir::East), facing: Some(Dir::East) };
    a.queue.push_back(Step::place(spec));
    let sent = run(&mut a, &mut out, 12);
    let sneak = sent.iter().find(|(_, p)| matches!(p, PlayIn::PlayerInput { flags } if flags & 0x20 != 0)).map(|x| x.0);
    let click = sent.iter().find(|(_, p)| matches!(p, PlayIn::UseItemOn { .. })).map(|x| x.0);
    assert!(sneak.is_some() && sneak < click, "sneak {sneak:?}, click {click:?}");
    match sent.iter().find_map(|(_, p)| if let PlayIn::UseItemOn { face, pos, .. } = p { Some((*face, *pos)) } else { None }) {
        Some((face, pos)) => assert_eq!((pos, face), ([1, 0, 2], 4), "the chest's west face"),
        None => panic!("no click"),
    }
}

#[test]
fn walking_never_enters_a_wall() {
    let mut a = agent(Role::Explorer);
    let mut out = Out::new(None);
    // A wall of stone two blocks high, three blocks ahead (+z), wide enough to need a detour.
    for x in -6..=6 {
        for y in 0..2 {
            a.world.set(x, y, 4, d::STONE);
        }
    }
    a.queue.push_back(Step::walk([0.5, 12.5], 1.0, true));
    for _ in 0..400 {
        a.tick(&mut out);
        decode(&mut out);
        assert!(!(a.body.pos[2] > 3.7 && a.body.pos[2] < 5.3 && a.body.pos[0].abs() < 6.0 && a.body.pos[1] < 1.9), "inside the wall: {:?}", a.body.pos);
    }
    assert!(a.queue.is_empty() || a.body.pos[2] < 12.0);
}

#[test]
fn chunk_waits_are_measured_per_chunk_and_per_view() {
    let shared = Arc::new(Shared::default());
    let cfg = Settings { role: Role::Explorer, site: [0.0, 0.0], view_distance: 2, seed: 1, chat_interval: None, stay: true };
    let mut a = Agent::new(cfg, shared.clone(), [0.5, 0.0, 0.5], 0.0, 0.0, 1);
    a.on_chunk_center(0, 0);
    assert_eq!(a.track.want.len(), 25);
    for x in -2..=2 {
        for z in -2..=2 {
            a.on_chunk_coords(x, z);
        }
    }
    assert!(a.track.want.is_empty());
    assert_eq!(shared.chunk_latency.summary().n, 25);
    assert_eq!(shared.area_ready_join.summary().n, 1);
    // Walking over a chunk border asks for the new column of five; teleporting far asks for all.
    a.on_chunk_center(1, 0);
    assert_eq!(a.track.want.len(), 5);
    for z in -2..=2 {
        a.on_chunk_coords(3, z);
    }
    assert_eq!(shared.area_ready_walk.summary().n, 1);
    a.on_chunk_center(50, 50);
    assert_eq!(a.track.want.len(), 25);
    // Chunks that arrive for a view that is gone do not count.
    a.on_chunk_coords(0, 0);
    assert_eq!(a.track.want.len(), 25);
}

#[test]
fn eats_when_hungry_and_counts_it() {
    let mut a = agent(Role::Explorer);
    let mut out = Out::new(None);
    a.on_health(20.0, 8);
    let sent = run(&mut a, &mut out, 120);
    assert!(sent.iter().any(|(_, p)| matches!(p, PlayIn::UseItem { .. })), "{:?}", sent.iter().map(|x| &x.1).collect::<Vec<_>>());
    assert_eq!(a.counts.eaten, 1);
}

#[allow(dead_code)]
fn unused(_: BytesMut) {}
