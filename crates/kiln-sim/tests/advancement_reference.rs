//! wp84 (a data pack played on both servers): an advancement criterion whose `player` is a plain reference to a predicate file
//! (`"player": "ns:holding_gold"`, as vanilla's `LootItemCondition.CODEC` takes it) loads and can be granted; Kiln refused the string
//! ("expected an object, found a string") and left the advancement out.

mod support;

use kiln_proto::Reader;

#[test]
fn an_advancement_may_name_a_predicate_file_as_its_player_condition() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("wp84-adv-ref");
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("datapacks/refpack");
    let write = |rel: &str, text: &str| {
        let p = pack.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    write("pack.mcmeta", r#"{"pack":{"description":"wp84","min_format":121,"max_format":121}}"#);
    write("data/rp/predicate/gold.json", r#"{"type":"minecraft:entity_properties","entity":"this","predicate":{"minecraft:equipment":{"mainhand":{"items":"minecraft:gold_ingot"}}}}"#);
    write(
        "data/rp/advancement/by_name.json",
        r#"{"criteria":{"c":{"trigger":"minecraft:inventory_changed","conditions":{"player":"rp:gold"}}},"display":{"icon":{"id":"minecraft:gold_ingot"},"title":{"text":"x"},"description":{"text":"y"},"show_toast":false,"announce_to_chat":false}}"#,
    );
    let config = kiln_sim::SimConfig::new(4, 4, Some(dir.clone()));
    let mut sim = kiln_sim::Sim::new(config);
    let (msg, stats) = kiln_sim::testing::join(1, "Gilder", 2);
    *stats.log.lock().unwrap() = Some(Vec::new());
    let log = stats.clone();
    assert!(sim.step([msg]));
    let mut client = kiln_sim::testing::Client::new(1, stats);
    for _ in 0..5 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    assert!(sim.step([kiln_link::ToSim::Console("advancement grant Gilder only rp:by_name".into())]));
    for _ in 0..3 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let packets = log.log.lock().unwrap().clone().unwrap();
    let told = packets.iter().any(|p| {
        let mut r = Reader::new(p);
        r.varint().ok() == Some(kiln_data::packets::play::clientbound::UPDATE_ADVANCEMENTS) && p.windows(7).any(|w| w == b"rp:by_n") 
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert!(told, "the client was told of rp:by_name (it loaded)");
}
