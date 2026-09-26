//! Writes entity packet test vectors for `tools/entity_vectors.py`, which decodes them with
//! vanilla's codecs: `<name>.bin` (packet body without the id), `<name>.expect` (decoded field
//! values, `path=value` exact or `path~value` approximate), `manifest.txt`, and the generated
//! entity tables (`tables.txt`) to compare against the jar at runtime.
//!
//! usage: cargo run -p kiln-proto --example entity_vectors -- <out dir>

use bytes::Bytes;
use kiln_data::entities::{self, DataField, data, pose, serializer as s, types};
use kiln_proto::Reader;
use kiln_proto::nbt::{Tag, text};
use kiln_proto::packets::ProfileProperty;
use kiln_proto::packets::entity::metadata::{
    Direction, GlobalPos, HumanoidArm, ItemStack, Particle, model_parts, shared_flags,
};
use kiln_proto::packets::entity::*;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const GAME: &str = "net.minecraft.network.protocol.game.";
const KEY_DER: &str = "30819f300d06092a864886f70d010101050003818d0030818902818100934d50dc66bed22100db4c4384d3e721\
ded02625dbfc25d65a24b584f251841b4ce9ba839c902f8b2b61606b6a157db432434600dce5f394f32bbc2a78c6f31888755982c77be38374dc\
83fd950326384f452615fd7bc69c2b0cedc81b9f8e6ac20fc0f19f24c9a0b7b5670370ae8ca5074bc8676895352cfae01f848962345d02030100\
01";

/// (serializer, value, [(field path suffix, expected decoded value)]).
type Case = (i32, DataValue, Vec<(&'static str, String)>);

struct Vectors {
    dir: PathBuf,
    manifest: String,
}

/// Expected decoded value: exact (`=`) or within 1e-3 (`~`).
#[derive(Clone)]
enum E {
    Is(String, String),
    Near(String, f64),
}

fn is(path: &str, v: impl ToString) -> E {
    E::Is(path.into(), v.to_string())
}

fn near(path: &str, v: f64) -> E {
    E::Near(path.into(), v)
}

fn vec3(path: &str, v: [f64; 3]) -> Vec<E> {
    vec![is(&format!("{path}.x"), f(v[0])), is(&format!("{path}.y"), f(v[1])), is(&format!("{path}.z"), f(v[2]))]
}

fn f(v: impl std::fmt::Debug) -> String {
    format!("{v:?}")
}

impl Vectors {
    fn add(&mut self, name: &str, class: &str, packet_name: &str, packet: Bytes, expect: Vec<E>) {
        self.add_flagged(name, class, packet_name, packet, expect, "");
    }

    /// `packet` is id + body; the manifest records the id for checking against packets.json.
    /// `flag` "items": the vector holds item stacks, which vanilla only decodes once default
    /// item components are bound (VanillaDump binds empty ones; vanilla_decode.py cannot).
    fn add_flagged(&mut self, name: &str, class: &str, packet_name: &str, packet: Bytes, expect: Vec<E>, flag: &str) {
        let mut r = Reader::new(&packet);
        let id = r.varint().unwrap();
        fs::write(self.dir.join(format!("{name}.bin")), r.rest()).unwrap();
        let mut text = String::new();
        for e in expect {
            match e {
                E::Is(p, v) => writeln!(text, "{p}={v}"),
                E::Near(p, v) => writeln!(text, "{p}~{v:?}"),
            }
            .unwrap();
        }
        fs::write(self.dir.join(format!("{name}.expect")), text).unwrap();
        let class = if class.contains('.') { class.to_string() } else { format!("{GAME}{class}") };
        writeln!(self.manifest, "{name} {class} minecraft:{packet_name} {id} {flag}").unwrap();
    }
}

fn builtin(registry: &str, entry: &str) -> i32 {
    kiln_data::builtin_id(registry, entry).unwrap_or_else(|| panic!("{entry} not in {registry}"))
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: entity_vectors <out dir>"));
    fs::create_dir_all(&dir).unwrap();
    let mut v = Vectors { dir: dir.clone(), manifest: String::new() };
    let uuid = Uuid::from_u128(0x5f4d_1c2a_9b3e_4f60_8a7d_2e1c_0b9a_8f7e);
    let uuid2 = Uuid::from_u128(0x0000_0001_0002_4003_8004_0000_0000_0005);

    // ---- add_entity / remove_entities -------------------------------------------------------
    let (pitch, yaw, head) = (Angle::from_degrees(-30.0), Angle::from_degrees(135.0), Angle::from_degrees(-170.0));
    let spawn = AddEntity {
        entity_id: 42,
        uuid,
        kind: types::PLAYER.id,
        pos: [8.5, 65.0, -12.25],
        velocity: [0.25, -0.0784, 1.5],
        pitch,
        yaw,
        head_yaw: head,
        data: 0,
    };
    let mut e = vec![is("id", 42), is("uuid", uuid), is("type", "minecraft:player")];
    e.extend([is("x", 8.5), is("y", 65.0), is("z", -12.25)]);
    e.extend([near("movement.x", 0.25), near("movement.y", -0.0784), near("movement.z", 1.5)]);
    e.extend([is("xRot", pitch.0), is("yRot", yaw.0), is("yHeadRot", head.0), is("data", 0)]);
    v.add("add_entity_player", "ClientboundAddEntityPacket", "add_entity", add_entity(&spawn), e);

    let item = AddEntity { entity_id: 70000, kind: types::ITEM.id, velocity: [5.5, -0.5, 100.0], data: 1, ..spawn };
    let e = vec![is("id", 70000), is("type", "minecraft:item"), near("movement.x", 5.5), near("movement.y", -0.5)];
    let e = [e, vec![near("movement.z", 100.0), is("data", 1)]].concat();
    v.add("add_entity_fast_item", "ClientboundAddEntityPacket", "add_entity", add_entity(&item), e);

    let still = AddEntity { velocity: [0.0; 3], ..spawn };
    let e = vec3("movement", [0.0; 3]);
    v.add("add_entity_still", "ClientboundAddEntityPacket", "add_entity", add_entity(&still), e);

    let e = vec![is("entityIds[0]", 1), is("entityIds[1]", 300), is("entityIds[2]", 70000)];
    v.add(
        "remove_entities",
        "ClientboundRemoveEntitiesPacket",
        "remove_entities",
        remove_entities(&[1, 300, 70000]),
        e,
    );

    // ---- relative moves --------------------------------------------------------------------
    let p = move_entity_pos(7, &PosDelta::Linear([1024, -512, 32767]), true);
    let e = vec![is("entityId", 7), is("delta.xa", 1024), is("delta.ya", -512), is("delta.za", 32767)];
    let e = [e, vec![is("onGround", true), is("hasPos", true), is("hasRot", false)]].concat();
    v.add("move_entity_pos", "ClientboundMoveEntityPacket$Pos", "move_entity_pos", p, e);

    let stepped = PosDelta::Stepped(vec![([1, 2, 3], 1), ([-4, 5, -32768], 2)]);
    let p = move_entity_pos(7, &stepped, false);
    let mut e = vec![is("delta.steps[0].xa", 1), is("delta.steps[0].ya", 2), is("delta.steps[0].za", 3)];
    e.extend([is("delta.steps[0].ticks", 1), is("delta.steps[1].xa", -4), is("delta.steps[1].za", -32768)]);
    e.extend([is("delta.steps[1].ticks", 2), is("onGround", false)]);
    v.add("move_entity_pos_stepped", "ClientboundMoveEntityPacket$Pos", "move_entity_pos", p, e);

    let p = move_entity_pos_rot(8, &PosDelta::Linear([-1, 0, 1]), Angle(64), Angle(-32), false);
    let e = vec![is("entityId", 8), is("delta.xa", -1), is("delta.za", 1), is("yRot", 64), is("xRot", -32)];
    let e = [e, vec![is("onGround", false), is("hasPos", true), is("hasRot", true)]].concat();
    v.add("move_entity_pos_rot", "ClientboundMoveEntityPacket$PosRot", "move_entity_pos_rot", p, e);

    let p = move_entity_rot(8, Angle(100), Angle(-10), true);
    let e = vec![is("entityId", 8), is("yRot", 100), is("xRot", -10), is("onGround", true), is("hasPos", false)];
    v.add("move_entity_rot", "ClientboundMoveEntityPacket$Rot", "move_entity_rot", p, e);

    let e = vec![is("entityId", 8), is("yHeadRot", -100)];
    v.add("rotate_head", "ClientboundRotateHeadPacket", "rotate_head", rotate_head(8, Angle(-100)), e);

    // ---- absolute positions ----------------------------------------------------------------
    let p = entity_position_sync(9, &PositionPath::Linear([100.125, 70.0, -3.5]), 45.5, -12.25, true);
    let mut e = vec3("position.endPosition", [100.125, 70.0, -3.5]);
    e.extend([is("id", 9), is("yRot", 45.5), is("xRot", -12.25), is("onGround", true)]);
    v.add("entity_position_sync", "ClientboundEntityPositionSyncPacket", "entity_position_sync", p, e);

    let steps = [([1.0, 2.0, 3.0], 1), ([4.0, 5.0, 6.5], 3)];
    let p = entity_position_sync(9, &PositionPath::Stepped(&steps), 10.0, 20.0, false);
    let mut e = vec3("position.steps[0].position", [1.0, 2.0, 3.0]);
    e.extend(vec3("position.steps[1].position", [4.0, 5.0, 6.5]));
    e.extend([is("position.steps[0].tickOffset", 1), is("position.steps[1].tickOffset", 3)]);
    e.extend([is("yRot", 10.0), is("xRot", 20.0), is("onGround", false)]);
    v.add("entity_position_sync_stepped", "ClientboundEntityPositionSyncPacket", "entity_position_sync", p, e);

    let rel = Relative::X | Relative::Y_ROT | Relative::DELTA_Z;
    let p = teleport_entity(11, [1.5, 2.5, 3.5], [0.1, 0.2, 0.3], 90.0, 45.0, rel, true);
    let mut e = vec3("change.position", [1.5, 2.5, 3.5]);
    e.extend(vec3("change.deltaMovement", [0.1, 0.2, 0.3]));
    e.extend([is("id", 11), is("change.yRot", 90.0), is("change.xRot", 45.0), is("onGround", true)]);
    e.push(is("relatives", "[DELTA_Z, X, Y_ROT]"));
    v.add("teleport_entity", "ClientboundTeleportEntityPacket", "teleport_entity", p, e);

    let p = teleport_entity(11, [0.0; 3], [0.0; 3], 0.0, 0.0, Relative::ABSOLUTE, false);
    let e = vec![is("relatives", "[]"), is("onGround", false)];
    v.add("teleport_entity_absolute", "ClientboundTeleportEntityPacket", "teleport_entity", p, e);

    let p = set_entity_motion(5, [-0.3, 0.42, 0.0]);
    let e = vec![is("id", 5), near("movement.x", -0.3), near("movement.y", 0.42), is("movement.z", 0.0)];
    v.add("set_entity_motion", "ClientboundSetEntityMotionPacket", "set_entity_motion", p, e);

    // ---- entity data -----------------------------------------------------------------------
    let flame = Particle { kind: builtin("minecraft:particle_type", "minecraft:flame"), options: vec![] };
    let mut d = EntityData::new();
    d.set(data::entity::SHARED_FLAGS, &DataValue::Byte((shared_flags::CROUCHING | shared_flags::SPRINTING) as i8))
        .set(data::entity::CUSTOM_NAME, &DataValue::OptionalComponent(Some(text("Steve"))))
        .set(data::entity::POSE, &DataValue::Pose(pose::CROUCHING))
        .set(data::living_entity::HEALTH, &DataValue::Float(20.0))
        .set(data::living_entity::EFFECT_PARTICLES, &DataValue::Particles(vec![flame.clone()]))
        .set(data::living_entity::SLEEPING_POS, &DataValue::OptionalBlockPos(Some([-5, 64, 1000])))
        .set(data::avatar::PLAYER_MAIN_HAND, &DataValue::HumanoidArm(HumanoidArm::Left))
        .set(data::avatar::PLAYER_MODE_CUSTOMISATION, &DataValue::Byte(model_parts::ALL as i8))
        .set(data::player::PLAYER_ABSORPTION, &DataValue::Float(4.0))
        .set(data::player::SHOULDER_PARROT_LEFT, &DataValue::OptionalUnsignedInt(Some(2)))
        .set(data::player::SHOULDER_PARROT_RIGHT, &DataValue::OptionalUnsignedInt(None));
    let values = [
        (data::entity::SHARED_FLAGS, "10"),
        (data::entity::CUSTOM_NAME, "Steve"),
        (data::entity::POSE, "CROUCHING"),
        (data::living_entity::HEALTH, "20.0"),
        (data::living_entity::EFFECT_PARTICLES, ""),
        (data::living_entity::SLEEPING_POS, ""),
        (data::avatar::PLAYER_MAIN_HAND, "LEFT"),
        (data::avatar::PLAYER_MODE_CUSTOMISATION, "127"),
        (data::player::PLAYER_ABSORPTION, "4.0"),
        (data::player::SHOULDER_PARROT_LEFT, "2"),
        (data::player::SHOULDER_PARROT_RIGHT, "empty"),
    ];
    let mut e = vec![is("id", 42)];
    for (i, (field, value)) in values.iter().enumerate() {
        e.push(is(&format!("packedItems[{i}].id"), field.index));
        e.push(is(&format!("packedItems[{i}].serializer"), field.serializer));
        if !value.is_empty() {
            e.push(is(&format!("packedItems[{i}].value"), value));
        }
    }
    e.extend([is("packedItems[4].value[0]", "minecraft:flame"), is("packedItems[5].value.x", -5)]);
    e.extend([is("packedItems[5].value.y", 64), is("packedItems[5].value.z", 1000)]);
    v.add("set_entity_data_player", "ClientboundSetEntityDataPacket", "set_entity_data", set_entity_data(42, &d), e);

    // Every implemented serializer except the data-driven registry holders (cat/cow/wolf/frog/
    // pig/chicken/zombie-nautilus variants, painting variant), which need registries the static
    // decoder does not have. Indices are arbitrary: the packet decodes by serializer id alone.
    // Key order as vanilla writes it, so re-encoding reproduces the bytes (order is not semantic).
    let red =
        Tag::Compound(vec![("color".into(), Tag::String("red".into())), ("text".into(), Tag::String("Hi".into()))]);
    let dirt = kiln_data::blocks::default_state::DIRT as i32;
    let cases: Vec<Case> = vec![
        (s::BYTE, DataValue::Byte(-5), vec![("", "-5".into())]),
        (s::INT, DataValue::Int(-123456), vec![("", "-123456".into())]),
        (s::LONG, DataValue::Long(1 << 40), vec![("", "1099511627776".into())]),
        (s::FLOAT, DataValue::Float(0.5), vec![("", "0.5".into())]),
        (s::STRING, DataValue::String("hello world".into()), vec![("", "hello world".into())]),
        (s::COMPONENT, DataValue::Component(red), vec![("", "Hi".into()), (".color", "red".into())]),
        (s::OPTIONAL_COMPONENT, DataValue::OptionalComponent(None), vec![("", "empty".into())]),
        (s::BOOLEAN, DataValue::Boolean(true), vec![("", "true".into())]),
        (s::ROTATIONS, DataValue::Rotations([1.0, -2.5, 359.5]), vec![(".x", "1.0".into()), (".z", "359.5".into())]),
        (
            s::BLOCK_POS,
            DataValue::BlockPos([-29999984, -64, 29999984]),
            vec![(".x", "-29999984".into()), (".y", "-64".into()), (".z", "29999984".into())],
        ),
        (s::OPTIONAL_BLOCK_POS, DataValue::OptionalBlockPos(None), vec![("", "empty".into())]),
        (s::DIRECTION, DataValue::Direction(Direction::East), vec![("", "EAST".into())]),
        (s::DIRECTION, DataValue::Direction(Direction::Down), vec![("", "DOWN".into())]),
        (
            s::OPTIONAL_LIVING_ENTITY_REFERENCE,
            DataValue::OptionalEntityReference(Some(uuid)),
            vec![("", uuid.to_string())],
        ),
        (s::OPTIONAL_LIVING_ENTITY_REFERENCE, DataValue::OptionalEntityReference(None), vec![("", "empty".into())]),
        (s::BLOCK_STATE, DataValue::BlockState(1), vec![("", "Block{minecraft:stone}".into())]),
        (
            s::OPTIONAL_BLOCK_STATE,
            DataValue::OptionalBlockState(Some(dirt)),
            vec![("", "Block{minecraft:dirt}".into())],
        ),
        (s::OPTIONAL_BLOCK_STATE, DataValue::OptionalBlockState(None), vec![("", "empty".into())]),
        (s::PARTICLE, DataValue::Particle(flame.clone()), vec![("", "minecraft:flame".into())]),
        (s::PARTICLES, DataValue::Particles(vec![]), vec![("", "[]".into())]),
        (
            s::VILLAGER_DATA,
            DataValue::VillagerData {
                kind: builtin("minecraft:villager_type", "minecraft:plains"),
                profession: builtin("minecraft:villager_profession", "minecraft:librarian"),
                level: 3,
            },
            vec![
                (".type", "minecraft:plains".into()),
                (".profession", "minecraft:librarian".into()),
                (".level", "3".into()),
            ],
        ),
        (s::OPTIONAL_UNSIGNED_INT, DataValue::OptionalUnsignedInt(Some(0)), vec![("", "0".into())]),
        (s::POSE, DataValue::Pose(pose::SLEEPING), vec![("", "SLEEPING".into())]),
        (
            s::OPTIONAL_GLOBAL_POS,
            DataValue::OptionalGlobalPos(Some(GlobalPos { dimension: "minecraft:the_nether".into(), pos: [1, 2, 3] })),
            vec![(".dimension", "minecraft:the_nether".into()), (".pos.x", "1".into()), (".pos.z", "3".into())],
        ),
        (
            s::VECTOR3,
            DataValue::Vector3([0.5, 1.5, -2.0]),
            vec![(".x", "0.5".into()), (".y", "1.5".into()), (".z", "-2.0".into())],
        ),
        (
            s::QUATERNION,
            DataValue::Quaternion([0.0, 0.70710677, 0.0, 0.70710677]),
            vec![(".x", "0.0".into()), (".y", "0.70710677".into()), (".w", "0.70710677".into())],
        ),
        (s::HUMANOID_ARM, DataValue::HumanoidArm(HumanoidArm::Right), vec![("", "RIGHT".into())]),
        (s::SNIFFER_STATE, DataValue::Enum(2), vec![("", "SCENTING".into())]),
        (s::ARMADILLO_STATE, DataValue::Enum(1), vec![("", "ROLLING".into())]),
        (s::COPPER_GOLEM_STATE, DataValue::Enum(1), vec![("", "GETTING_ITEM".into())]),
        (s::WEATHERING_COPPER_STATE, DataValue::Enum(3), vec![("", "OXIDIZED".into())]),
        (s::DYE_COLOR, DataValue::Enum(14), vec![("", "RED".into())]),
    ];
    let mut d = EntityData::new();
    let mut e = vec![is("id", 1)];
    for (i, (serializer, value, checks)) in cases.iter().enumerate() {
        d.set(DataField { index: i as u8, serializer: *serializer }, value);
        e.push(is(&format!("packedItems[{i}].serializer"), serializer));
        for (suffix, want) in checks {
            e.push(is(&format!("packedItems[{i}].value{suffix}"), want));
        }
    }
    v.add("set_entity_data_all", "ClientboundSetEntityDataPacket", "set_entity_data", set_entity_data(1, &d), e);

    let sword = builtin("minecraft:item", "minecraft:diamond_sword");
    let mut d = EntityData::new();
    d.set(data::item_entity::ITEM, &DataValue::ItemStack(Some(ItemStack { item: sword, count: 1 })))
        .set(
            DataField { index: 0, serializer: s::ITEM_STACK },
            &DataValue::ItemStack(Some(ItemStack { item: 1, count: 64 })),
        )
        .set(DataField { index: 1, serializer: s::ITEM_STACK }, &DataValue::ItemStack(None));
    let e = vec![
        is("packedItems[0].id", data::item_entity::ITEM.index),
        is("packedItems[0].value", "1 minecraft:diamond_sword"),
    ];
    let e = [e, vec![is("packedItems[1].value", "64 minecraft:stone"), is("packedItems[2].value", "empty")]].concat();
    let p = set_entity_data(2, &d);
    v.add_flagged("set_entity_data_items", "ClientboundSetEntityDataPacket", "set_entity_data", p, e, "items");

    // ---- animations --------------------------------------------------------------------------
    let e = vec![is("id", 5), is("action", 1)];
    v.add("animate", "ClientboundAnimatePacket", "animate", animate(5, animation::CRITICAL_HIT), e);
    let e = vec![is("entityId", 5), is("hand", "OFF_HAND"), is("animation.type", "STAB"), is("animation.duration", 10)];
    v.add(
        "swing_animation",
        "ClientboundSwingAnimationPacket",
        "swing_animation",
        swing_animation(5, true, swing::STAB, 10),
        e,
    );
    let p = swing_animation(6, false, swing::WHACK, swing::DEFAULT_DURATION);
    let e = vec![is("hand", "MAIN_HAND"), is("animation.type", "WHACK"), is("animation.duration", 6)];
    v.add("swing_animation_default", "ClientboundSwingAnimationPacket", "swing_animation", p, e);

    // ---- player info -----------------------------------------------------------------------
    let key: Vec<u8> =
        (0..KEY_DER.len()).step_by(2).map(|i| u8::from_str_radix(&KEY_DER[i..i + 2], 16).unwrap()).collect();
    let props = [ProfileProperty { name: "textures", value: "dGV4dHVyZXM=", signature: Some("c2ln") }];
    let display = text("The Steve");
    let steve = PlayerInfoEntry {
        chat_session: Some(ChatSession {
            session_id: uuid2,
            expires_at: 1_800_000_000_000,
            public_key: &key,
            key_signature: &[1, 2, 3, 4],
        }),
        latency: 57,
        display_name: Some(&display),
        list_order: 3,
        show_hat: false,
        ..PlayerInfoEntry::new(uuid, "Steve", &props, 1)
    };
    let alex = PlayerInfoEntry { listed: false, ..PlayerInfoEntry::new(uuid2, "Alex", &[], 3) };
    let p = player_info_update(PlayerInfoActions::INITIALIZE, &[steve, alex]);
    let mut actions = [
        "ADD_PLAYER",
        "INITIALIZE_CHAT",
        "UPDATE_GAME_MODE",
        "UPDATE_LISTED",
        "UPDATE_LATENCY",
        "UPDATE_DISPLAY_NAME",
        "UPDATE_LIST_ORDER",
        "UPDATE_HAT",
    ];
    actions.sort();
    let mut e = vec![is("actions", format!("[{}]", actions.join(", ")))];
    e.extend([
        is("entries[0].profileId", uuid),
        is("entries[0].profile.id", uuid),
        is("entries[0].profile.name", "Steve"),
    ]);
    e.extend([is("entries[0].profile.properties[0].name", "textures")]);
    e.extend([is("entries[0].profile.properties[0].value", "dGV4dHVyZXM=")]);
    e.extend([is("entries[0].profile.properties[0].signature", "c2ln")]);
    e.extend([is("entries[0].listed", true), is("entries[0].latency", 57), is("entries[0].gameMode", "CREATIVE")]);
    e.extend([
        is("entries[0].displayName", "The Steve"),
        is("entries[0].showHat", false),
        is("entries[0].listOrder", 3),
    ]);
    e.extend([is("entries[0].chatSession.sessionId", uuid2)]);
    e.extend([is("entries[0].chatSession.profilePublicKey.expiresAt", 1_800_000_000_000i64)]);
    e.extend([is("entries[0].chatSession.profilePublicKey.key", KEY_DER)]);
    e.extend([is("entries[0].chatSession.profilePublicKey.keySignature", "01020304")]);
    e.extend([is("entries[1].profile.name", "Alex"), is("entries[1].profile.properties", "[]")]);
    e.extend([is("entries[1].listed", false), is("entries[1].gameMode", "SPECTATOR"), is("entries[1].showHat", true)]);
    e.extend([is("entries[1].displayName", "null"), is("entries[1].chatSession", "null")]);
    v.add("player_info_update_init", "ClientboundPlayerInfoUpdatePacket", "player_info_update", p, e);

    let entry = PlayerInfoEntry { latency: 250, ..PlayerInfoEntry::new(uuid, "", &[], 0) };
    let actions =
        PlayerInfoActions::UPDATE_LISTED | PlayerInfoActions::UPDATE_LATENCY | PlayerInfoActions::UPDATE_DISPLAY_NAME;
    let p = player_info_update(actions, &[entry]);
    let e =
        vec![is("actions", "[UPDATE_DISPLAY_NAME, UPDATE_LATENCY, UPDATE_LISTED]"), is("entries[0].profileId", uuid)];
    let e =
        [e, vec![is("entries[0].latency", 250), is("entries[0].listed", true), is("entries[0].displayName", "null")]]
            .concat();
    v.add("player_info_update_partial", "ClientboundPlayerInfoUpdatePacket", "player_info_update", p, e);

    let e = vec![is("profileIds[0]", uuid), is("profileIds[1]", uuid2)];
    v.add(
        "player_info_remove",
        "ClientboundPlayerInfoRemovePacket",
        "player_info_remove",
        player_info_remove(&[uuid, uuid2]),
        e,
    );

    // ---- a tracker-driven sequence ---------------------------------------------------------
    let start = MoveState { pos: [0.5, 64.0, 0.5], yaw: 0.0, pitch: 0.0, head_yaw: 0.0, on_ground: true };
    let mut tracker = MovementTracker::new(99, types::PLAYER.update_interval, &start);
    let spawned = tracker.spawn(uuid, types::PLAYER.id, [0.0; 3], 0);
    let e = vec![is("id", 99), is("type", "minecraft:player"), is("x", 0.5), is("y", 64.0), is("z", 0.5)];
    v.add("tracker_spawn", "ClientboundAddEntityPacket", "add_entity", spawned, e);
    tracker.tick(&start);
    tracker.tick(&start);
    let moved = tracker.tick(&MoveState { pos: [1.0, 64.0, 0.25], yaw: 90.0, head_yaw: 90.0, ..start });
    let e = vec![is("entityId", 99), is("delta.xa", 2048), is("delta.ya", 0), is("delta.za", -1024), is("yRot", 64)];
    v.add("tracker_move", "ClientboundMoveEntityPacket$PosRot", "move_entity_pos_rot", moved[0].clone(), e);
    let e = vec![is("entityId", 99), is("yHeadRot", 64)];
    v.add("tracker_head", "ClientboundRotateHeadPacket", "rotate_head", moved[1].clone(), e);

    fs::write(dir.join("manifest.txt"), &v.manifest).unwrap();
    fs::write(dir.join("tables.txt"), tables()).unwrap();
    println!("wrote {} vectors to {}", v.manifest.lines().count(), dir.display());
}

/// The generated entity tables, one fact per line, in the format `VanillaDump entities` prints.
fn tables() -> String {
    let mut out = String::new();
    for (i, name) in s::NAMES.iter().enumerate() {
        writeln!(out, "serializer {} {i}", name.to_ascii_uppercase()).unwrap();
    }
    for t in entities::TYPES {
        writeln!(
            out,
            "type {} {} {:?} {:?} {:?} {} {} {}",
            t.name, t.id, t.width, t.height, t.eye_height, t.tracking_range, t.update_interval, t.track_deltas
        )
        .unwrap();
        writeln!(out, "typeclass {} {}", t.name, entities::CLASSES[t.class as usize].name).unwrap();
    }
    for c in entities::CLASSES {
        for (field, d) in c.fields {
            writeln!(out, "field {} {field} {} {}", c.name, d.index, d.serializer).unwrap();
        }
    }
    out
}
