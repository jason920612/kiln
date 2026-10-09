//! The 1.0 plugin API in the running simulation: HUD packets, locked menus and the shop,
//! land claims with PvP protection, homes, NPCs, an arena (events between plugins and block
//! edits in the owning region), deaths and spawns.

use bytes::{Bytes, BytesMut};
use kiln_data::packets::play::clientbound as ids;
use kiln_inventory::{ContainerClick, ContainerInput};
use kiln_item::HashedStack;
use kiln_link::{PlayIn, ToSim};
use kiln_proto::codec::Reader;
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct Game {
    sim: Sim,
    clients: Vec<Client>,
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn packet_id(p: &Bytes) -> i32 {
    Reader::new(p).varint().unwrap_or(-1)
}

fn item(name: &str) -> i32 {
    kiln_data::builtin_id("minecraft:item", name).unwrap_or_else(|| panic!("item {name}"))
}

fn stack(name: &str, count: i32) -> ItemStack {
    ItemStack { item: item(name), count, added: Vec::new(), removed: Vec::new() }
}

impl Game {
    /// A world with the plugins `ids` (each with the extra config), and the players joined.
    fn new(name: &str, ids: &[&str], extra: &[(&str, &str)], players: &[&str]) -> Game {
        let dir = kiln_plugin_host::examples::custom_dir(name, ids, extra).expect("example plugins");
        let mut config = SimConfig::new(8, 4, None);
        // A call budget no call reaches: tests run next to builds (a preempted call that runs
        // out of its 500 us would be denied, fail-closed, for reasons the test is not about).
        config.plugins = Some(kiln_sim::PluginSettings { call_budget: std::time::Duration::from_millis(500), ..kiln_sim::PluginSettings::new(dir) });
        let mut sim = Sim::new(config);
        let mut joins = Vec::new();
        let mut clients = Vec::new();
        for (i, p) in players.iter().enumerate() {
            let (msg, stats) = join(i as u64 + 1, p, 2);
            *stats.log.lock().unwrap() = Some(Vec::new());
            joins.push(msg);
            clients.push(Client::new(i as u64 + 1, stats));
        }
        joins.push(ToSim::Console("gamerule minecraft:spawn_mobs false".into()));
        assert!(sim.step(joins));
        let mut g = Game { sim, clients };
        g.ticks(6);
        g
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in &mut self.clients {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn send(&mut self, who: usize, pkt: PlayIn) {
        assert!(self.sim.step([ToSim::Packet(who as u64 + 1, pkt)]));
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn command(&mut self, who: usize, cmd: &str) {
        self.send(who, PlayIn::ChatCommand { command: cmd.into() });
    }

    fn received(&mut self, who: usize) -> Vec<Bytes> {
        std::mem::take(self.clients[who].stats.log.lock().unwrap().as_mut().unwrap())
    }

    fn got_text(&mut self, who: usize, text: &str) -> bool {
        self.received(who).iter().any(|p| contains(p, text))
    }

    fn ground(&self, who: usize) -> [i32; 3] {
        let p = self.clients[who].pos;
        [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32]
    }

    fn block_name(&self, p: [i32; 3]) -> &'static str {
        kiln_blocks::BlockId::of(self.sim.block_at(p[0], p[1], p[2]).expect("loaded")).name()
    }

    fn click(&mut self, who: usize, container_id: i32, slot: i16, input: ContainerInput) {
        let c = ContainerClick { container_id, state_id: 0, slot, button: 0, input, changed: Vec::new(), carried: HashedStack::Empty };
        let mut body = BytesMut::new();
        c.write(&mut body);
        self.send(who, PlayIn::ContainerClick { body: body.freeze() });
    }

    fn inventory_has(&self, who: usize, name: &str) -> bool {
        let id = item(name);
        self.sim.inventory(who as u64 + 1).unwrap_or_default().into_iter().flatten().any(|(i, n)| i == id && n > 0)
    }

    fn uuid(who: usize) -> uuid::Uuid {
        uuid::Uuid::from_u64_pair(0x6b69_6c6e, who as u64 + 1)
    }
}

/// A private sidebar, a boss bar and a welcome title reach the player; they are made of the
/// vanilla scoreboard, boss event and title packets, for this player only.
#[test]
fn the_hud_reaches_the_player_as_vanilla_packets() {
    let mut g = Game::new("api-hud", &["scoreboard-hud"], &[], &["Ann", "Ben"]);
    g.ticks(30);
    let ann = g.received(0);
    let has = |id: i32, text: &str| ann.iter().any(|p| packet_id(p) == id && contains(p, text));
    assert!(has(ids::SET_OBJECTIVE, "kiln_sb"), "the sidebar objective");
    assert!(ann.iter().any(|p| packet_id(p) == ids::SET_DISPLAY_OBJECTIVE && contains(p, "kiln_sb")), "shown in the sidebar slot");
    assert!(has(ids::SET_SCORE, "Online: "), "a line");
    assert!(ann.iter().any(|p| packet_id(p) == ids::BOSS_EVENT), "the health bar");
    assert!(has(ids::SET_TITLE_TEXT, "Welcome"), "the welcome title after joining");
    // The online list: both players are counted.
    assert!(ann.iter().any(|p| packet_id(p) == ids::SET_SCORE && contains(p, "Online: ") && contains(p, "2")), "two players online");
}

/// `/shop` opens a locked chest menu; a click buys through an atomic `try-add` and the item
/// arrives a tick later; shift-clicking the player's own items into the menu moves nothing.
#[test]
fn the_shop_menu_is_locked_and_sells() {
    let mut g = Game::new("api-shop", &["shop"], &[], &["Cara"]);
    g.console("gamemode creative Cara");
    g.ticks(2);
    g.send(0, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack("minecraft:stone", 10)) });
    g.ticks(1);
    g.console("gamemode survival Cara");
    g.ticks(1);
    g.received(0);
    g.command(0, "shop");
    g.ticks(2);
    assert!(g.received(0).iter().any(|p| packet_id(p) == ids::OPEN_SCREEN && contains(p, "Shop")), "the menu opens with its title");
    let (kind, slots) = g.sim.open_menu(1).expect("a menu is open");
    assert_eq!(kind, "minecraft:generic_9x3");
    assert_eq!(slots[12].map(|(n, _)| n), Some("minecraft:iron_sword"));
    assert_eq!(slots[10], Some(("minecraft:bread", 8)));
    // Shift-click the stone in the hotbar (menu slot 54): it must stay where it is.
    g.click(0, 1, 54, ContainerInput::QuickMove);
    g.ticks(1);
    assert!(g.inventory_has(0, "minecraft:stone"), "locked: nothing moved into the menu");
    assert_eq!(g.sim.open_menu(1).unwrap().1[0], None, "nor appeared in it");
    // Buy the iron sword with the 100 starting coins.
    g.click(0, 1, 12, ContainerInput::Pickup);
    g.ticks(3);
    assert!(g.inventory_has(0, "minecraft:iron_sword"), "bought");
    assert!(g.got_text(0, "Bought Iron sword"));
    // A diamond costs 100 and the player has 60.
    g.click(0, 1, 14, ContainerInput::Pickup);
    g.ticks(3);
    assert!(!g.inventory_has(0, "minecraft:diamond"));
    assert!(g.got_text(0, "Not enough money."));
    // The menu stays as it was, and closing it works as usual.
    assert_eq!(g.sim.open_menu(1).unwrap().1[12].map(|(n, _)| n), Some("minecraft:iron_sword"));
    g.send(0, PlayIn::ContainerClose { container_id: 1 });
    g.ticks(1);
    assert!(g.sim.open_menu(1).is_none(), "closed");
    // The wand: bought, tagged by the shop, and a right click zaps (the shop hears of it).
    g.click(0, 0, 0, ContainerInput::Pickup);
    g.command(0, "shop");
    g.ticks(2);
    g.click(0, 1, 16, ContainerInput::Pickup);
    g.ticks(3);
    assert!(g.inventory_has(0, "minecraft:stick"), "the wand");
    g.send(0, PlayIn::ContainerClose { container_id: 2 });
    g.ticks(1);
    let slot = g.sim.inventory(1).unwrap().iter().position(|s| s.is_some_and(|(i, _)| i == item("minecraft:stick"))).expect("the wand's slot");
    // Inventory list index to hotbar index: the first nine are the hotbar.
    g.send(0, PlayIn::SetCarriedItem { slot: slot as i16 });
    g.ticks(1);
    g.received(0);
    g.send(0, PlayIn::UseItem { hand: kiln_proto::packets::serverbound::Hand::Main, sequence: 9, yaw: 0.0, pitch: 0.0 });
    g.ticks(1);
    assert!(g.got_text(0, "Zap!"), "the item-use event reached the shop");
}

/// A claim: gold block placed by Dora protects 17x17 blocks; Eli cannot dig or build there,
/// cannot hit Dora there, and can elsewhere. Operators are not subject to it.
#[test]
fn claims_protect_land_and_players() {
    let mut g = Game::new("api-claims", &["claims"], &[], &["Dora", "Eli"]);
    g.console("gamemode creative Dora");
    g.console("gamemode survival Eli");
    g.console("tp Dora 300 -60 300");
    g.console("tp Eli 305 -60 300");
    g.ticks(12);
    g.send(0, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack("minecraft:gold_block", 3)) });
    g.ticks(1);
    g.console("gamemode survival Dora");
    g.ticks(1);
    let d = g.ground(0);
    // Dora places the gold block on the ground in front of her.
    g.received(0);
    g.send(0, PlayIn::UseItemOn { hand: 0, pos: d, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 });
    g.ticks(1);
    assert!(g.got_text(0, "Claimed 17x17 blocks."), "the claim was made");
    assert_eq!(g.block_name([d[0], d[1] + 1, d[2]]), "minecraft:gold_block");
    // Eli, a few blocks away, cannot dig the dirt under the claim block's neighbour.
    let e = g.ground(1);
    assert!((e[0] - d[0]).abs() < 8);
    g.received(1);
    let target = [e[0], e[1], e[2]];
    let before = g.block_name(target);
    g.send(1, PlayIn::PlayerAction { action: 0, pos: target, face: 1, sequence: 2 });
    g.ticks(2);
    assert_eq!(g.block_name(target), before, "not broken");
    assert!(g.got_text(1, "This land is claimed by"), "told why");
    // Eli cannot hit Dora on her land, and Dora is unhurt.
    let dora = g.sim.entity_id(1).unwrap();
    let hp = g.sim.health(1).unwrap().0;
    g.send(1, PlayIn::Attack { entity_id: dora });
    g.ticks(2);
    assert_eq!(g.sim.health(1).unwrap().0, hp, "no fighting on claimed land");
    // Away from the claim the same hit lands.
    g.console("tp Dora 400 -60 400");
    g.console("tp Eli 402 -60 400");
    g.ticks(12);
    let hp = g.sim.health(1).unwrap().0;
    g.send(1, PlayIn::Attack { entity_id: dora });
    g.ticks(2);
    assert!(g.sim.health(1).unwrap().0 < hp, "outside the claim players can fight");
    // An operator digs anywhere.
    g.console("op Eli");
    g.ticks(1);
    g.console("tp Eli 305 -60 300");
    g.ticks(12);
    let e = g.ground(1);
    g.send(1, PlayIn::PlayerAction { action: 0, pos: e, face: 1, sequence: 3 });
    g.ticks(2);
    assert_eq!(g.block_name(e), "minecraft:air", "ops bypass");
}

/// Homes: the warm-up teleports the player back to where `/sethome` was said.
#[test]
fn homes_bring_the_player_back() {
    let mut g = Game::new("api-homes", &["homes"], &[("homes", "warmup = 3")], &["Finn"]);
    g.console("tp Finn 200 -60 200");
    g.ticks(12);
    let home = g.clients[0].pos;
    g.command(0, "sethome");
    g.ticks(1);
    assert!(g.got_text(0, "Home home set."));
    g.console("tp Finn 500 -60 500");
    g.ticks(12);
    assert!(g.clients[0].pos[0] > 400.0);
    g.command(0, "home");
    g.ticks(1);
    assert!(g.got_text(0, "Teleporting to home"));
    g.ticks(8);
    let (_, pos) = g.sim.player_level(1).unwrap();
    assert!((pos[0] - home[0]).abs() < 1.0 && (pos[2] - home[2]).abs() < 1.0, "back at {home:?}, is at {pos:?}");
}

/// NPCs: a spawned armor stand greets whoever right-clicks it, counts the clicks in its own
/// data, and only its plugin can remove it.
#[test]
fn npcs_greet_and_are_removed_by_their_plugin() {
    let mut g = Game::new("api-npc", &["npc"], &[], &["Gus"]);
    g.console("op Gus");
    g.ticks(2);
    g.command(0, "npc spawn Guide");
    g.ticks(3);
    assert!(g.got_text(0, "Guide is here."));
    let stands = g.sim.entity_ids_of("minecraft:armor_stand");
    assert_eq!(stands.len(), 1, "spawned");
    // Right-click it twice.
    let pet = PlayIn::Interact { entity_id: stands[0], hand: kiln_proto::packets::serverbound::Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
    g.send(0, pet.clone());
    g.send(0, pet);
    g.ticks(1);
    assert!(g.got_text(0, "Hello, Gus! (2)"));
    let is_stand = |t: &kiln_proto::nbt::Tag| t.get("id").and_then(kiln_proto::nbt::Tag::as_str) == Some("minecraft:armor_stand");
    let nbt = g.sim.entity_nbt().into_iter().find(is_stand).expect("it saves");
    let data = nbt.get("kiln:plugin").and_then(|t| t.get("npc")).cloned();
    assert!(data.as_ref().and_then(|d| d.get("clicks")).is_some(), "the click count lives in the entity: {nbt:?}");
    assert!(data.as_ref().and_then(|d| d.get("kiln:owner")).is_some(), "marked as the plugin's");
    // The plugin removes it; an armor stand it did not make stays (summoned by hand).
    g.console("summon minecraft:armor_stand 10 -60 10");
    g.ticks(2);
    assert_eq!(g.sim.entity_ids_of("minecraft:armor_stand").len(), 2);
    g.command(0, "npc remove");
    g.ticks(3);
    let left = g.sim.entity_ids_of("minecraft:armor_stand");
    assert_eq!(left.len(), 1, "only its own was removed");
    assert!(!left.contains(&stands[0]));
}

/// Deaths and respawns are observed: the HUD counts the deaths and greets the respawned player.
#[test]
fn deaths_and_respawns_are_observed() {
    let mut g = Game::new("api-deaths", &["scoreboard-hud"], &[], &["Hana"]);
    g.console("gamemode survival Hana");
    g.ticks(5);
    g.console("kill Hana");
    g.ticks(2);
    assert_eq!(g.sim.plugin_player_value(Game::uuid(0), "scoreboard-hud", "deaths"), Some(1i64.to_le_bytes().to_vec()), "counted in the player's data");
    g.received(0);
    g.send(0, PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn));
    g.ticks(3);
    assert!(g.received(0).iter().any(|p| packet_id(p) == ids::SET_TITLE_TEXT && contains(p, "Welcome")), "greeted after respawning");
}

/// Plugins ask each other: the gatekeeper's veto keeps the player out; with no veto the player
/// is moved, armed and the floor is rebuilt block by block through the region owning it.
#[test]
fn the_arena_asks_the_gatekeeper_and_edits_blocks_in_its_cell() {
    // The arena at (1000, 80, 1000) in a flat world: a floor of 7x7.
    let mut g = Game::new("api-arena", &["arena", "gatekeeper"], &[("gatekeeper", "deny = \"arena:join\"")], &["Ida"]);
    g.console("gamemode survival Ida");
    g.ticks(2);
    g.received(0);
    g.command(0, "arena join");
    g.ticks(2);
    assert!(g.got_text(0, "The arena is closed."), "the veto");
    assert_eq!(g.sim.game_mode(1), Some(0), "nothing happened to the player");
    // Without the veto (a fresh world): moved, adventure mode, a sword.
    let mut g = Game::new("api-arena-open", &["arena", "gatekeeper"], &[], &["Ida"]);
    g.console("gamemode survival Ida");
    g.ticks(2);
    g.command(0, "arena join");
    g.ticks(4);
    assert_eq!(g.sim.game_mode(1), Some(2), "adventure");
    assert!(g.inventory_has(0, "minecraft:wooden_sword"));
    let (_, pos) = g.sim.player_level(1).unwrap();
    assert!((pos[0] - 1000.5).abs() < 1.0 && (pos[2] - 1000.5).abs() < 1.0, "teleported to the arena ({pos:?})");
    g.ticks(15);
    // The floor: the task runs in the region owning the cell.
    g.command(0, "arena reset");
    g.ticks(4);
    let names: Vec<&str> = [(1000, 1000), (1001, 1000), (999, 1001)].iter().map(|&(x, z)| g.block_name([x, 80, z])).collect();
    assert_eq!(names, ["minecraft:smooth_stone", "minecraft:stone_bricks", "minecraft:smooth_stone"]);
}
