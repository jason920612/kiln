//! Replays the vanilla interaction vectors of `tools/InteractVectors.java`
//! (`KILN_INTERACT_VECTORS`, written by `tools/interact_vectors.py`) through the simulation:
//! each scenario's blocks are placed with commands, the player is set up as recorded, and every
//! step (a use, a sign or book edit, a pick) must leave the inventory, the watched blocks (state
//! and block entity data), the item entities and the packets of interest (sounds, sign editor,
//! block updates, block entity data, level events, overlay messages) exactly as vanilla had them.
//! Skipped when the vectors are not there.

use crate::testing::{Client, SinkStats, join};
use crate::{Sim, SimConfig};
use bytes::{Bytes, BytesMut};
use kiln_item::ItemStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use serde_json::{Value, json};

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn stack_of(h: &str) -> ItemStack {
    let bytes = unhex(h);
    let mut r = kiln_proto::Reader::new(&bytes);
    let s = ItemStack::read_optional(&mut r).unwrap_or_else(|e| panic!("stack {h}: {e:?}"));
    assert_eq!(r.remaining(), 0);
    s
}

fn stack_hex(s: &ItemStack) -> String {
    let mut out = BytesMut::new();
    s.write_optional(&mut out);
    hex(&out)
}

/// A tag with compound keys sorted (vanilla's order is a hash order).
fn sorted(t: &Tag) -> Tag {
    match t {
        Tag::Compound(f) => {
            let mut f: Vec<(String, Tag)> = f.iter().map(|(k, v)| (k.clone(), sorted(v))).collect();
            f.sort_by(|a, b| a.0.cmp(&b.0));
            Tag::Compound(f)
        }
        // A mixed list is written wrapped (`{"": value}`).
        Tag::List(l) => Tag::heterogeneous_list(l.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

fn tag_of(h: &str) -> Tag {
    kiln_proto::nbt::read_network(&unhex(h)).unwrap().0
}

fn sound_source_name(id: i32) -> &'static str {
    ["master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient", "voice", "ui"].get(id as usize).copied().unwrap_or("?")
}

/// Kiln's packet as the vectors print it (`None`: not a kind the vectors record).
fn decode(pkt: &Bytes) -> Option<Value> {
    use kiln_data::packets::play::clientbound as ids;
    let mut r = kiln_proto::codec::Reader::new(pkt);
    let id = r.varint().ok()?;
    let pos = |r: &mut kiln_proto::codec::Reader| kiln_proto::packets::read_position(r).map(|p| json!([p[0], p[1], p[2]]));
    Some(match id {
        ids::SOUND => {
            let holder = r.varint().ok()?;
            let name = if holder == 0 { "?".to_owned() } else { kiln_data::builtin_entries("minecraft:sound_event")?.get(holder as usize - 1)?.to_string() };
            let source = r.varint().ok()?;
            let (x, y, z) = (r.i32().ok()?, r.i32().ok()?, r.i32().ok()?);
            let (volume, pitch) = (r.f32().ok()?, r.f32().ok()?);
            json!({"t": "sound", "name": name, "source": sound_source_name(source), "pos": [x as f64 / 8.0, y as f64 / 8.0, z as f64 / 8.0],
                   "volume": volume, "pitch": pitch})
        }
        ids::OPEN_SIGN_EDITOR => {
            let p = pos(&mut r).ok()?;
            json!({"t": "open_sign_editor", "pos": p, "front": r.varint().ok()? != 0})
        }
        ids::BLOCK_UPDATE => {
            let p = pos(&mut r).ok()?;
            json!({"t": "block_update", "pos": p, "state": r.varint().ok()?})
        }
        ids::BLOCK_ENTITY_DATA => {
            let p = pos(&mut r).ok()?;
            let kind = r.varint().ok()?;
            let (tag, _) = kiln_proto::nbt::read_network(r.rest()).ok()?;
            let mut out = BytesMut::new();
            sorted(&tag).write_network(&mut out);
            json!({"t": "block_entity_data", "pos": p, "type": kiln_world::block_entity::type_name(kind as u16), "tag": hex(&out)})
        }
        ids::OPEN_BOOK => json!({"t": "open_book", "hand": r.varint().ok()?}),
        ids::SET_HELD_SLOT => json!({"t": "set_held_slot", "slot": r.varint().ok()?}),
        ids::LEVEL_EVENT => {
            let event = r.i32().ok()?;
            let p = pos(&mut r).ok()?;
            json!({"t": "level_event", "event": event, "pos": p, "data": r.i32().ok()?, "global": r.bool().ok()?})
        }
        ids::SYSTEM_CHAT => {
            let rest = r.rest();
            let (tag, n) = kiln_proto::nbt::read_network(rest).ok()?;
            let overlay = rest.get(n).copied()? != 0;
            let mut out = BytesMut::new();
            sorted(&tag).write_network(&mut out);
            json!({"t": "system_chat", "overlay": overlay, "text": hex(&out)})
        }
        _ => return None,
    })
}

const INTERESTING: [i32; 8] = [
    kiln_data::packets::play::clientbound::SET_HELD_SLOT,
    kiln_data::packets::play::clientbound::SOUND,
    kiln_data::packets::play::clientbound::OPEN_SIGN_EDITOR,
    kiln_data::packets::play::clientbound::BLOCK_UPDATE,
    kiln_data::packets::play::clientbound::BLOCK_ENTITY_DATA,
    kiln_data::packets::play::clientbound::OPEN_BOOK,
    kiln_data::packets::play::clientbound::LEVEL_EVENT,
    kiln_data::packets::play::clientbound::SYSTEM_CHAT,
];

fn take_packets(stats: &SinkStats) -> Vec<Value> {
    let all = std::mem::take(stats.log.lock().unwrap().as_mut().unwrap());
    all.iter()
        .filter(|p| kiln_proto::codec::Reader::new(p).varint().ok().is_some_and(|id| INTERESTING.contains(&id)))
        .filter_map(decode)
        .collect()
}

/// The vectors' packet as compared: a vanilla record in the same shape (compound tags sorted).
fn normalize_want(v: &Value) -> Value {
    let mut v = v.clone();
    match v["t"].as_str().unwrap() {
        "block_entity_data" | "system_chat" => {
            let key = if v["t"] == "system_chat" { "text" } else { "tag" };
            let tag = tag_of(v[key].as_str().unwrap());
            let mut out = BytesMut::new();
            sorted(&tag).write_network(&mut out);
            v[key] = Value::String(hex(&out));
        }
        _ => {}
    }
    v
}

fn i32_of(v: &Value) -> i32 {
    v.as_i64().unwrap() as i32
}

fn arr3(v: &Value) -> [i32; 3] {
    let a = v.as_array().unwrap();
    [i32_of(&a[0]), i32_of(&a[1]), i32_of(&a[2])]
}

fn set_slot(p: &mut crate::Player, key: &str, s: ItemStack) {
    let slot = match key {
        "feet" => Some(0),
        "legs" => Some(1),
        "chest" => Some(2),
        "head" => Some(3),
        "offhand" => Some(4),
        _ => None,
    };
    match slot {
        Some(i) => p.inv.equipment[i] = s,
        None => p.inv.items[key[1..].parse::<usize>().unwrap()] = s,
    }
}

fn run_case(line: &Value) -> Vec<String> {
    let mut sim = Sim::new(SimConfig::new(2, 2, None));
    let (msg, stats) = join(1, "Interact", 2);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats.clone());
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    // Another player, standing by, for the scenarios in which one holds a sign's editing lock.
    if line["steps"].as_array().unwrap().iter().any(|s| s["op"] == "lock_sign") {
        let (msg, stats2) = join(2, "Other", 2);
        assert!(sim.step([msg]));
        let mut other = Client::new(2, stats2);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            other.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let pos: Vec<f64> = line["pos"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
        sim.players.get_mut(&2).unwrap().pos = [pos[0] + 0.5, pos[1], pos[2]];
    }
    // (The vectors were recorded without announcements of advancements; with a datapack Kiln has them.)
    let mut console: Vec<ToSim> = vec![ToSim::Console("gamerule minecraft:show_advancement_messages false".into())];
    console.extend(line["commands"].as_array().unwrap().iter().map(|c| ToSim::Console(c.as_str().unwrap().to_owned())));
    assert!(sim.step(console));
    let pos: Vec<f64> = line["pos"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let rot: Vec<f32> = line["rot"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = [pos[0], pos[1], pos[2]];
        p.rot = [rot[0], rot[1]];
        p.on_ground = true;
        p.sneaking = line["sneaking"].as_bool().unwrap();
        p.fall_distance = 0.0;
        p.game_mode = match line["game_mode"].as_str().unwrap() {
            "creative" => 1,
            "adventure" => 2,
            "spectator" => 3,
            _ => 0,
        };
        p.inv = kiln_inventory::PlayerInventory::new();
        p.inv.selected = line["selected"].as_u64().unwrap() as usize;
        for (k, h) in line["slots"].as_object().unwrap() {
            set_slot(p, k, stack_of(h.as_str().unwrap()));
        }
        for (i, slot) in crate::combat::SLOTS.iter().enumerate() {
            p.equipment_seen[i] = p.inv.equipped(*slot).clone();
        }
    }
    // The settling step: the player's own packets of setup are not part of the scenario.
    assert!(sim.step([]));
    *stats.log.lock().unwrap() = Some(Vec::new());
    let mut errors = Vec::new();
    let steps = line["steps"].as_array().unwrap();
    let results = line["result"].as_array().unwrap();
    for (n, (step, want)) in steps.iter().zip(results).enumerate() {
        let mut inbox = Vec::new();
        let hand_of = |v: &Value| if i32_of(v) == 0 { Hand::Main } else { Hand::Off };
        match step["op"].as_str().unwrap() {
            "use" => inbox.push(ToSim::Packet(1, PlayIn::UseItem { hand: hand_of(&step["hand"]), sequence: 1, yaw: rot[0], pitch: rot[1] })),
            "use_on" => {
                let c = step["cursor"].as_array().unwrap();
                inbox.push(ToSim::Packet(
                    1,
                    PlayIn::UseItemOn {
                        hand: i32_of(&step["hand"]),
                        pos: arr3(&step["pos"]),
                        face: i32_of(&step["face"]),
                        cursor: [c[0].as_f64().unwrap() as f32, c[1].as_f64().unwrap() as f32, c[2].as_f64().unwrap() as f32],
                        inside: false,
                        sequence: 1,
                    },
                ));
            }
            "sign_update" => {
                let l: Vec<String> = step["lines"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect();
                let lines: [String; 4] = l.try_into().unwrap();
                inbox.push(ToSim::Packet(1, PlayIn::SignUpdate { pos: arr3(&step["pos"]), lines: Box::new(lines), front: step["front"].as_bool().unwrap() }));
            }
            "edit_book" => {
                let pages: Vec<String> = step["pages"].as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_owned()).collect();
                inbox.push(ToSim::Packet(
                    1,
                    PlayIn::EditBook { slot: i32_of(&step["slot"]), pages, title: step["title"].as_str().map(str::to_owned) },
                ));
            }
            "pick_block" => inbox.push(ToSim::Packet(1, PlayIn::PickItemFromBlock { pos: arr3(&step["pos"]), include_data: step["include"].as_bool().unwrap() })),
            "select" => inbox.push(ToSim::Packet(1, PlayIn::SetCarriedItem { slot: i32_of(&step["slot"]) as i16 })),
            "cooldown" => {
                let item = ItemStack::of(step["item"].as_str().unwrap(), 1).unwrap();
                let p = sim.players.get_mut(&1).unwrap();
                p.add_cooldown(&item, i32_of(&step["ticks"]));
            }
            "lock_sign" => sim.lock_sign(2, arr3(&step["pos"])),
            other => panic!("unknown op {other}"),
        }
        assert!(sim.step(inbox));
        let mut eq = |what: &str, got: String, expected: String| {
            if got != expected {
                errors.push(format!("step {n} ({}) {what}: kiln {got}, vanilla {expected}", step["op"]));
            }
        };
        let p = &sim.players[&1];
        let inv = &want["inv"];
        for (i, h) in inv["items"].as_array().unwrap().iter().enumerate() {
            eq(&format!("slot {i}"), stack_hex(&p.inv.items[i]), h.as_str().unwrap().to_owned());
        }
        for (i, k) in ["feet", "legs", "chest", "head", "offhand"].iter().enumerate() {
            eq(k, stack_hex(&p.inv.equipment[i]), inv[*k].as_str().unwrap().to_owned());
        }
        eq("selected", p.inv.selected.to_string(), inv["selected"].to_string());
        // The same packets, whatever the order: vanilla sends a sound the moment it is made and
        // the block changes at the end of the tick, Kiln's regions deliver both with the tick's
        // block work (the client cannot tell).
        let mut got_packets: Vec<String> = take_packets(&stats).iter().map(|v| v.to_string()).collect();
        let mut want_packets: Vec<String> = want["packets"].as_array().unwrap().iter().map(|v| normalize_want(v).to_string()).collect();
        got_packets.sort();
        want_packets.sort();
        eq("packets", format!("{got_packets:?}"), format!("{want_packets:?}"));
        for b in want["blocks"].as_array().unwrap() {
            let at = arr3(&b["pos"]);
            eq(&format!("block {at:?}"), sim.block_at(at[0], at[1], at[2]).map_or(-1, i32::from).to_string(), b["state"].to_string());
            // Vanilla's `saveWithFullMetadata` always writes the (possibly empty) `components`.
            // The chunk's copy, with what a live block entity (a furnace, a sign editor's spawner...)
            // holds in its own fields on top.
            let saved = sim.block_entity_saved(at[0], at[1], at[2]).map(|mut t| {
                if let (Tag::Compound(f), Some(Tag::Compound(live))) = (&mut t, sim.block_entity_nbt(at[0], at[1], at[2])) {
                    for (k, v) in live {
                        f.retain(|(ok, _)| *ok != k);
                        f.push((k, v));
                    }
                }
                t
            });
            let got = saved.map(|t| match sorted(&t) {
                Tag::Compound(mut f) => {
                    if !f.iter().any(|(k, _)| k == "components") {
                        f.push(("components".into(), Tag::Compound(Vec::new())));
                        f.sort_by(|a, b| a.0.cmp(&b.0));
                    }
                    Tag::Compound(f)
                }
                other => other,
            });
            let expected = b["be"].as_str().map(|h| sorted(&tag_of(h)));
            eq(&format!("block entity {at:?}"), format!("{got:?}"), format!("{expected:?}"));
        }
        let mut got_items: Vec<String> = sim.item_stacks().iter().map(stack_hex).collect();
        got_items.sort();
        let want_items: Vec<String> = want["entities"].as_array().unwrap().iter().map(|e| e["item"].as_str().unwrap().to_owned()).collect();
        eq("item entities", format!("{got_items:?}"), format!("{want_items:?}"));
        let p = &sim.players[&1];
        for (item, want_count) in want["used"].as_object().unwrap() {
            let id = kiln_item::registry::ITEM.id(item).unwrap();
            eq(&format!("used {item}"), p.stats.get(crate::player_stats::Stat::item(crate::player_stats::USED, id)).to_string(), want_count.to_string());
        }
        if errors.len() > 8 {
            break;
        }
    }
    errors
}

/// What middle click gives for every state of every block (`pick_table.jsonl` beside the
/// vectors): `BlockState.getCloneItemStack` without block entity data.
#[test]
fn pick_table_parity() {
    let Some(path) = std::env::var_os("KILN_INTERACT_VECTORS") else {
        eprintln!("skipped: set KILN_INTERACT_VECTORS (tools/interact_vectors.py)");
        return;
    };
    let table = std::path::Path::new(&path).with_file_name("pick_table.jsonl");
    let Ok(text) = std::fs::read_to_string(table) else { return };
    let (mut checked, mut wrong) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let Some(state) = v["state"].as_u64() else { continue };
        let want = v["item"].as_str().unwrap();
        let name = v["block"].as_str().unwrap();
        // Blocks that carry their block entity's data into the picked item (banners, decorated
        // pots, copper golem statues...) are not modelled: only the item is compared.
        let got = crate::pick::clone_item(state as u16).unwrap_or_else(ItemStack::empty);
        let item_only = |h: &str| stack_of(h).item_name().to_owned();
        let same = if matches!(
            name.trim_start_matches("minecraft:"),
            "decorated_pot" | "copper_golem_statue" | "exposed_copper_golem_statue" | "weathered_copper_golem_statue" | "oxidized_copper_golem_statue"
        ) || name.ends_with("banner")
        {
            item_only(&stack_hex(&got)) == item_only(want)
        } else {
            stack_hex(&got) == want
        };
        checked += 1;
        if !same {
            wrong.push(format!("{name} state {state}: kiln {}, vanilla {want}", stack_hex(&got)));
        }
    }
    println!("pick table: {checked} states, {} differ", wrong.len());
    assert!(wrong.is_empty(), "{:#?}", &wrong[..wrong.len().min(20)]);
}

#[test]
fn interact_parity() {
    let Some(path) = std::env::var_os("KILN_INTERACT_VECTORS") else {
        eprintln!("skipped: set KILN_INTERACT_VECTORS (tools/interact_vectors.py)");
        return;
    };
    let filter = std::env::var("KILN_PARITY_FILTER").ok();
    let text = std::fs::read_to_string(path).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap().to_owned();
        assert!(v.get("error").is_none(), "{name}: vanilla failed: {}", v["error"]);
        if filter.as_ref().is_some_and(|f| !name.contains(f.as_str())) {
            continue;
        }
        let errors = run_case(&v);
        if errors.is_empty() {
            passed += 1;
        } else {
            println!("FAIL {name}\n  {}", errors.join("\n  "));
            failed.push(name);
        }
    }
    println!("interact parity: {passed} passed, {} failed", failed.len());
    assert!(failed.is_empty(), "failed: {failed:?}");
}
