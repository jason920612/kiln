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
            // (The vectors print floats the way Java does: the shortest text of the float.)
            let java = |f: f32| format!("{f}").parse::<f64>().unwrap_or(f as f64);
            json!({"t": "sound", "name": name, "source": sound_source_name(source), "pos": [x as f64 / 8.0, y as f64 / 8.0, z as f64 / 8.0],
                   "volume": java(volume), "pitch": java(pitch)})
        }
        ids::SOUND_ENTITY => {
            let holder = r.varint().ok()?;
            let name = if holder == 0 { "?".to_owned() } else { kiln_data::builtin_entries("minecraft:sound_event")?.get(holder as usize - 1)?.to_string() };
            let source = r.varint().ok()?;
            let _entity = r.varint().ok()?;
            let (volume, pitch) = (r.f32().ok()?, r.f32().ok()?);
            let java = |f: f32| format!("{f}").parse::<f64>().unwrap_or(f as f64);
            json!({"t": "sound_entity", "name": name, "source": sound_source_name(source), "volume": java(volume), "pitch": java(pitch)})
        }
        ids::MAP_ITEM_DATA => {
            let id = r.varint().ok()?;
            let scale = r.u8().ok()? as i8;
            let locked = r.bool().ok()?;
            let decos = if r.bool().ok()? {
                let n = r.varint().ok()?;
                let mut rows = Vec::new();
                for _ in 0..n {
                    let kind = r.varint().ok()?;
                    let (x, y, rot) = (r.u8().ok()? as i8, r.u8().ok()? as i8, r.u8().ok()? as i8);
                    let name = if r.bool().ok()? {
                        let tail = r.rest();
                        let (tag, used) = kiln_proto::nbt::read_network(tail).ok()?;
                        r = kiln_proto::codec::Reader::new(&tail[used..]);
                        let mut out = BytesMut::new();
                        sorted(&tag).write_network(&mut out);
                        Value::String(hex(&out))
                    } else {
                        Value::Null
                    };
                    rows.push(json!([kind, x, y, rot, name]));
                }
                Value::Array(rows)
            } else {
                Value::Null
            };
            let w = r.u8().ok()? as i32;
            let patch = if w > 0 {
                let (h, x, y) = (r.u8().ok()? as i32, r.u8().ok()? as i32, r.u8().ok()? as i32);
                let n = r.varint().ok()? as usize;
                let colors = r.bytes(n).ok()?.to_vec();
                json!([x, y, w, h, hex(&colors)])
            } else {
                Value::Null
            };
            json!({"t": "map", "id": id, "scale": scale, "locked": locked, "decos": decos, "patch": patch})
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
        // The menu packets (recorded for cases that watch menus).
        ids::OPEN_SCREEN => {
            let (container, kind) = (r.varint().ok()?, r.varint().ok()?);
            let (tag, _) = kiln_proto::nbt::read_network(r.rest()).ok()?;
            let mut out = BytesMut::new();
            sorted(&tag).write_network(&mut out);
            json!({"t": "open_screen", "id": container, "type": kind, "title": hex(&out)})
        }
        ids::CONTAINER_SET_CONTENT => {
            let (container, _state) = (r.varint().ok()?, r.varint().ok()?);
            let n = r.varint().ok()?;
            let items: Vec<Value> = (0..n).map(|_| ItemStack::read_optional(&mut r).ok().map(|s| Value::String(stack_hex(&s)))).collect::<Option<_>>()?;
            let carried = ItemStack::read_optional(&mut r).ok()?;
            json!({"t": "set_content", "id": container, "items": items, "carried": stack_hex(&carried)})
        }
        ids::CONTAINER_SET_SLOT => {
            let (container, _state) = (r.varint().ok()?, r.varint().ok()?);
            let slot = r.i16().ok()?;
            let item = ItemStack::read_optional(&mut r).ok()?;
            json!({"t": "set_slot", "id": container, "slot": slot, "item": stack_hex(&item)})
        }
        ids::CONTAINER_SET_DATA => {
            let container = r.varint().ok()?;
            json!({"t": "set_data", "id": container, "index": r.i16().ok()?, "value": r.i16().ok()?})
        }
        ids::CONTAINER_CLOSE => json!({"t": "container_close", "id": r.varint().ok()?}),
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

const INTERESTING: [i32; 15] = [
    kiln_data::packets::play::clientbound::SOUND_ENTITY,
    kiln_data::packets::play::clientbound::MAP_ITEM_DATA,
    kiln_data::packets::play::clientbound::OPEN_SCREEN,
    kiln_data::packets::play::clientbound::CONTAINER_SET_CONTENT,
    kiln_data::packets::play::clientbound::CONTAINER_SET_SLOT,
    kiln_data::packets::play::clientbound::CONTAINER_SET_DATA,
    kiln_data::packets::play::clientbound::CONTAINER_CLOSE,
    kiln_data::packets::play::clientbound::SET_HELD_SLOT,
    kiln_data::packets::play::clientbound::SOUND,
    kiln_data::packets::play::clientbound::OPEN_SIGN_EDITOR,
    kiln_data::packets::play::clientbound::BLOCK_UPDATE,
    kiln_data::packets::play::clientbound::BLOCK_ENTITY_DATA,
    kiln_data::packets::play::clientbound::OPEN_BOOK,
    kiln_data::packets::play::clientbound::LEVEL_EVENT,
    kiln_data::packets::play::clientbound::SYSTEM_CHAT,
];

fn take_packets(stats: &SinkStats, menus: bool, maps: bool) -> Vec<Value> {
    use kiln_data::packets::play::clientbound as ids;
    let all = std::mem::take(stats.log.lock().unwrap().as_mut().unwrap());
    all.iter()
        .filter(|p| {
            kiln_proto::codec::Reader::new(p).varint().ok().is_some_and(|id| {
                INTERESTING.contains(&id) && (maps || !matches!(id, ids::SOUND_ENTITY | ids::MAP_ITEM_DATA)) && (menus || !matches!(id, ids::OPEN_SCREEN | ids::CONTAINER_SET_CONTENT | ids::CONTAINER_SET_SLOT | ids::CONTAINER_SET_DATA | ids::CONTAINER_CLOSE))
            })
        })
        .filter_map(decode)
        .collect()
}

/// The vectors' packet as compared: a vanilla record in the same shape (compound tags sorted).
fn normalize_want(v: &Value) -> Value {
    let mut v = v.clone();
    match v["t"].as_str().unwrap() {
        "block_entity_data" | "system_chat" | "open_screen" => {
            let key = match v["t"].as_str().unwrap() {
                "system_chat" => "text",
                "open_screen" => "title",
                _ => "tag",
            };
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

/// The hanging entities of the level, as `InteractVectors.hangings` lists them: [type, x, y, z,
/// facing, item, rotation, painting area], sorted.
fn hangings_json(sim: &Sim) -> Value {
    let mut rows: Vec<(String, f64, f64, f64, Value)> = Vec::new();
    for region in sim.dims[crate::OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            let Some(dir) = kiln_entity::ext_entity::hanging::direction_of(phys) else { continue };
            let (item, rot, area) = if let Some(f) = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::item_frame::ItemFrame>(phys) {
                (if f.item.is_empty() { Value::Null } else { Value::String(stack_hex(&f.item)) }, f.rotation, 0)
            } else if let Some(p) = kiln_entity::ext_entity::get::<kiln_entity::ext_entity::painting::Painting>(phys) {
                let (w, h) = p.size();
                (Value::Null, 0, w * h)
            } else {
                continue;
            };
            let p = phys.position();
            rows.push((phys.type_name.to_owned(), p.x, p.y, p.z, json!([phys.type_name, p.x, p.y, p.z, dir.index(), item, rot, area])));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)).then(a.3.total_cmp(&b.3)));
    Value::Array(rows.into_iter().map(|r| r.4).collect())
}

/// The saved data of the maps in the player's inventory, as `InteractVectors.mapsOf` lists them.
fn maps_json(sim: &Sim) -> Value {
    // Every map made so far (they all begin at id 0 in a case).
    let mut store = sim.maps.lock().unwrap();
    let ids: Vec<i32> = (0..=store.last_id()).collect();
    let mut rows = Vec::new();
    for id in ids {
        let Some(d) = store.get(id) else { continue };
        let decos: Vec<Value> = d
            .decorations()
            .map(|x| {
                let name = x.name.as_ref().map_or(Value::Null, |t| {
                    let mut out = BytesMut::new();
                    sorted(t).write_network(&mut out);
                    Value::String(hex(&out))
                });
                json!([x.kind, x.x, x.y, x.rot, name])
            })
            .collect();
        rows.push(json!({"id": id, "scale": d.scale, "center": [d.center[0], d.center[1]], "locked": d.locked, "tracking": d.tracking_position,
                         "unlimited": d.unlimited_tracking, "colors": hex(&d.colors), "decos": decos}));
    }
    Value::Array(rows)
}

/// A stand's saved data as compared: its fields (no uuid; where it stands and how it moved are
/// the level's), sorted, each as text.
fn stand_fields(t: &Tag) -> Vec<(String, String)> {
    let Tag::Compound(fields) = sorted(t) else { return Vec::new() };
    fields
        .into_iter()
        .filter(|(k, _)| !matches!(k.as_str(), "UUID" | "OnGround" | "Motion" | "fall_distance" | "id"))
        // (The fire burns down with Kiln's ticks; whether it burns is compared.)
        .map(|(k, v)| if k == "Fire" { (k, format!("{}", v.as_i64().unwrap_or(0) > 0)) } else { (k, format!("{v:?}")) })
        .collect()
}

type StandRow = ([f64; 3], Vec<(String, String)>);

/// A beehive's saved data with every bee's `ticks_in_hive` zeroed.
fn no_hive_ticks(t: Tag) -> Tag {
    let Tag::Compound(mut fields) = t else { return t };
    for (k, v) in fields.iter_mut() {
        if k == "bees"
            && let Tag::List(list) = v
        {
            for bee in list.iter_mut() {
                if let Tag::Compound(bf) = bee {
                    for (bk, bv) in bf.iter_mut() {
                        if bk == "ticks_in_hive" {
                            *bv = Tag::Int(0);
                        }
                    }
                }
            }
        }
    }
    Tag::Compound(fields)
}

/// A trial spawner's saved data with the UUIDs of its mobs made alike (the mobs of the vectors are not Kiln's).
fn no_mob_uuids(t: Tag) -> Tag {
    let Tag::Compound(mut fields) = t else { return t };
    for (k, v) in fields.iter_mut() {
        if k == "current_mobs"
            && let Tag::List(list) = v
        {
            for u in list.iter_mut() {
                *u = Tag::IntArray(vec![0; 4]);
            }
        }
    }
    Tag::Compound(fields)
}

/// The living mobs of the level: (type, x, y, z), as `InteractVectors.mobRows` lists them.
fn mob_rows(sim: &Sim) -> Vec<(String, f64, f64, f64)> {
    let mut rows = Vec::new();
    for region in sim.dims[crate::OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            if matches!(phys.kind, kiln_entity::entity::EntityKind::Mob(_)) && kiln_entity::mob::data(phys).is_some_and(|m| kiln_entity::mob::is_alive(phys, m)) {
                let p = phys.position();
                rows.push((phys.type_name.to_owned(), p.x, p.y, p.z));
            }
        }
    }
    rows
}

/// The armor stands of the level, sorted by position.
fn stand_rows(sim: &Sim) -> Vec<StandRow> {
    let mut rows: Vec<StandRow> = Vec::new();
    for region in sim.dims[crate::OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            if phys.type_name != "minecraft:armor_stand" {
                continue;
            }
            let p = phys.position();
            rows.push(([p.x, p.y, p.z], stand_fields(&kiln_entity::persist::save(phys, &|_| None))));
        }
    }
    rows.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])).then(a.0[2].total_cmp(&b.0[2])));
    rows
}

/// The stands the vectors name (`[x, y, z, saved data (hex)]` each), as rows.
fn want_stand_rows(v: &Value) -> Vec<StandRow> {
    v.as_array().unwrap().iter().map(|r| ([r[0].as_f64().unwrap(), r[1].as_f64().unwrap(), r[2].as_f64().unwrap()], stand_fields(&tag_of(r[3].as_str().unwrap())))).collect()
}

/// What differs between the stands Kiln has and the vectors name ("" when nothing).
fn stand_diff(got: &[StandRow], want: &[StandRow]) -> String {
    if got.len() != want.len() {
        return format!("{} stands at {:?}, vanilla {} at {:?}", got.len(), got.iter().map(|r| r.0).collect::<Vec<_>>(), want.len(), want.iter().map(|r| r.0).collect::<Vec<_>>());
    }
    let mut out = Vec::new();
    for (g, w) in got.iter().zip(want) {
        if g.0.iter().zip(&w.0).any(|(a, b)| (a - b).abs() > 1.0e-6) {
            out.push(format!("stand at {:?}, vanilla {:?}", g.0, w.0));
        }
        let mut keys: Vec<&String> = g.1.iter().chain(&w.1).map(|(k, _)| k).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            let (a, b) = (g.1.iter().find(|(x, _)| x == k), w.1.iter().find(|(x, _)| x == k));
            if a.map(|f| &f.1) != b.map(|f| &f.1) {
                out.push(format!("{k}: kiln {}, vanilla {}", a.map_or("-", |f| f.1.as_str()), b.map_or("-", |f| f.1.as_str())));
            }
        }
    }
    out.join("; ")
}

/// The id of the hanging entity (or armor stand) nearest to `at`.
fn nearest_hanging(sim: &Sim, at: [f64; 3]) -> Option<i32> {
    let mut best: Option<(f64, i32)> = None;
    for region in sim.dims[crate::OVERWORLD_ID].regions.iter() {
        for e in region.part().0.list.iter().filter(|e| !e.removed) {
            let Some(phys) = e.phys.as_deref() else { continue };
            if kiln_entity::ext_entity::hanging::direction_of(phys).is_none() && phys.type_name != "minecraft:armor_stand" {
                continue;
            }
            let p = phys.position();
            let d = (p.x - at[0]).powi(2) + (p.y - at[1]).powi(2) + (p.z - at[2]).powi(2);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, e.id));
            }
        }
    }
    best.map(|b| b.1)
}

fn run_case(line: &Value) -> Vec<String> {
    // (A map covers 128 blocks around the origin: the replay's player sees as far.)
    let maps = line["maps"].as_bool() == Some(true);
    let view = if maps { 8 } else { 2 };
    let mut config = SimConfig::new(2, view, None);
    config.enable_command_block = true;
    let mut sim = Sim::new(config);
    // (Vaults roll loot tables: the level has the vanilla ones whatever the working directory.)
    if line["ticking"].as_bool() == Some(true) && sim.loot.is_none() {
        sim.loot = crate::combat_parity::vanilla_loot();
    }
    // (... and trial spawners the configs of the datapack.)
    if line["ticking"].as_bool() == Some(true) && sim.trial_configs.len() == 0 {
        let work = std::env::var_os("KILN_WORK").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        sim.trial_configs = std::sync::Arc::new(crate::mob_spawner::TrialConfigs::load(&work.join("generated")));
    }
    let (msg, stats) = join(1, "Interact", view);
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
    // The recorded level's clock stands at 100 as each scenario begins: the setup commands come just before
    // (a block they place that would not last, a kelp plant without support, is ticked as little as in the
    // vectors' frozen level).
    while sim.game_time() < 98 {
        let mut idle = Vec::new();
        client.tick(None, &mut idle);
        assert!(sim.step(idle));
    }
    // (The vectors were recorded without announcements of advancements; with a datapack Kiln has them.)
    let mut console: Vec<ToSim> = vec![ToSim::Console("gamerule minecraft:show_advancement_messages false".into())];
    if line["op"].as_bool() == Some(true) {
        console.push(ToSim::Console("op Interact".into()));
    }
    console.extend(line["commands"].as_array().unwrap().iter().map(|c| ToSim::Console(c.as_str().unwrap().to_owned())));
    assert!(sim.step(console));
    let pos: Vec<f64> = line["pos"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let rot: Vec<f32> = line["rot"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    {
        let p = sim.players.get_mut(&1).unwrap();
        p.pos = [pos[0], pos[1], pos[2]];
        p.rot = [rot[0], rot[1]];
        p.on_ground = true;
        p.set_shift_key(line["sneaking"].as_bool().unwrap());
        p.fall_distance = 0.0;
        p.food = line["food"].as_i64().map_or(20, |f| f as i32);
        // (Vaults remember players by uuid: the vectors' mock player's.)
        if let Some(u) = line["player_uuid"].as_str() {
            p.uuid = uuid::Uuid::parse_str(u).unwrap();
        }
        p.game_mode = match line["game_mode"].as_str().unwrap() {
            "creative" => 1,
            "adventure" => 2,
            "spectator" => 3,
            _ => 0,
        };
        p.loot = crate::combat_parity::vanilla_loot();
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
    // The recorded level's clock stands at 100 as each scenario begins.
    // (A level that ticks whole has made the tick of its setup by then.)
    let start = line["clock"].as_i64().unwrap_or(100);
    while sim.game_time() < start {
        assert!(sim.step([]));
    }
    *stats.log.lock().unwrap() = Some(Vec::new());
    // Commands the vectors ran at the start but the replay runs now, after its level has settled (a
    // hive ages with every tick; the vectors' level made one for it, `InteractVectors.run`).
    if let Some(late) = line["late"].as_array().filter(|l| !l.is_empty()) {
        let mut inbox: Vec<ToSim> = late.iter().map(|c| ToSim::Console(c.as_str().unwrap().to_owned())).collect();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let _ = take_packets(&stats, false, false);
    }
    let mut errors = Vec::new();
    let mut seen_bees: std::collections::HashSet<i32> = Default::default();
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
            "use_entity" | "attack_entity" => {
                let at: Vec<f64> = step["pos"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
                let Some(id) = nearest_hanging(&sim, [at[0], at[1], at[2]]) else { panic!("{}: no hanging entity near {at:?} at step {n}", line["name"]) };
                if step["op"] == "attack_entity" {
                    inbox.push(ToSim::Packet(1, PlayIn::Attack { entity_id: id }));
                } else {
                    // (The hit point is relative to the entity, which is where the vectors aim: its middle.)
                    inbox.push(ToSim::Packet(
                        1,
                        PlayIn::Interact { entity_id: id, hand: hand_of(&step["hand"]), location: step["hit"].as_array().map_or([0.0; 3], |h| [h[0].as_f64().unwrap(), h[1].as_f64().unwrap(), h[2].as_f64().unwrap()]), sneaking: step["sneak"].as_bool().unwrap() },
                    ));
                }
            }
            "command" => inbox.push(ToSim::Console(step["command"].as_str().unwrap().to_owned())),
            "kill_mobs" => inbox.push(ToSim::Console("kill @e[type=minecraft:zombie]".to_owned())),
            // The game time moves on (the recorded level does not tick, so its clock is moved by hand).
            "wait" => {
                // (A level that ticks the block entities in the vectors makes `ticks` of them, the step's own included.)
                let first = i32::from(line["ticking"].as_bool() == Some(true));
                for _ in first..i32_of(&step["ticks"]) {
                    let mut idle = Vec::new();
                    client.tick(None, &mut idle);
                    assert!(sim.step(idle));
                }
            }
            // `ticks` server ticks pass for the maps: the step's own tick is one of them.
            "map_wait" => {
                for _ in 1..i32_of(&step["ticks"]) {
                    let mut idle = Vec::new();
                    client.tick(None, &mut idle);
                    assert!(sim.step(idle));
                }
            }
            "select" => inbox.push(ToSim::Packet(1, PlayIn::SetCarriedItem { slot: i32_of(&step["slot"]) as i16 })),
            "menu_button" => {
                let id = sim.players[&1].containers.counter;
                inbox.push(ToSim::Packet(1, PlayIn::ContainerButtonClick { container_id: id, button_id: i32_of(&step["button"]) }));
            }
            "release_use" => inbox.push(ToSim::Packet(1, PlayIn::PlayerAction { action: 6, pos: [0, 0, 0], face: 0, sequence: 1 })),
            "set_command_block" => {
                use kiln_proto::packets::serverbound::{CommandBlockMode, CommandBlockUpdate};
                inbox.push(ToSim::Packet(
                    1,
                    PlayIn::SetCommandBlock(Box::new(CommandBlockUpdate {
                        pos: arr3(&step["pos"]),
                        command: step["command"].as_str().unwrap().to_owned(),
                        mode: match step["mode"].as_str().unwrap() {
                            "sequence" => CommandBlockMode::Sequence,
                            "auto" => CommandBlockMode::Auto,
                            _ => CommandBlockMode::Redstone,
                        },
                        track_output: step["track"].as_bool().unwrap(),
                        conditional: step["conditional"].as_bool().unwrap(),
                        automatic: step["auto"].as_bool().unwrap(),
                    })),
                ));
            }
            "menu_slot_state" => {
                let id = sim.players[&1].containers.counter;
                inbox.push(ToSim::Packet(1, PlayIn::ContainerSlotStateChanged { slot: i32_of(&step["slot"]), container_id: id, enabled: step["enabled"].as_bool().unwrap() }));
            }
            "menu_click" => {
                let m = sim.players[&1].open_menu.as_ref().unwrap_or(&sim.players[&1].menu);
                let input = [
                    kiln_inventory::ContainerInput::Pickup,
                    kiln_inventory::ContainerInput::QuickMove,
                    kiln_inventory::ContainerInput::Swap,
                    kiln_inventory::ContainerInput::Clone,
                    kiln_inventory::ContainerInput::Throw,
                    kiln_inventory::ContainerInput::QuickCraft,
                    kiln_inventory::ContainerInput::PickupAll,
                ][i32_of(&step["input"]) as usize];
                let click = kiln_inventory::ContainerClick {
                    container_id: m.container_id,
                    state_id: m.state_id(),
                    slot: i32_of(&step["slot"]) as i16,
                    button: i32_of(&step["button"]) as i8,
                    input,
                    changed: Vec::new(),
                    carried: kiln_item::HashedStack::Empty,
                };
                let mut body = BytesMut::new();
                click.write(&mut body);
                inbox.push(ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() }));
            }
            "menu_close" | "menu_close_tick" => {
                let id = sim.players[&1].containers.counter;
                inbox.push(ToSim::Packet(1, PlayIn::ContainerClose { container_id: id }));
            }
            "dig" => {
                // (The recorded player stands on the ground; this one has had ticks to fall in.)
                sim.players.get_mut(&1).unwrap().on_ground = true;
                inbox.push(ToSim::Packet(1, PlayIn::PlayerAction { action: 0, pos: arr3(&step["pos"]), face: 1, sequence: 1 }));
            }
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
        let mut got_packets: Vec<String> = take_packets(&stats, line["menus"].as_bool() == Some(true), maps).iter().map(|v| v.to_string()).collect();
        if std::env::var_os("KILN_TRACE_PACKETS").is_some() {
            eprintln!("TRACE {} step {n}: {got_packets:?}", line["name"]);
        }
        let mut want_packets: Vec<String> = want["packets"].as_array().unwrap().iter().map(|v| normalize_want(v).to_string()).collect();
        // Vanilla sends two or more changes of one section as a Section Blocks Update, which the
        // vectors do not record (Kiln sends each change on its own).
        if step["op"] == "command" && got_packets.iter().filter(|p| p.contains("\"t\":\"block_update\"")).count() >= 2 {
            got_packets.retain(|p| !p.contains("\"t\":\"block_update\""));
        }
        // (The attack sound is the cooldown's: this level does not tick between the vanilla steps.)
        got_packets.retain(|p| !p.contains("entity.player.attack."));
        want_packets.retain(|p| !p.contains("entity.player.attack."));
        // (Kiln's mobs tick between the steps and make their idle noises; the vectors' level does not tick them.)
        if line["ticking"].as_bool() == Some(true) {
            got_packets.retain(|p| !p.contains("\"source\":\"hostile\""));
            want_packets.retain(|p| !p.contains("\"source\":\"hostile\""));
            // (Kiln sends what a packet changed before its block entities tick, vanilla at the tick's end: a block entity with nothing yet to tell,
            // and the same update twice, are not part of the comparison.)
            got_packets.retain(|p| !p.contains("\"tag\":\"0a00\""));
            // (A config of the datapack puts its mobs around the spawner at random: where is not compared.)
            if line["name"].as_str() == Some("trial_key_config") {
                got_packets.retain(|p| !p.contains("\"event\":3012"));
                want_packets.retain(|p| !p.contains("\"event\":3012"));
            }
            want_packets.retain(|p| !p.contains("\"tag\":\"0a00\""));
            got_packets.sort();
            got_packets.dedup();
            want_packets.sort();
            want_packets.dedup();
            got_packets.sort();
        }
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
            // (A hive's bees age with the ticks Kiln's level makes between the steps; the recorded level stands still.)
            let (got, expected) = (got.map(no_hive_ticks).map(no_mob_uuids), expected.map(no_hive_ticks).map(no_mob_uuids));
            eq(&format!("block entity {at:?}"), format!("{got:?}"), format!("{expected:?}"));
        }
        let mut got_items: Vec<String> = sim.item_stacks().iter().filter(|s| line["mobs"].as_bool() != Some(true) || s.item_name() != "minecraft:rotten_flesh").map(stack_hex).collect();
        got_items.sort();
        let want_items: Vec<String> = want["entities"].as_array().unwrap().iter().map(|e| e["item"].as_str().unwrap().to_owned()).collect();
        eq("item entities", format!("{got_items:?}"), format!("{want_items:?}"));
        let p = &sim.players[&1];
        if want.get("stands").is_some() {
            let diff = stand_diff(&stand_rows(&sim), &want_stand_rows(&want["stands"]));
            eq("armor stands", diff, String::new());
        }
        if let Some(want_bees) = want.get("bees_new") {
            // The bees that appeared in this step (where, and whether they have a target).
            let mut fresh: Vec<[f64; 4]> = Vec::new();
            for region in sim.dims[crate::OVERWORLD_ID].regions.iter() {
                for e in region.part().0.list.iter().filter(|e| !e.removed && e.kind.name == "minecraft:bee") {
                    if seen_bees.insert(e.id) {
                        let target = e.phys.as_deref().and_then(kiln_entity::mob::data).is_some_and(|m| m.target.is_some());
                        fresh.push([e.pos[0], e.pos[1], e.pos[2], f64::from(u8::from(target))]);
                    }
                }
            }
            fresh.sort_by(|a, b| a.partial_cmp(b).unwrap());
            // (The flag is an integer in the vectors.)
            let rows: Vec<Value> = fresh.iter().map(|r| json!([r[0], r[1], r[2], r[3] as i64])).collect();
            eq("new bees", Value::Array(rows).to_string(), want_bees.to_string());
        }
        if let Some(want_mobs) = want.get("mobs") {
            let mut got: Vec<String> = mob_rows(&sim).iter().map(|r| format!("{r:?}")).collect();
            let mut want_rows: Vec<String> = want_mobs.as_array().unwrap().iter().map(|r| format!("{:?}", (r[0].as_str().unwrap().to_owned(), r[1].as_f64().unwrap(), r[2].as_f64().unwrap(), r[3].as_f64().unwrap()))).collect();
            got.sort();
            want_rows.sort();
            eq("mobs", format!("{got:?}"), format!("{want_rows:?}"));
        }
        if let Some(want_maps) = want.get("maps") {
            eq("maps", maps_json(&sim).to_string(), want_maps.to_string());
        }
        if want.get("hangings").is_some() {
            eq("hanging entities", hangings_json(&sim).to_string(), want["hangings"].to_string());
        }
        if let Some(f) = want["food"].as_array() {
            eq("food", format!("{:?}", (p.food, p.saturation, p.exhaustion)), format!("{:?}", (f[0].as_i64().unwrap() as i32, f[1].as_f64().unwrap() as f32, f[2].as_f64().unwrap() as f32)));
        }
        if let Some(cs) = want["custom"].as_object() {
            for (name, want_count) in cs {
                let stat = crate::player_stats::Stat::custom(name).unwrap_or_else(|| panic!("stat {name}"));
                eq(&format!("custom {name}"), p.stats.get(stat).to_string(), want_count.to_string());
            }
        }
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
