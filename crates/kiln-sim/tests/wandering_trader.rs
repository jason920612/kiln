//! Wandering traders: trades from the vanilla trade sets, the drinking, the despawn delay, the
//! spawner (a trader and two trader llamas on leads) and what a save keeps.
//!
//! Trade sets come from the vanilla datapack (`KILN_DATAPACK` or `work/generated`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/recipe").is_dir();
        }
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
        let found = dir.join("data/minecraft/recipe").is_dir();
        if found {
            unsafe { std::env::set_var("KILN_DATAPACK", dir) };
        }
        found
    })
}

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> Option<Self> {
        if !have_datapack() {
            eprintln!("no vanilla datapack (KILN_DATAPACK or work/generated): skipped");
            return None;
        }
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "User", 3);
        assert!(sim.step([msg, ToSim::Console("gamemode creative User".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..8 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let mut w = Self { sim, client };
        w.run("gamemode survival User");
        Some(w)
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn interact(&mut self, id: i32) {
        let pkt = PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn summon(&mut self, name: &str, nbt: &str) -> i32 {
        let before = self.sim.entity_ids_of(name);
        let p = self.client.pos;
        self.run(&format!("summon {name} {} {} {} {{PersistenceRequired:1b{nbt}}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(1);
        *self.sim.entity_ids_of(name).iter().find(|id| !before.contains(id)).expect("summoned")
    }

    fn nbt(&self, name: &str) -> Tag {
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }
}

#[test]
fn a_click_opens_the_trade_screen_with_the_wandering_traders_offers() {
    let Some(mut w) = World::new() else { return };
    let trader = w.summon("minecraft:wandering_trader", ",NoAI:1b");
    w.interact(trader);
    w.ticks(1);
    let (kind, _level, slots) = w.sim.merchant_screen(1).expect("a merchant screen opened");
    assert_eq!(kind, 1, "level 1");
    assert!(slots.len() >= 3, "offers");
    // The saved trader keeps its offers: some of the buying, uncommon and common sets.
    let saved = w.nbt("minecraft:wandering_trader");
    let Some(Tag::Compound(offers)) = saved.get("Offers") else { panic!("{saved:?}") };
    let Some(Tag::List(recipes)) = offers.iter().find(|(k, _)| k == "Recipes").map(|(_, v)| v) else { panic!() };
    // 1 buying + 1 uncommon + 2 common... as the sets say (at least 5 in all).
    assert!(recipes.len() >= 5, "{} trades", recipes.len());
}

#[test]
fn a_trader_leaves_when_its_despawn_delay_runs_out() {
    let Some(mut w) = World::new() else { return };
    w.summon("minecraft:wandering_trader", ",NoAI:1b,DespawnDelay:30");
    let saved = w.nbt("minecraft:wandering_trader");
    assert!(saved.get("DespawnDelay").and_then(Tag::as_i64).unwrap() <= 30);
    w.ticks(40);
    assert!(w.sim.entity_ids_of("minecraft:wandering_trader").is_empty(), "gone");
}

#[test]
fn a_trader_keeps_its_wander_target_and_despawn_delay_across_a_save() {
    let Some(mut w) = World::new() else { return };
    w.summon("minecraft:wandering_trader", ",NoAI:1b,DespawnDelay:4321,wander_target:[I;10,70,-3]");
    let saved = w.nbt("minecraft:wandering_trader");
    assert!(saved.get("DespawnDelay").and_then(Tag::as_i64).unwrap() <= 4321);
    assert_eq!(saved.get("wander_target"), Some(&Tag::IntArray(vec![10, 70, -3])));
}

#[test]
fn the_spawner_makes_a_trader_with_two_leashed_trader_llamas() {
    let Some(mut w) = World::new() else { return };
    w.run("gamerule minecraft:spawn_wandering_traders true");
    // Keep trying (a try in ten passes the roll); the world is flat and loaded around the player.
    for _ in 0..400 {
        w.sim.force_trader_attempt();
        w.ticks(2);
        if !w.sim.entity_ids_of("minecraft:wandering_trader").is_empty() {
            break;
        }
    }
    assert_eq!(w.sim.entity_ids_of("minecraft:wandering_trader").len(), 1, "a trader appeared");
    w.ticks(3);
    let trader = w.nbt("minecraft:wandering_trader");
    assert!(trader.get("DespawnDelay").and_then(Tag::as_i64).unwrap() > 47000, "a day or two to live: {trader:?}");
    assert!(trader.get("wander_target").is_some(), "somewhere to walk to");
    let trader_uuid = trader.get("UUID").cloned().unwrap();
    let llamas: Vec<Tag> = w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:trader_llama")).collect();
    assert!(!llamas.is_empty() && llamas.len() <= 2, "{} llamas", llamas.len());
    for l in llamas {
        let leash = l.get("leash").unwrap_or_else(|| panic!("a lead: {l:?}"));
        assert_eq!(leash.get("UUID"), Some(&trader_uuid), "led by the trader");
    }
    // The chance goes back to 25 after a spawn.
    assert_eq!(w.sim.trader_spawner().2, 25);
}

#[test]
fn a_trader_llama_led_by_a_trader_lives_as_long_as_the_trader() {
    let Some(mut w) = World::new() else { return };
    w.run("gamerule minecraft:spawn_wandering_traders true");
    for _ in 0..400 {
        w.sim.force_trader_attempt();
        w.ticks(2);
        if !w.sim.entity_ids_of("minecraft:wandering_trader").is_empty() {
            break;
        }
    }
    w.ticks(3);
    let llamas = w.sim.entity_ids_of("minecraft:trader_llama");
    assert!(!llamas.is_empty());
    // Their own delays are not counted while the trader's is: the saved ones follow the trader's.
    let trader_delay = w.nbt("minecraft:wandering_trader").get("DespawnDelay").and_then(Tag::as_i64).unwrap();
    for l in w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:trader_llama")) {
        let d = l.get("DespawnDelay").and_then(Tag::as_i64).unwrap();
        assert!((d - trader_delay).abs() <= 3, "llama {d} trader {trader_delay}");
    }
}

impl World {
    /// Puts `count` of `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        self.run("gamemode creative User");
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = kiln_proto::packets::ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
        self.run("gamemode survival User");
    }

    fn click(&mut self, container_id: i32, slot: i16, quick: bool) {
        use kiln_inventory::{ContainerClick, ContainerInput};
        let input = if quick { ContainerInput::QuickMove } else { ContainerInput::Pickup };
        let c = ContainerClick { container_id, state_id: 0, slot, button: 0, input, changed: Vec::new(), carried: kiln_item::HashedStack::Empty };
        let mut body = bytes::BytesMut::new();
        c.write(&mut body);
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ContainerClick { body: body.freeze() })]));
    }

    fn count_of(&self, item: &str) -> i32 {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.sim.inventory(1).unwrap().iter().flatten().filter(|s| s.0 == id).map(|s| s.1).sum()
    }
}

#[test]
fn trading_with_a_wandering_trader_uses_up_its_offers_and_pays_experience() {
    let Some(mut w) = World::new() else { return };
    w.hold("minecraft:emerald", 10);
    let offers = r#"Offers:{Recipes:[{buy:{id:"minecraft:emerald",count:2},sell:{id:"minecraft:stick",count:3},maxUses:2,xp:1,priceMultiplier:0.05f}]}"#;
    let trader = w.summon("minecraft:wandering_trader", &format!(",NoAI:1b,{offers}"));
    w.interact(trader);
    let (id, merchant, slots) = w.sim.merchant_screen(1).expect("the screen opened");
    assert_eq!(merchant, trader);
    assert!(slots.iter().all(Option::is_none));
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SelectTrade { offer: 0 })]));
    // Shift-clicking the result trades until the offer runs out (2 uses): 4 emeralds for 6 sticks.
    w.click(id, 2, true);
    assert_eq!(w.count_of("minecraft:stick"), 6);
    // The rest of the payment sits in the payment slot (it goes back when the screen closes).
    let (_, _, slots) = w.sim.merchant_screen(1).unwrap();
    assert_eq!(slots[0].map(|s| s.1), Some(6));
    w.ticks(2);
    // The trader is still trading until the screen closes; the uses were counted on its offers.
    let saved = w.nbt("minecraft:wandering_trader");
    let Some(Tag::Compound(offers)) = saved.get("Offers") else { panic!("{saved:?}") };
    let Some(Tag::List(recipes)) = offers.iter().find(|(k, _)| k == "Recipes").map(|(_, v)| v) else { panic!() };
    assert_eq!(recipes[0].get("uses").and_then(Tag::as_i64), Some(2));
    // Experience for the trades: orbs (3 to 6 each) near the trader.
    let orbs = w.sim.entities().iter().filter(|e| e.0 == "minecraft:experience_orb").count();
    assert!(orbs >= 1 || w.sim.experience(1).unwrap().2 > 0, "trading experience");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::ContainerClose { container_id: id })]));
    assert!(w.sim.merchant_screen(1).is_none());
    assert_eq!(w.count_of("minecraft:emerald"), 6, "the payment came back");
}

#[test]
fn a_wandering_trader_stands_still_and_looks_at_the_player_it_trades_with() {
    let Some(mut w) = World::new() else { return };
    let trader = w.summon("minecraft:wandering_trader", ",DespawnDelay:48000");
    // It would walk off to wander; trading pins it.
    w.interact(trader);
    let (_, _, _) = w.sim.merchant_screen(1).expect("the screen opened");
    let before = w.nbt("minecraft:wandering_trader").get("Pos").cloned();
    w.ticks(20);
    let after = w.nbt("minecraft:wandering_trader").get("Pos").cloned();
    assert_eq!(before, after, "it stood still while trading");
    // The despawn delay does not count down while it trades.
    let delay = w.nbt("minecraft:wandering_trader").get("DespawnDelay").and_then(Tag::as_i64).unwrap();
    w.ticks(20);
    assert_eq!(w.nbt("minecraft:wandering_trader").get("DespawnDelay").and_then(Tag::as_i64), Some(delay));
}
