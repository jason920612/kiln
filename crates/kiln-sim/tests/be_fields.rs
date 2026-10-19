//! wp82 (block entity data wp80 saw differ in `data get block`): a block entity reads the fields of its own type and drops the rest. Kiln kept every field it
//! does not model, so `setblock … {Foo:1b}` showed `Foo`, and a jukebox given `is_playing` (the song player is a field of the running song, not saved)
//! echoed it. A lectern's page is `Mth.clamp(page, 0, pageCount - 1)`: a book without pages leaves -1.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn lab() -> (Sim, Client) {
    let mut sim = Sim::new(SimConfig::new(2, 3, None));
    let (msg, stats) = join(1, "Keeper", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode creative Keeper".into())]));
    let mut client = Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    (sim, client)
}

fn place(sim: &mut Sim, client: &mut Client, block: &str) -> Tag {
    assert!(sim.step([ToSim::Console(format!("setblock 2 100 2 {block}"))]));
    for _ in 0..3 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    sim.block_entity_nbt(2, 100, 2).expect("a block entity")
}

#[test]
fn a_block_entity_drops_the_fields_it_does_not_know() {
    let (mut sim, mut client) = lab();
    let jukebox = place(&mut sim, &mut client, "minecraft:jukebox{RecordItem:{id:\"minecraft:music_disc_13\",count:1},ticks_since_song_started:5L,is_playing:1b,Foo:3b}");
    assert!(jukebox.get("is_playing").is_none(), "{jukebox:?}");
    assert!(jukebox.get("Foo").is_none(), "{jukebox:?}");
    assert!(jukebox.get("RecordItem").is_some());
    assert_eq!(jukebox.get("ticks_since_song_started").and_then(Tag::as_i64), Some(5));
    let chest = place(&mut sim, &mut client, "minecraft:chest{Foo:3b,Items:[{Slot:0b,id:\"minecraft:stone\",count:3}]}");
    assert!(chest.get("Foo").is_none(), "{chest:?}");
    assert!(chest.get("Items").is_some());
    // What the type does keep beside its own fields: a decorated pot's sherds.
    let pot = place(&mut sim, &mut client, "minecraft:decorated_pot{sherds:[\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\",\"minecraft:brick\"]}");
    assert!(pot.get("sherds").is_some(), "{pot:?}");
}

#[test]
fn a_lectern_page_is_clamped_the_way_mth_clamp_does() {
    let (mut sim, mut client) = lab();
    let none = place(&mut sim, &mut client, "minecraft:lectern{Book:{id:\"minecraft:writable_book\",count:1},Page:2}");
    assert_eq!(none.get("Page").and_then(Tag::as_i64), Some(-1), "a book without pages: min(max(2, 0), 0 - 1)");
    let three = place(
        &mut sim,
        &mut client,
        "minecraft:lectern{Book:{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[{raw:\"a\"},{raw:\"b\"},{raw:\"c\"}]}}},Page:5}",
    );
    assert_eq!(three.get("Page").and_then(Tag::as_i64), Some(2));
}
