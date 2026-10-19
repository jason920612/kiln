//! wp83 (enchantments): what a data pack that defines or redefines enchantments changes. The test packs are
//! `tools/wp83/packs/enchant/*` (written for these tests, not vanilla's files). The registries and tags are
//! process-wide, so every test holds `kiln_data::live::exclusive()` and puts the tables back when it is done.
//!
//! - `location_changed`, `tick`, `hit_block` and `projectile_spawned` effects of a pack's own enchantments run (vanilla
//!   evaluates the effect components of whatever enchantment is on the item; Kiln knew soul speed, frost walker, channeling and
//!   flame by name);
//! - a pack that redefines `minecraft:soul_speed` or `minecraft:fire_aspect` changes what they do;
//! - the tags `prevents_ice_melting` (and the like) say which enchantments stop what silk touch used to stop by id;
//! - the markers `prevent_equipment_drop` and `prevent_armor_change` belong to whichever enchantment has them.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::{Path, PathBuf};

/// Whether this machine has the vanilla data (the tags and recipes the server loads besides the packs).
fn have_datapack() -> bool {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let work = std::env::var_os("KILN_WORK").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data/minecraft/enchantment").is_dir() {
            return false;
        }
        unsafe { std::env::set_var("KILN_DATAPACK", dir) };
    }
    true
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

struct World {
    sim: Sim,
    client: Client,
    dir: PathBuf,
    /// The block the player stands on.
    ground: [i32; 3],
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Drop for World {
    fn drop(&mut self) {
        kiln_data::live::reset();
        kiln_item::enchant_flags::reset();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl World {
    /// A world with the test packs `packs` in its `datapacks/`, and a survival player.
    fn new(tag: &str, packs: &[&str]) -> Option<World> {
        if !have_datapack() {
            eprintln!("no vanilla data: skipped");
            return None;
        }
        let guard = kiln_data::live::exclusive();
        kiln_data::live::reset();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("enchant-packs-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        for pack in packs {
            copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/wp83/packs/enchant").join(pack), &dir.join("datapacks").join(pack));
        }
        std::fs::create_dir_all(&dir).unwrap();
        let mut sim = Sim::new(SimConfig::new(4, 2, Some(dir.clone())));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival Keeper".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut w = World { sim, client: Client::new(1, stats), dir, ground: [0; 3], _guard: guard };
        w.ticks(6);
        let p = w.client.pos;
        w.ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Some(w)
    }

    /// Runs `n` ticks; the client reports being on the ground where it stands.
    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            let at = self.client.pos;
            self.client.tick(Some(at), &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    fn name(&self, p: [i32; 3]) -> &'static str {
        kiln_data::blocks_types::block_of(self.block(p)).name
    }

    /// A platform of dirt one block below the player, `r` blocks out each way.
    fn platform(&mut self, r: i32) {
        let [x, y, z] = self.ground;
        self.run(&format!("fill {} {y} {} {} {y} {} minecraft:dirt", x - r, z - r, x + r, z + r));
    }

    /// Teleports the player to the middle of the block above `ground` + (dx, dz) and lets the client confirm.
    fn go(&mut self, dx: i32, dz: i32) {
        let [x, y, z] = self.ground;
        self.run(&format!("tp Keeper {} {} {}", (x + dx) as f64 + 0.5, y + 1, (z + dz) as f64 + 0.5));
        self.ticks(4);
    }

    /// Walks the player one step to the middle of the block above `ground` + (dx, dz): the client reports the new position.
    fn step_to(&mut self, dx: i32, dz: i32) {
        let [x, y, z] = self.ground;
        self.client.pos = [(x + dx) as f64 + 0.5, (y + 1) as f64, (z + dz) as f64 + 0.5];
        self.ticks(3);
    }

    fn equip(&mut self, slot: &str, item: &str) {
        self.run(&format!("item replace entity Keeper {slot} with {item}"));
    }
}

#[test]
fn a_packs_location_changed_effect_runs_when_the_wearer_changes_block() {
    let Some(mut w) = World::new("stomp", &["kz_effects"]) else { return };
    w.platform(4);
    let [x, y, z] = w.ground;
    w.go(0, 0);
    w.equip("armor.feet", "minecraft:diamond_boots[enchantments={\"kz:stomp\":1}]");
    w.ticks(3);
    // The boots were put on: the effect ran at once, on the dirt under the feet.
    assert_eq!(w.name([x, y, z]), "minecraft:gold_block", "put on");
    // A step to the next block runs it there; the ground that is not dirt (here: gold already) is not replaced.
    assert_eq!(w.name([x + 1, y, z]), "minecraft:dirt");
    w.go(1, 0);
    assert_eq!(w.name([x + 1, y, z]), "minecraft:gold_block", "a new block");
    // Standing still does nothing more: dirt put back under the player stays dirt.
    w.run(&format!("setblock {} {y} {z} minecraft:dirt", x + 1));
    w.ticks(5);
    assert_eq!(w.name([x + 1, y, z]), "minecraft:dirt", "the position did not change");
    // Without the boots it does not happen.
    w.equip("armor.feet", "minecraft:air");
    w.go(2, 0);
    assert_eq!(w.name([x + 2, y, z]), "minecraft:dirt", "no boots");
}

#[test]
fn a_packs_tick_effect_runs_a_function_every_tick() {
    let Some(mut w) = World::new("ticker", &["kz_effects"]) else { return };
    w.platform(2);
    let [x, y, z] = w.ground;
    w.go(0, 0);
    w.equip("armor.feet", "minecraft:diamond_boots[enchantments={\"kz:ticker\":1}]");
    w.ticks(3);
    // `kz:count` puts an emerald block below the wearer's feet each time it runs.
    let marker = [x, y, z];
    assert_eq!(w.name(marker), "minecraft:emerald_block");
    w.run(&format!("setblock {} {} {} minecraft:stone", marker[0], marker[1], marker[2]));
    w.ticks(2);
    assert_eq!(w.name(marker), "minecraft:emerald_block", "again, the next ticks");
}

#[test]
fn a_packs_hit_block_effect_summons_lightning_where_an_arrow_lands() {
    let Some(mut w) = World::new("strike", &["kz_effects"]) else { return };
    w.platform(6);
    let p = w.client.pos;
    let bow = |enchantments: &str| format!(",weapon:{{id:\"minecraft:bow\",count:1,components:{{\"minecraft:enchantments\":{{{enchantments}}}}}}}");
    // An arrow from a plain bow lands without a bolt.
    w.run(&format!("summon minecraft:arrow {} {} {} {{Motion:[0.0d,-2.0d,0.0d],crit:0b{}}}", p[0] + 4.0, p[1] + 3.0, p[2], bow("\"minecraft:power\":1")));
    w.ticks(10);
    assert!(w.sim.entity_ids_of("minecraft:lightning_bolt").is_empty(), "no hit_block effect");
    // Strike: a bolt where the arrow hit.
    w.run(&format!("summon minecraft:arrow {} {} {} {{Motion:[0.0d,-2.0d,0.0d],crit:0b{}}}", p[0] - 4.0, p[1] + 3.0, p[2], bow("\"kz:strike\":1")));
    let mut bolts = 0;
    for _ in 0..10 {
        w.ticks(1);
        bolts = bolts.max(w.sim.entity_ids_of("minecraft:lightning_bolt").len());
    }
    assert_eq!(bolts, 1, "the hit_block effect of kz:strike summoned a lightning bolt");
}

/// The fire of the arrow `shot` in the arrow's saved data (`Fire`, ticks), shot from a bow with `enchantments`.
fn shot_arrow_fire(w: &mut World, enchantments: &str) -> i32 {
    w.run("clear Keeper");
    w.equip("weapon.mainhand", &format!("minecraft:bow[enchantments={{{enchantments}}}]"));
    w.run("give Keeper minecraft:arrow 5");
    w.ticks(2);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 1, yaw: 0.0, pitch: 0.0 })]));
    w.ticks(22);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 6, pos: [0, 0, 0], face: 0, sequence: 2 })]));
    let arrows: Vec<Tag> = w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:arrow")).collect();
    assert_eq!(arrows.len(), 1, "one arrow shot: {arrows:?}");
    match arrows[0].get("Fire") {
        Some(Tag::Short(f)) => *f as i32,
        other => panic!("no Fire in {other:?}"),
    }
}

#[test]
fn a_packs_projectile_spawned_effect_sets_the_arrow_alight_for_as_long_as_it_says() {
    let Some(mut w) = World::new("flare", &["kz_effects"]) else { return };
    w.ticks(2);
    // Flame (vanilla's definition: 100 seconds) and a pack's kz:flare (7 seconds) are the effect lists of their own enchantments.
    let flame = shot_arrow_fire(&mut w, "\"minecraft:flame\":1");
    w.run("kill @e[type=minecraft:arrow]");
    w.ticks(2);
    let flare = shot_arrow_fire(&mut w, "\"kz:flare\":1");
    assert!(flame > 1500, "flame: {flame}");
    assert!((100..=140).contains(&flare), "flare: {flare}");
}

#[test]
fn redefining_soul_speed_changes_its_modifier_and_it_stops_off_soul_blocks() {
    let Some(mut w) = World::new("soul", &["soul_override"]) else { return };
    w.platform(3);
    let [x, y, z] = w.ground;
    w.run(&format!("setblock {x} {y} {z} minecraft:soul_sand"));
    w.go(0, 0);
    w.equip("armor.feet", "minecraft:diamond_boots[enchantments={\"minecraft:soul_speed\":1}]");
    w.ticks(3);
    let mods = w.sim.attribute_modifiers(1, "minecraft:movement_speed").unwrap();
    let soul = mods.iter().find(|m| m.0 == "minecraft:enchantment.soul_speed/feet").unwrap_or_else(|| panic!("no soul speed modifier in {mods:?}"));
    assert_eq!(soul.1, 0.5, "the pack's amount, not vanilla's 0.0405");
    // The pack's definition has no second modifier (vanilla's adds movement efficiency).
    assert!(w.sim.attribute_modifiers(1, "minecraft:movement_efficiency").unwrap().iter().all(|m| m.0 != "minecraft:enchantment.soul_speed/feet"));
    // Off the soul sand (a step to the next block) the requirement fails: the modifier goes.
    w.step_to(1, 0);
    let mods = w.sim.attribute_modifiers(1, "minecraft:movement_speed").unwrap();
    assert!(mods.iter().all(|m| m.0 != "minecraft:enchantment.soul_speed/feet"), "{mods:?}");
}

#[test]
fn a_spears_stab_burns_for_as_long_as_the_packs_fire_aspect_says() {
    let Some(mut w) = World::new("fire", &["fire_override"]) else { return };
    w.platform(3);
    let p = w.client.pos;
    // A zombie that stays put one block ahead (south, where the player faces at yaw 0), unhurt by the daylight.
    w.run(&format!("summon minecraft:zombie {} {} {} {{NoAI:1b,PersistenceRequired:1b,Silent:1b,Invulnerable:0b,CanBreakDoors:0b,ArmorItems:[{{id:\"minecraft:golden_helmet\",count:1}},{{}},{{}},{{}}],drop_chances:{{head:0.0f}}}}", p[0], p[1], p[2] + 1.5));
    w.equip("weapon.mainhand", "minecraft:iron_spear[enchantments={\"minecraft:fire_aspect\":1}]");
    w.ticks(30);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 8, pos: [0, 0, 0], face: 0, sequence: 3 })]));
    w.ticks(1);
    let zombies: Vec<Tag> = w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:zombie")).collect();
    assert_eq!(zombies.len(), 1);
    let fire = match zombies[0].get("Fire") {
        Some(Tag::Short(f)) => *f as i32,
        other => panic!("no Fire in {other:?}"),
    };
    // 9 seconds from the pack (vanilla's own fire aspect I: 4 seconds = 80 ticks).
    assert!((170..=180).contains(&fire), "fire: {fire}");
}

/// Breaks the block under the player's feet + (dx, 0, dz) by hand and with `tool`, in survival, and returns what is at its place.
fn break_ice(w: &mut World, tool: &str, dx: i32) -> &'static str {
    let [x, y, z] = w.ground;
    // On the platform, beside the player: the dirt below is what the water melts onto.
    let ice = [x + dx, y + 1, z];
    w.run(&format!("setblock {} {} {} minecraft:ice", ice[0], ice[1], ice[2]));
    w.equip("weapon.mainhand", tool);
    w.ticks(2);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 0, pos: ice, face: 1, sequence: 10 + dx })]));
    // The client says it is done (with an iron pickaxe ice takes 3 ticks); the server breaks it.
    w.ticks(6);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerAction { action: 3, pos: ice, face: 1, sequence: 40 + dx })]));
    w.ticks(4);
    w.name(ice)
}

#[test]
fn ice_melts_unless_the_tool_has_an_enchantment_of_the_tag() {
    let Some(mut w) = World::new("ice", &["ice_tags"]) else { return };
    w.platform(6);
    // The pack replaced `#minecraft:prevents_ice_melting` with kz:warm alone: silk touch no longer stops the melt.
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe[enchantments={\"kz:warm\":1}]", 1), "minecraft:air", "kz:warm is in the tag");
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe[enchantments={\"minecraft:silk_touch\":1}]", 2), "minecraft:water", "silk touch is not any more");
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe", 3), "minecraft:water", "a plain pickaxe");
}

#[test]
fn vanilla_tags_keep_silk_touch_stopping_the_melt() {
    let Some(mut w) = World::new("ice-vanilla", &["kz_effects"]) else { return };
    w.platform(6);
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe[enchantments={\"minecraft:silk_touch\":1}]", 1), "minecraft:air");
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe", 2), "minecraft:water");
}

#[test]
fn a_packs_enchantment_with_the_vanishing_marker_vanishes_with_its_wearer() {
    let Some(mut w) = World::new("hex", &["kz_curse"]) else { return };
    w.run("clear Keeper");
    w.run("give Keeper minecraft:diamond_helmet[enchantments={\"kz:hex\":1}]");
    w.run("give Keeper minecraft:apple 3");
    w.run("give Keeper minecraft:diamond_helmet");
    w.ticks(2);
    w.run("kill Keeper");
    w.ticks(3);
    let dropped: Vec<String> = w.sim.item_stacks().iter().map(|s| s.item_name().to_owned()).collect();
    // The apples and the plain helmet drop; the hexed helmet is destroyed (`destroyVanishingCursedItems`).
    assert!(dropped.iter().any(|n| n == "minecraft:apple"), "{dropped:?}");
    assert_eq!(dropped.iter().filter(|n| *n == "minecraft:diamond_helmet").count(), 1, "{dropped:?}");
}

#[test]
fn a_packs_enchantment_with_the_binding_marker_cannot_be_taken_off() {
    let Some(mut w) = World::new("hex-binding", &["kz_curse"]) else { return };
    w.run("clear Keeper");
    w.equip("armor.head", "minecraft:diamond_helmet[enchantments={\"kz:hex\":1}]");
    w.run("give Keeper minecraft:diamond_helmet");
    w.ticks(2);
    // Using another helmet swaps it with the worn one unless the worn one is bound.
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 1, yaw: 0.0, pitch: 0.0 })]));
    w.ticks(2);
    let head = w.sim.saved_player_nbt(1).unwrap();
    let items = ();
    let _ = items;
    let worn = head.get("equipment").and_then(|e| e.get("head")).expect("a helmet is worn");
    let components = format!("{worn:?}");
    assert!(components.contains("kz:hex"), "the bound helmet stayed: {components}");
}

#[test]
fn reload_rereads_the_tags_but_keeps_the_enchantments_the_world_loaded() {
    let Some(mut w) = World::new("reload", &["kz_effects", "ice_tags"]) else { return };
    w.platform(6);
    let [x, y, z] = w.ground;
    // On disk the pack now says diamond (not gold) for kz:stomp, and the tag lets silk touch stop the melt (not kz:warm).
    let packs = w.dir.join("datapacks");
    let stomp = packs.join("kz_effects/data/kz/enchantment/stomp.json");
    let text = std::fs::read_to_string(&stomp).unwrap();
    std::fs::write(&stomp, text.replace("minecraft:gold_block", "minecraft:diamond_block")).unwrap();
    std::fs::write(packs.join("ice_tags/data/minecraft/tags/enchantment/prevents_ice_melting.json"), r#"{"replace": true, "values": ["minecraft:silk_touch"]}"#).unwrap();
    w.run("reload");
    w.ticks(3);
    // Vanilla loads the enchantment registry once with the world: `/reload` does not change what kz:stomp does ...
    w.go(0, 0);
    w.equip("armor.feet", "minecraft:diamond_boots[enchantments={\"kz:stomp\":1}]");
    w.ticks(3);
    assert_eq!(w.name([x, y, z]), "minecraft:gold_block", "the definition of the world");
    // ... but the tags are read again.
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe[enchantments={\"minecraft:silk_touch\":1}]", 2), "minecraft:air", "silk touch is in the tag now");
    assert_eq!(break_ice(&mut w, "minecraft:iron_pickaxe[enchantments={\"kz:warm\":1}]", 3), "minecraft:water", "kz:warm no longer");
}

impl World {
    fn hold(&mut self, slot: i16, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = kiln_proto::packets::ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot, item: Some(stack) })]));
    }

    fn use_on(&mut self, pos: [i32; 3]) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 77 };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn click(&mut self, container_id: i32, slot: i16, input: kiln_inventory::ContainerInput) {
        let c = kiln_inventory::ContainerClick { container_id, state_id: 0, slot, button: 0, input, changed: Vec::new(), carried: kiln_item::HashedStack::Empty };
        let mut body = bytes::BytesMut::new();
        c.write(&mut body);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() })]));
    }

    /// The enchantments (`name`, level) of the items the player carries, named by the item.
    fn enchanted(&self) -> Vec<(String, String)> {
        let nbt = self.sim.saved_player_nbt(1).unwrap();
        let Some(Tag::List(items)) = nbt.get("Inventory") else { return Vec::new() };
        items
            .iter()
            .map(|i| i.unwrap_list_element())
            .map(|i| (i.get("id").and_then(Tag::as_str).unwrap_or("").to_owned(), format!("{:?}", i.get("components"))))
            .collect()
    }
}

#[test]
fn enchant_command_follows_the_packs_limits_supported_items_and_exclusive_sets() {
    let Some(mut w) = World::new("enchant-command", &["kz_table"]) else { return };
    let has = |w: &World, what: &str| w.enchanted().iter().any(|(_, c)| c.contains(what));
    w.run("item replace entity Keeper weapon.mainhand with minecraft:diamond_sword");
    w.ticks(2);
    // Level 4 is above kz:aura's max_level 3.
    w.run("enchant Keeper kz:aura 4");
    assert!(!has(&w, "kz:aura"), "level above max_level");
    w.run("enchant Keeper kz:aura 3");
    assert!(has(&w, "kz:aura"), "{:?}", w.enchanted());
    // kz:spark is in the exclusive set of kz:aura: refused, though the sword supports it.
    w.run("enchant Keeper kz:spark 1");
    assert!(!has(&w, "kz:spark"), "exclusive_set");
    // A pickaxe is not a supported item.
    w.run("item replace entity Keeper weapon.mainhand with minecraft:diamond_pickaxe");
    w.ticks(2);
    w.run("enchant Keeper kz:aura 1");
    assert!(!w.enchanted().iter().any(|(id, c)| id == "minecraft:diamond_pickaxe" && c.contains("kz:aura")), "supported_items");
    // A fresh sword takes kz:spark.
    w.run("item replace entity Keeper weapon.mainhand with minecraft:diamond_sword");
    w.ticks(2);
    w.run("enchant Keeper kz:spark 2");
    assert!(has(&w, "kz:spark"));
}

#[test]
fn the_enchanting_table_and_the_anvil_use_the_packs_enchantments() {
    let Some(mut w) = World::new("table-anvil", &["kz_table"]) else { return };
    // (An anvil in the void falls.)
    w.platform(6);
    w.run("gamemode creative Keeper");
    w.ticks(2);
    let [x, y, z] = w.ground;
    // `#in_enchanting_table` is kz:aura alone in this pack: a sword comes out with kz:aura and nothing else.
    let table = [x + 3, y + 1, z - 3];
    w.run(&format!("setblock {} {} {} minecraft:enchanting_table", table[0], table[1], table[2]));
    for (dx, dz) in [(-2, -2), (-2, 0), (-2, 2), (0, -2), (0, 2), (2, -2), (2, 0), (2, 2)] {
        w.run(&format!("setblock {} {} {} minecraft:bookshelf", table[0] + dx, table[1], table[2] + dz));
    }
    w.hold(36, "minecraft:diamond_sword", 1);
    w.use_on(table);
    assert_eq!(w.sim.open_menu(1).map(|m| m.0), Some("minecraft:enchantment"));
    w.click(1, 29, kiln_inventory::ContainerInput::QuickMove);
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerButtonClick { container_id: 1, button_id: 2 })]));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: 1 })]));
    let swords: Vec<String> = w.enchanted().into_iter().filter(|(id, _)| id == "minecraft:diamond_sword").map(|(_, c)| c).collect();
    assert_eq!(swords.len(), 1, "{swords:?}");
    assert!(swords[0].contains("kz:aura"), "{swords:?}");
    assert_eq!(swords[0].matches("Int(").count(), 1, "only kz:aura is in the table's tag: {swords:?}");
    // The anvil: two swords with kz:aura 2 make kz:aura 3 (max_level 3 of the pack), not more and not 1.
    w.run("clear Keeper");
    w.run("give Keeper minecraft:diamond_sword[enchantments={\"kz:aura\":2}]");
    w.run("give Keeper minecraft:diamond_sword[enchantments={\"kz:aura\":2}]");
    let anvil = [x - 3, y + 1, z - 3];
    w.run(&format!("setblock {} {} {} minecraft:anvil", anvil[0], anvil[1], anvil[2]));
    w.use_on(anvil);
    assert_eq!(w.sim.open_menu(1).map(|m| m.0), Some("minecraft:anvil"));
    // Shift-click both swords in (inventory menu slots 3.. are the player's), then take the result.
    for slot in [30, 31] {
        w.click(2, slot, kiln_inventory::ContainerInput::QuickMove);
    }
    let (_, slots) = w.sim.open_menu(1).unwrap();
    assert!(slots[2].is_some(), "a result: {slots:?}");
    w.click(2, 2, kiln_inventory::ContainerInput::QuickMove);
    let merged: Vec<String> = w.enchanted().into_iter().filter(|(id, c)| id == "minecraft:diamond_sword" && c.contains("kz:aura")).map(|(_, c)| c).collect();
    assert!(merged.iter().any(|c| c.contains("Int(3)")), "kz:aura III: {merged:?}");
}

#[test]
fn every_enchantment_with_a_charge_time_effect_speeds_the_crossbow_up() {
    let Some(mut w) = World::new("crossbow", &["kz_effects"]) else { return };
    // Quick charge III (-0.75 s) and the pack's kz:swift (-0.25 s) both apply to 1.25 s: 0.25 s = 5 ticks. (Only the highest level's
    // effect would leave 0.5 s = 10 ticks.)
    let charged_after = |w: &mut World, enchantments: &str, ticks: usize| -> bool {
        w.run("clear Keeper");
        w.equip("weapon.mainhand", &format!("minecraft:crossbow[enchantments={{{enchantments}}}]"));
        w.run("give Keeper minecraft:arrow 4");
        w.ticks(2);
        assert!(w.sim.step([ToSim::Packet(1, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 5, yaw: 0.0, pitch: 0.0 })]));
        w.ticks(ticks);
        w.enchanted().iter().any(|(id, c)| id == "minecraft:crossbow" && c.contains("charged_projectiles"))
    };
    assert!(charged_after(&mut w, "\"minecraft:quick_charge\":3,\"kz:swift\":1", 8), "both effects: charged within 8 ticks");
    assert!(!charged_after(&mut w, "\"minecraft:quick_charge\":3", 8), "quick charge III alone needs 10 ticks");
}
