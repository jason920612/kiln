//! Serverbound decoders against bytes written by vanilla's own codecs
//! (`testdata/serverbound.txt`, from `tools/VanillaServerboundVectors.java`; the values here
//! are the ones that tool encodes).

use kiln_data::packets as ids;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::common::{self, CookieResponse, ResourcePackAction};
use kiln_proto::packets::serverbound::*;
use kiln_proto::packets::{PlayIn, decode_play};
use kiln_proto::{DecodeError, Reader};
use std::collections::BTreeMap;
use uuid::Uuid;

/// (state, packet, case) -> body.
fn vectors() -> BTreeMap<(String, String, String), Vec<u8>> {
    include_str!("testdata/serverbound.txt")
        .lines()
        .map(|line| {
            let f: Vec<&str> = line.split(' ').collect();
            let hex = f.get(3).copied().unwrap_or("");
            let body = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
            ((f[0].to_string(), f[1].to_string(), f[2].to_string()), body)
        })
        .collect()
}

fn play_id(packet: &str) -> i32 {
    let name = format!("minecraft:{packet}");
    ids::play::serverbound::NAMES.iter().position(|n| *n == name).unwrap_or_else(|| panic!("no play packet {name}")) as i32
}

const POS: [i32; 3] = [-123, -45, 6789];
const PACK: Uuid = Uuid::from_u128(0xfedcba98_7654_4321_8fed_cba987654321);

fn expected_play() -> Vec<(&'static str, &'static str, PlayIn)> {
    use PlayIn as P;
    let s = |v: &str| v.to_string();
    vec![
        ("client_command", "respawn", P::ClientCommand(ClientCommand::PerformRespawn)),
        ("client_command", "stats", P::ClientCommand(ClientCommand::RequestStats)),
        ("client_command", "game_rules", P::ClientCommand(ClientCommand::RequestGameRuleValues)),
        ("attack", "attack", P::Attack { entity_id: 70000 }),
        ("use_item", "use_item", P::UseItem { hand: Hand::Off, sequence: 42, yaw: 90.5, pitch: -12.25 }),
        ("container_close", "close", P::ContainerClose { container_id: 3 }),
        ("container_button_click", "click", P::ContainerButtonClick { container_id: 2, button_id: 5 }),
        ("container_slot_state_changed", "toggle", P::ContainerSlotStateChanged { slot: 4, container_id: 9, enabled: true }),
        ("pick_item_from_block", "pick", P::PickItemFromBlock { pos: POS, include_data: true }),
        ("pick_item_from_entity", "pick", P::PickItemFromEntity { entity_id: 99, include_data: false }),
        (
            "sign_update",
            "front",
            P::SignUpdate { pos: POS, lines: Box::new([s("Hello"), s(""), s("\u{e9}\u{4e16}\u{754c}"), s("line 4")]), front: true },
        ),
        ("sign_update", "back", P::SignUpdate { pos: POS, lines: Box::new([s("a"), s("b"), s("c"), s("d")]), front: false }),
        (
            "set_command_block",
            "auto",
            P::SetCommandBlock(Box::new(CommandBlockUpdate {
                pos: POS,
                command: s("say hi"),
                mode: CommandBlockMode::Auto,
                track_output: true,
                conditional: false,
                automatic: true,
            })),
        ),
        (
            "set_command_block",
            "redstone",
            P::SetCommandBlock(Box::new(CommandBlockUpdate {
                pos: POS,
                command: s(""),
                mode: CommandBlockMode::Redstone,
                track_output: false,
                conditional: true,
                automatic: false,
            })),
        ),
        ("set_command_minecart", "minecart", P::SetCommandMinecart { entity_id: 12, command: s("time set day"), track_output: true }),
        (
            "set_structure_block",
            "save",
            P::SetStructureBlock(Box::new(StructureBlockUpdate {
                pos: POS,
                update_type: StructureUpdateType::SaveArea,
                mode: StructureMode::Save,
                name: s("kiln:house"),
                offset: [-3, 0, 48],
                size: [16, 8, 1],
                mirror: Mirror::FrontBack,
                rotation: Rotation::CounterClockwise90,
                metadata: s("meta"),
                integrity: 0.75,
                seed: -987654321,
                ignore_entities: true,
                show_air: true,
                show_bounding_box: false,
                strict: false,
            })),
        ),
        (
            "set_structure_block",
            "load_flags",
            P::SetStructureBlock(Box::new(StructureBlockUpdate {
                pos: POS,
                update_type: StructureUpdateType::LoadArea,
                mode: StructureMode::Load,
                name: s(""),
                offset: [0; 3],
                size: [0; 3],
                mirror: Mirror::None,
                rotation: Rotation::None,
                metadata: s(""),
                integrity: 1.0,
                seed: 0,
                ignore_entities: false,
                show_air: false,
                show_bounding_box: true,
                strict: true,
            })),
        ),
        (
            "set_jigsaw_block",
            "jigsaw",
            P::SetJigsawBlock(Box::new(JigsawBlockUpdate {
                pos: POS,
                name: s("minecraft:bottom"),
                target: s("kiln:top"),
                pool: s("minecraft:empty"),
                final_state: s("minecraft:stone"),
                rollable: true,
                selection_priority: 3,
                placement_priority: -1,
            })),
        ),
        (
            "set_jigsaw_block",
            "aligned",
            P::SetJigsawBlock(Box::new(JigsawBlockUpdate {
                pos: POS,
                name: s("minecraft:a"),
                target: s("minecraft:b"),
                pool: s("minecraft:c"),
                final_state: s(""),
                rollable: false,
                selection_priority: 0,
                placement_priority: 0,
            })),
        ),
        ("jigsaw_generate", "generate", P::JigsawGenerate { pos: POS, levels: 7, keep_jigsaws: true }),
        ("rename_item", "rename", P::RenameItem { name: s("Excalibur") }),
        ("select_trade", "trade", P::SelectTrade { offer: 4 }),
        (
            "set_beacon",
            "both",
            P::SetBeacon {
                primary: kiln_data::builtin_id("minecraft:mob_effect", "minecraft:speed"),
                secondary: kiln_data::builtin_id("minecraft:mob_effect", "minecraft:regeneration"),
            },
        ),
        ("set_beacon", "none", P::SetBeacon { primary: None, secondary: None }),
        ("edit_book", "signed", P::EditBook { slot: 40, pages: vec![s("page one"), s("page two")], title: Some(s("My Book")) }),
        ("edit_book", "unsigned", P::EditBook { slot: 0, pages: vec![], title: None }),
        ("player_abilities", "flying", P::PlayerAbilities { flying: true }),
        ("player_abilities", "landed", P::PlayerAbilities { flying: false }),
        ("client_tick_end", "tick_end", P::ClientTickEnd),
        ("paddle_boat", "left", P::PaddleBoat { left: true, right: false }),
        ("move_vehicle", "move", P::MoveVehicle { pos: [1.5, 62.0, -7.25], rot: [45.0, -10.0], on_ground: true }),
        ("change_difficulty", "hard", P::ChangeDifficulty { difficulty: 3 }),
        ("lock_difficulty", "lock", P::LockDifficulty { locked: true }),
        ("change_game_mode", "creative", P::ChangeGameMode { game_mode: 1 }),
        (
            "teleport_to_entity",
            "teleport",
            P::TeleportToEntity { target: Uuid::from_u128(0x069a79f4_44e9_4726_a5be_fca90e38aaf5) },
        ),
        ("spectator_action", "spectate", P::SpectatorAction { entity_id: Some(42) }),
        ("spectator_action", "stop", P::SpectatorAction { entity_id: None }),
        ("chat_ack", "ack", P::ChatAck { offset: 17 }),
        ("bundle_item_selected", "select", P::SelectBundleItem { slot: 36, index: 2 }),
        ("bundle_item_selected", "deselect", P::SelectBundleItem { slot: 36, index: -1 }),
        ("seen_advancements", "closed", P::SeenAdvancements { tab: None }),
        ("seen_advancements", "opened", P::SeenAdvancements { tab: Some(s("minecraft:story/root")) }),
        ("recipe_book_seen_recipe", "seen", P::RecipeBookSeenRecipe { recipe: 321 }),
        (
            "recipe_book_change_settings",
            "smoker",
            P::RecipeBookChangeSettings { book: RecipeBookType::Smoker, open: true, filtering: false },
        ),
        ("place_recipe", "place", P::PlaceRecipe { container_id: 1, recipe: 5, use_max_items: true }),
        ("block_entity_tag_query", "query", P::BlockEntityTagQuery { transaction: 8, pos: POS }),
        ("entity_tag_query", "query", P::EntityTagQuery { transaction: 9, entity_id: 1000 }),
        (
            "set_game_rule",
            "rules",
            P::SetGameRules {
                rules: vec![(s("minecraft:keep_inventory"), s("true")), (s("minecraft:random_tick_speed"), s("3"))],
            },
        ),
        ("punch", "punch", P::Punch),
        ("ping_request", "ping", P::PingRequest { time: 1_700_000_000_123 }),
        ("pong", "pong", P::Pong { id: -123456 }),
        ("resource_pack", "accepted", P::ResourcePack { id: PACK, action: ResourcePackAction::Accepted }),
        ("resource_pack", "discarded", P::ResourcePack { id: PACK, action: ResourcePackAction::Discarded }),
        ("custom_click_action", "payload", P::CustomClickAction { id: s("kiln:vote"), payload: Some(click_payload()) }),
        ("custom_click_action", "empty", P::CustomClickAction { id: s("kiln:close"), payload: None }),
        ("custom_click_action", "string", P::CustomClickAction { id: s("kiln:text"), payload: Some(Tag::String(s("hi"))) }),
        ("cookie_response", "present", P::CookieResponse(cookie(true))),
        ("cookie_response", "absent", P::CookieResponse(cookie(false))),
    ]
}

fn click_payload() -> Tag {
    Tag::Compound(vec![("n".into(), Tag::Int(3)), ("choice".into(), Tag::String("yes".into()))])
}

fn cookie(present: bool) -> CookieResponse {
    CookieResponse { key: "kiln:session".into(), payload: present.then(|| vec![1, 2, 3, 0xff]) }
}

/// Compound field order is not semantic (vanilla's is hash order).
fn sorted(tag: Option<Tag>) -> Option<Tag> {
    tag.map(|t| match t {
        Tag::Compound(mut f) => {
            f.sort_by(|a, b| a.0.cmp(&b.0));
            Tag::Compound(f)
        }
        t => t,
    })
}

fn normalize(p: PlayIn) -> PlayIn {
    match p {
        PlayIn::CustomClickAction { id, payload } => PlayIn::CustomClickAction { id, payload: sorted(payload) },
        p => p,
    }
}

fn decode(packet: &str, body: &[u8]) -> Result<PlayIn, DecodeError> {
    let mut r = Reader::new(body);
    Ok(decode_play(play_id(packet), &mut r)?.expect("decoded, not ignored"))
}

#[test]
fn play_packets_decode_to_vanilla_values() {
    let v = vectors();
    let mut checked = 0;
    for (packet, case, want) in expected_play() {
        let body = v.get(&("play".into(), packet.into(), case.into())).unwrap_or_else(|| panic!("no vector {packet} {case}"));
        let got = decode(packet, body).unwrap_or_else(|e| panic!("{packet} {case}: {e}"));
        assert_eq!(normalize(got), normalize(want), "{packet} {case}");
        checked += 1;
    }
    // Interact carries an LpVec3, so compare its location approximately.
    let near = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    for (case, id, hand, location, sneak) in [
        ("main", 300, Hand::Main, [0.25, 1.5, -0.125], false),
        ("off_sneak", 7, Hand::Off, [0.0; 3], true),
        ("far", 7, Hand::Main, [-3.5, 9.0, 4.0], false),
    ] {
        let body = &v[&("play".into(), "interact".into(), case.into())];
        match decode("interact", body).unwrap() {
            PlayIn::Interact { entity_id, hand: h, location: l, sneaking } => {
                assert_eq!((entity_id, h, sneaking), (id, hand, sneak), "interact {case}");
                assert!(near(l, location), "interact {case}: {l:?}");
            }
            other => panic!("interact {case}: {other:?}"),
        }
        checked += 1;
    }
    let play_vectors = v.keys().filter(|(state, ..)| state == "play").count();
    assert_eq!(checked, play_vectors, "every play vector has an expected value");
}

#[test]
fn configuration_and_login_packets_decode_to_vanilla_values() {
    let v = vectors();
    let get = |state: &str, packet: &str, case: &str| -> Reader<'_> {
        Reader::new(v.get(&(state.into(), packet.into(), case.into())).unwrap_or_else(|| panic!("{state} {packet} {case}")))
    };
    let mut r = get("configuration", "resource_pack", "accepted");
    assert_eq!(common::read_resource_pack_response(&mut r).unwrap(), (PACK, ResourcePackAction::Accepted));
    r.finish().unwrap();
    let mut r = get("configuration", "resource_pack", "discarded");
    let (_, action) = common::read_resource_pack_response(&mut r).unwrap();
    assert!(action == ResourcePackAction::Discarded && action.is_terminal());

    let mut r = get("configuration", "custom_click_action", "payload");
    let (id, payload) = common::read_custom_click_action(&mut r).unwrap();
    r.finish().unwrap();
    assert_eq!((id.as_str(), sorted(payload)), ("kiln:vote", sorted(Some(click_payload()))));
    let mut r = get("configuration", "custom_click_action", "empty");
    assert_eq!(common::read_custom_click_action(&mut r).unwrap(), ("kiln:close".into(), None));
    r.finish().unwrap();

    for state in ["login", "configuration"] {
        for (case, present) in [("present", true), ("absent", false)] {
            let mut r = get(state, "cookie_response", case);
            assert_eq!(common::read_cookie_response(&mut r).unwrap(), cookie(present), "{state} {case}");
            r.finish().unwrap();
        }
    }
    let mut r = get("configuration", "pong", "pong");
    assert_eq!(r.i32().unwrap(), -123456);
    r.finish().unwrap();
    let mut r = get("configuration", "keep_alive", "keep_alive");
    assert_eq!(r.i64().unwrap(), 1 << 40);
    r.finish().unwrap();
    get("configuration", "accept_code_of_conduct", "accept").finish().unwrap();
}

#[test]
fn decoders_reject_what_vanilla_rejects() {
    let v = vectors();
    let body = |packet: &str, case: &str| v[&("play".to_string(), packet.to_string(), case.to_string())].clone();
    // Out-of-range readEnum ordinals.
    assert!(decode("client_command", &[3]).is_err());
    let mut b = body("set_command_block", "auto");
    let mode_at = 8 + 1 + "say hi".len();
    b[mode_at] = 3;
    assert!(decode("set_command_block", &b).is_err());
    // Sign lines over 384 characters.
    let mut long = vec![0u8; 8];
    long.extend([0x81, 0x03]); // 385
    long.extend(std::iter::repeat_n(b'x', 385));
    assert!(decode("sign_update", &long).is_err());
    // Book with 101 pages.
    let mut book = vec![0, 101];
    book.extend(std::iter::repeat_n(0, 101));
    book.push(0);
    assert!(decode("edit_book", &book).is_err());
    // Bundle index below -1.
    assert!(decode("bundle_item_selected", &[1, 0xfe, 0xff, 0xff, 0xff, 0x0f]).is_err()); // -2
    // Trailing bytes.
    let mut b = body("attack", "attack");
    b.push(0);
    assert_eq!(decode("attack", &b), Err(DecodeError::TrailingBytes(1)));
    // Invalid identifiers.
    let mut bad = vec![5];
    bad.extend(b"a b:c");
    bad.push(0);
    assert!(common::read_cookie_response(&mut Reader::new(&bad)).is_err());
    // Cookie payloads over 5120 bytes.
    let mut big = vec![6];
    big.extend(b"k:big!");
    big[6] = b'x';
    big.extend([1, 0x81, 0x28]); // 5121
    big.extend(std::iter::repeat_n(0, 5121));
    assert!(common::read_cookie_response(&mut Reader::new(&big)).is_err());
}

#[test]
fn structure_block_values_are_clamped_like_vanilla() {
    let v = vectors();
    let mut b = v[&("play".into(), "set_structure_block".into(), "save".into())].clone();
    // pos (8) + update type (1) + mode (1) + name (1 + 10), then offset and size bytes.
    let at = 8 + 1 + 1 + 1 + "kiln:house".len();
    b[at..at + 6].copy_from_slice(&[0x80, 49, 0, 0xff, 100, 0]);
    match decode("set_structure_block", &b).unwrap() {
        PlayIn::SetStructureBlock(s) => {
            assert_eq!(s.offset, [-48, 48, 0]);
            assert_eq!(s.size, [0, 48, 0]);
        }
        other => panic!("{other:?}"),
    }
}
