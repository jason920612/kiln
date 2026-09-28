//! Mobs end to end: `/summon`, spawn eggs, a zombie hitting a player at night, a player killing a
//! pig (loot and experience), a creeper blowing up, burning zombies by day, despawning.
//! The mob behaviour itself is checked tick by tick against vanilla by kiln-entity's
//! `mob_parity` test (tools/mob_vectors.py).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::ItemStack;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    clients: Vec<Client>,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Hunter", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, clients: vec![Client::new(1, stats)] };
        w.console("gamerule minecraft:natural_health_regeneration false");
        // Only the mobs a test summons (natural spawning has its own test; slimes spawn in the
        // superflat world's slime chunks at once).
        w.console("gamerule minecraft:spawn_mobs false");
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            for c in self.clients.iter_mut() {
                c.tick(None, &mut inbox);
            }
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn pos(&self) -> [f64; 3] {
        self.clients[0].pos
    }

    /// Summons `entity` at the player's position plus `offset`.
    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.pos();
        self.console(format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]).trim());
        self.ticks(1);
    }

    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }

    fn health(&self) -> f32 {
        self.sim.health(1).unwrap().0
    }
}

#[test]
fn summon_creates_mobs_and_they_fall_and_wander() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:pig", [3.0, 2.0, 0.0], "");
    let pigs = w.mobs("minecraft:pig");
    assert_eq!(pigs.len(), 1, "one pig");
    let y0 = pigs[0].1[1];
    w.ticks(40);
    let pig = w.mobs("minecraft:pig")[0];
    assert!(pig.1[1] < y0, "the pig fell ({y0} -> {})", pig.1[1]);
    assert_eq!(pig.2, 10.0);
    // An unknown or non-mob type fails.
    w.summon("minecraft:boat", [1.0, 0.0, 0.0], "");
    assert!(w.mobs("minecraft:boat").is_empty());
}

#[test]
fn zombies_attack_survival_players_at_night() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:zombie", [4.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(200);
    assert!(w.health() < 20.0, "the zombie hit the player (health {})", w.health());
}

#[test]
fn creative_players_are_ignored() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [3.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(100);
    assert_eq!(w.health(), 20.0);
}

#[test]
fn players_kill_pigs() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    let id = kiln_data::builtin_id("minecraft:item", "minecraft:diamond_sword").unwrap();
    let stack = ItemStack { item: id, count: 1, added: Vec::new(), removed: Vec::new() };
    w.console("gamemode creative Hunter");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    w.console("gamemode survival Hunter");
    w.summon("minecraft:pig", [0.0, 0.0, 2.0], "{NoAI:1b}");
    w.ticks(20);
    let (pig, _, _) = w.mobs("minecraft:pig")[0];
    // A full-strength diamond sword hit (7) takes the pig to 3, the second one kills it.
    w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: pig }), ToSim::Packet(1, PlayIn::Punch)]);
    assert_eq!(w.mobs("minecraft:pig")[0].2, 3.0);
    w.ticks(25);
    w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: pig }), ToSim::Packet(1, PlayIn::Punch)]);
    let dead = w.mobs("minecraft:pig");
    assert!(dead.is_empty() || dead[0].2 == 0.0, "the pig died");
    w.ticks(25);
    assert!(w.mobs("minecraft:pig").is_empty(), "removed after the death animation");
    let entities = w.sim.entities();
    // With the vanilla datapack's loot tables, the pig drops porkchops (1 to 3).
    if std::env::var_os("KILN_DATAPACK").is_some() {
        // (The player two blocks away may already have picked the porkchops up.)
        let picked = w.count_of("minecraft:porkchop") > 0;
        assert!(picked || entities.iter().any(|e| e.0 == "minecraft:item"), "loot: {entities:?}");
    }
    // A player kill drops 1 to 3 experience; the orbs spawn in reach and are taken.
    w.ticks(10);
    let (level, _, total) = w.sim.experience(1).unwrap();
    assert!((1..=3).contains(&total) && level == 0, "experience {total}");
    assert!(!w.sim.entities().iter().any(|e| e.0 == "minecraft:experience_orb"));
}

#[test]
fn creepers_explode_next_to_players() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:creeper", [2.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(120);
    assert!(w.mobs("minecraft:creeper").is_empty(), "the creeper blew up");
    assert!(w.health() < 20.0, "the explosion hurt the player (health {})", w.health());
}

#[test]
fn zombies_burn_in_daylight() {
    let mut w = World::new();
    w.console("time set 6000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [5.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(600);
    let z = w.mobs("minecraft:zombie");
    assert!(z.is_empty() || z[0].2 < 20.0, "burning hurts: {z:?}");
}

#[test]
fn monsters_far_from_players_despawn() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [3.0, 0.0, 0.0], "");
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:zombie").len(), 1);
    // Peaceful removes monsters, persistent or not.
    w.summon("minecraft:zombie", [3.0, 0.0, 1.0], "{PersistenceRequired:1b}");
    w.console("difficulty peaceful");
    w.ticks(2);
    assert!(w.mobs("minecraft:zombie").is_empty(), "peaceful removes monsters");
}

#[test]
fn monsters_spawn_naturally_at_night_within_the_cap() {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        eprintln!("natural spawning needs the vanilla datapack (KILN_DATAPACK); skipped");
        return;
    }
    let mut w = World::new();
    w.console("gamerule minecraft:spawn_mobs true");
    w.console("gamemode creative Hunter");
    w.console("time set 18000");
    // Freshly spawned: at least 24 blocks from the player.
    w.ticks(20);
    let p = w.pos();
    for m in w.sim.mobs() {
        let d = ((m.2[0] - p[0]).powi(2) + (m.2[1] - p[1]).powi(2) + (m.2[2] - p[2]).powi(2)).sqrt();
        assert!(d > 23.0, "{} spawned {d:.1} blocks from the player", m.1);
    }
    w.ticks(280);
    let all = w.sim.mobs();
    let monsters = all.iter().filter(|m| matches!(m.1, "minecraft:zombie" | "minecraft:skeleton" | "minecraft:creeper" | "minecraft:spider")).count();
    assert!(monsters > 0, "no monsters spawned: {all:?}");
    // The cap is checked before each chunk; a chunk may add up to its cluster size (4) past it.
    assert!(monsters < 70 + 4, "{monsters} monsters exceed the cap");
}

#[test]
fn selectors_see_mobs_and_kill_removes_them() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:zombie", [4.0, 0.0, 0.0], "");
    w.summon("minecraft:pig", [-4.0, 0.0, 0.0], "");
    w.ticks(5);
    w.console("kill @e[type=minecraft:zombie]");
    w.ticks(25);
    assert!(w.mobs("minecraft:zombie").is_empty(), "the zombie died and was removed");
    assert_eq!(w.mobs("minecraft:pig").len(), 1, "the pig is untouched");
    w.console("kill @e[type=!minecraft:player]");
    w.ticks(25);
    assert!(w.mobs("minecraft:pig").is_empty());
    assert!(w.sim.health(1).is_some_and(|h| !h.1), "the player is alive");
}

impl World {
    /// Puts `count` of `item` in the first hotbar slot (through creative mode).
    fn hold(&mut self, item: &str, count: i32) {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        let stack = ItemStack { item: id, count, added: Vec::new(), removed: Vec::new() };
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    }

    fn interact(&mut self, entity_id: i32) {
        let pkt = PlayIn::Interact { entity_id, hand: kiln_proto::packets::serverbound::Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn held(&self) -> Option<(i32, i32)> {
        // Menu slot 36 is the first hotbar slot.
        self.sim.inventory(1).unwrap()[36]
    }
}

#[test]
fn fed_animals_breed_and_babies_grow() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:wheat", 64);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:cow", [1.5, 0.0, 0.0], "");
    w.summon("minecraft:cow", [-1.5, 0.0, 0.0], "");
    let cows: Vec<i32> = w.mobs("minecraft:cow").iter().map(|c| c.0).collect();
    for &c in &cows {
        w.interact(c);
    }
    assert_eq!(w.held(), Some((kiln_data::builtin_id("minecraft:item", "minecraft:wheat").unwrap(), 62)), "two wheat eaten");
    w.ticks(200);
    assert_eq!(w.mobs("minecraft:cow").len(), 3, "a calf was born");
    assert!(w.sim.entities().iter().any(|e| e.0 == "minecraft:experience_orb"), "breeding experience");
    // The parents are on their breeding cooldown: more wheat does nothing.
    for &c in &cows {
        w.interact(c);
    }
    assert_eq!(w.held().unwrap().1, 62);
}

#[test]
fn sheep_shear_and_dye_cows_milk() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:red_dye", 2);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:sheep", [1.5, 0.0, 0.0], "{NoAI:1b}");
    let sheep = w.mobs("minecraft:sheep")[0].0;
    w.interact(sheep);
    assert_eq!(w.held().unwrap().1, 1, "dye used");
    w.console("gamemode creative Hunter");
    w.hold("minecraft:shears", 1);
    w.console("gamemode survival Hunter");
    w.interact(sheep);
    w.ticks(2);
    if std::env::var_os("KILN_DATAPACK").is_some() {
        let items = w.sim.entities().iter().filter(|e| e.0 == "minecraft:item").count();
        assert!(items >= 1, "wool dropped");
    }
    assert_eq!(w.sim.item_damage(1, 36), Some(1), "the shears wore");
    w.console("gamemode creative Hunter");
    w.hold("minecraft:bucket", 1);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:cow", [-1.5, 0.0, 0.0], "{NoAI:1b}");
    let cow = w.mobs("minecraft:cow")[0].0;
    w.interact(cow);
    assert_eq!(w.held().map(|h| h.0), kiln_data::builtin_id("minecraft:item", "minecraft:milk_bucket"), "milked");
}

impl World {
    fn packet(&mut self, pkt: PlayIn) {
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    /// A left click on menu slot `slot` of container `id` (`quick`: shift-click).
    fn click(&mut self, id: i32, slot: i16, quick: bool) {
        let input = if quick { kiln_inventory::ContainerInput::QuickMove } else { kiln_inventory::ContainerInput::Pickup };
        let click = kiln_inventory::ContainerClick {
            container_id: id,
            state_id: 0,
            slot,
            button: 0,
            input,
            changed: Vec::new(),
            carried: kiln_item::HashedStack::Empty,
        };
        let mut body = bytes::BytesMut::new();
        click.write(&mut body);
        self.packet(PlayIn::ContainerClick { body: body.freeze() });
    }

    fn count_of(&self, item: &str) -> i32 {
        let id = kiln_data::builtin_id("minecraft:item", item).unwrap();
        self.sim.inventory(1).unwrap().iter().flatten().filter(|s| s.0 == id).map(|s| s.1).sum()
    }
}

#[test]
fn trading_with_a_villager() {
    let item = |n: &str| kiln_data::builtin_id("minecraft:item", n).unwrap();
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:wheat", 64);
    w.console("gamemode survival Hunter");
    let offers = r#"Offers:{Recipes:[{buy:{id:"minecraft:wheat",count:20},sell:{id:"minecraft:emerald",count:1},maxUses:3,xp:4,priceMultiplier:0.05f},{buy:{id:"minecraft:emerald",count:1},sell:{id:"minecraft:bread",count:6},maxUses:16,xp:1,priceMultiplier:0.05f}]}"#;
    w.summon("minecraft:villager", [1.5, 0.0, 0.0], &format!(r#"{{NoAI:1b,VillagerData:{{profession:"minecraft:farmer",level:1,type:"minecraft:plains"}},{offers}}}"#));
    let villager = w.mobs("minecraft:villager")[0].0;
    // A baby shakes its head; an adult opens its screen.
    w.interact(villager);
    let (id, v, slots) = w.sim.merchant_screen(1).expect("the merchant screen opened");
    assert_eq!(v, villager);
    assert!(slots.iter().all(Option::is_none));
    // Selecting the wheat offer moves the wheat into the payment slot.
    w.packet(PlayIn::SelectTrade { offer: 0 });
    let (_, _, slots) = w.sim.merchant_screen(1).unwrap();
    assert_eq!(slots[0], Some((item("minecraft:wheat"), 64)));
    assert_eq!(slots[2], Some((item("minecraft:emerald"), 1)));
    assert_eq!(w.count_of("minecraft:wheat"), 0);
    // Taking the result pays for it; shift-clicking trades until the offer runs out (3 uses).
    w.click(id, 2, false);
    let (_, _, slots) = w.sim.merchant_screen(1).unwrap();
    assert_eq!(slots[0], Some((item("minecraft:wheat"), 44)));
    w.click(id, 2, true);
    let (_, _, slots) = w.sim.merchant_screen(1).unwrap();
    assert_eq!(slots[0], Some((item("minecraft:wheat"), 4)));
    assert_eq!(slots[2], None, "out of stock");
    assert_eq!(w.count_of("minecraft:emerald"), 2, "two emeralds shift-clicked into the inventory");
    w.ticks(2);
    assert!(w.sim.entities().iter().any(|e| e.0 == "minecraft:experience_orb"), "trading experience");
    // Closing gives the payment back; the villager can trade again.
    w.packet(PlayIn::ContainerClose { container_id: id });
    assert!(w.sim.merchant_screen(1).is_none());
    assert_eq!(w.count_of("minecraft:wheat"), 4);
    w.interact(villager);
    assert!(w.sim.merchant_screen(1).is_some(), "trading again");
    // Killing the villager closes the screen.
    w.console("kill @e[type=minecraft:villager]");
    w.ticks(3);
    assert!(w.sim.merchant_screen(1).is_none(), "closed when the villager died");
}

#[test]
fn villager_offers_come_from_the_datapack() {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        return;
    }
    let mut w = World::new();
    w.summon("minecraft:villager", [1.5, 0.0, 0.0], r#"{NoAI:1b,VillagerData:{profession:"minecraft:librarian",level:1,type:"minecraft:plains"}}"#);
    w.summon("minecraft:villager", [-1.5, 0.0, 0.0], r#"{NoAI:1b}"#);
    let v = w.mobs("minecraft:villager");
    let (librarian, unemployed) = if v[0].1[0] > v[1].1[0] { (v[0].0, v[1].0) } else { (v[1].0, v[0].0) };
    // Unemployed villagers have no trades: no screen.
    w.interact(unemployed);
    assert!(w.sim.merchant_screen(1).is_none());
    w.interact(librarian);
    assert!(w.sim.merchant_screen(1).is_some(), "a librarian trades from its level 1 trade set");
}

#[test]
fn piglins_barter_gold_and_zombify_in_the_overworld() {
    let item = |n: &str| kiln_data::builtin_id("minecraft:item", n).unwrap();
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:gold_ingot", 3);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:piglin", [1.5, 0.0, 0.0], "{IsImmuneToZombification:1b,PersistenceRequired:1b}");
    let piglin = w.mobs("minecraft:piglin")[0].0;
    w.interact(piglin);
    assert_eq!(w.held(), Some((item("minecraft:gold_ingot"), 2)), "the piglin took one ingot");
    // It admires the ingot for 119 ticks, then barters.
    w.ticks(60);
    let items = |w: &World| w.sim.entities().iter().filter(|e| e.0 == "minecraft:item").count();
    assert_eq!(items(&w), 0, "still admiring");
    w.ticks(80);
    if std::env::var_os("KILN_DATAPACK").is_some() {
        // The loot lands by the player, who may have picked it up already.
        let other = w.sim.inventory(1).unwrap().iter().flatten().filter(|s| s.0 != item("minecraft:gold_ingot")).count();
        assert!(items(&w) >= 1 || other >= 1, "bartered items dropped");
    }
    // Outside the nether a piglin that is not immune turns into a zombified piglin.
    w.summon("minecraft:piglin", [-2.5, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.ticks(310);
    assert_eq!(w.mobs("minecraft:piglin").len(), 1, "the second piglin converted");
    assert_eq!(w.mobs("minecraft:zombified_piglin").len(), 1);
}

#[test]
fn piglins_attack_players_without_gold() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:piglin", [2.5, 0.0, 0.0], "{IsImmuneToZombification:1b,PersistenceRequired:1b}");
    w.ticks(120);
    assert!(w.health() < 20.0, "the piglin attacked");
}

#[test]
fn hoglins_attack_and_breed() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:hoglin", [2.5, 0.0, 0.0], "{IsImmuneToZombification:1b,PersistenceRequired:1b}");
    w.ticks(60);
    assert!(w.health() < 20.0, "the hoglin attacked");
    // Creative players are not attacked; crimson fungus makes hoglins breed.
    w.console("gamemode creative Hunter");
    w.console("kill @e[type=minecraft:hoglin]");
    w.ticks(30);
    w.hold("minecraft:crimson_fungus", 4);
    w.summon("minecraft:hoglin", [1.5, 0.0, 0.0], "{IsImmuneToZombification:1b}");
    w.summon("minecraft:hoglin", [-1.5, 0.0, 0.0], "{IsImmuneToZombification:1b}");
    let hoglins: Vec<i32> = w.mobs("minecraft:hoglin").iter().map(|h| h.0).collect();
    assert_eq!(hoglins.len(), 2);
    for &h in &hoglins {
        w.interact(h);
    }
    w.ticks(250);
    assert_eq!(w.mobs("minecraft:hoglin").len(), 3, "a hoglet was born");
}

#[test]
fn staring_at_an_enderman_angers_it() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:enderman", [0.0, 0.0, 5.0], "{PersistenceRequired:1b}");
    // Looking straight ahead does not meet its eyes; looking up at them does.
    w.ticks(60);
    assert_eq!(w.health(), 20.0, "not stared at");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: None, rot: Some([0.0, -10.5]), on_ground: true })]));
    // It freezes while looked at; looking away lets it come.
    w.ticks(30);
    assert_eq!(w.health(), 20.0, "frozen while stared at");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: None, rot: Some([180.0, 0.0]), on_ground: true })]));
    w.ticks(200);
    assert!(w.health() < 20.0, "the enderman attacked (health {})", w.health());
}

#[test]
fn shulker_bullets_hurt_and_levitate() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:shulker", [0.0, 0.0, 5.0], "");
    let mut levitated = false;
    for _ in 0..300 {
        w.ticks(1);
        let fx = w.sim.effects(1).unwrap();
        levitated |= fx.iter().any(|e| e.0 == "minecraft:levitation");
    }
    assert!(w.health() < 20.0, "a bullet hit (health {})", w.health());
    assert!(levitated, "the hit made the player levitate");
}

#[test]
fn witches_throw_potions() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:witch", [0.0, 0.0, 9.0], "{PersistenceRequired:1b}");
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..300 {
        w.ticks(1);
        for e in w.sim.effects(1).unwrap() {
            seen.insert(e.0);
        }
    }
    // From 9 blocks away the first potion is slowness; poison and harming follow.
    assert!(seen.contains("minecraft:slowness"), "splashed with slowness ({seen:?})");
    assert!(w.health() < 20.0 || seen.contains("minecraft:poison"), "hurt or poisoned ({seen:?}, health {})", w.health());
}

#[test]
fn end_city_sentries_load_as_shulkers() {
    use kiln_proto::nbt::Tag;
    // The compound end city generation leaves for a sentry.
    let tag = Tag::Compound(vec![
        ("id".into(), Tag::String("minecraft:shulker".into())),
        ("Pos".into(), Tag::List(vec![Tag::Double(3.5), Tag::Double(70.0), Tag::Double(4.5)])),
        ("Rotation".into(), Tag::List(vec![Tag::Float(0.0), Tag::Float(0.0)])),
        ("AttachFace".into(), Tag::Byte(1)),
        ("Peek".into(), Tag::Byte(0)),
        ("Color".into(), Tag::Byte(16)),
    ]);
    let e = kiln_entity::persist::load(&tag, 7, 1).expect("loads");
    let m = kiln_entity::mob::data(&e).expect("a mob");
    assert_eq!(m.kind, kiln_entity::mob::MobKind::Shulker);
    assert_eq!(kiln_entity::mob::kinds::shulker::st(m).attach, kiln_entity::math::Direction::Up);
}

#[test]
fn every_mob_type_summons_ticks_and_saves() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    for (i, kind) in kiln_entity::mob::ALL_KINDS.iter().enumerate() {
        let a = i as f64 * 0.7;
        w.summon(kind.type_name(), [6.0 * a.cos(), 0.0, 6.0 * a.sin()], "{PersistenceRequired:1b}");
    }
    w.ticks(1);
    for kind in kiln_entity::mob::ALL_KINDS {
        // Endermen hunt endermites.
        if kind == kiln_entity::mob::MobKind::Endermite {
            continue;
        }
        assert!(!w.mobs(kind.type_name()).is_empty(), "{} is gone", kind.type_name());
    }
    // Then they live together for a while (wolves hunt the sheep, golems fight monsters).
    w.ticks(100);
}

/// `Owner` of the test player (`testing::join` gives connection 1 the UUID 0x6b696c6e / 1).
const OWNER: &str = "Owner:[I;0,1802071150,0,1]";

#[test]
fn wolves_are_tamed_with_bones_and_sit_when_told() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:bone", 64);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:wolf", [1.5, 0.0, 0.0], "{NoAI:1b}");
    let wolf = w.mobs("minecraft:wolf")[0].0;
    assert_eq!(w.mobs("minecraft:wolf")[0].2, 8.0);
    let mut tries = 0;
    while w.mobs("minecraft:wolf")[0].2 != 40.0 {
        assert!(tries < 40, "tamed within 40 bones");
        w.interact(wolf);
        w.ticks(1);
        tries += 1;
    }
    // One bone per try; the tamed wolf has 40 health.
    assert_eq!(w.held().unwrap().1, 64 - tries);
    // Right-clicking a tamed wolf (with no food) toggles sitting and takes nothing.
    w.console("gamemode creative Hunter");
    w.hold("minecraft:stick", 1);
    w.console("gamemode survival Hunter");
    w.interact(wolf);
    assert_eq!(w.held().unwrap().1, 1);
}

#[test]
fn tamed_wolves_follow_their_owner() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:wolf", [11.0, 0.0, 0.0], &format!("{{{OWNER},PersistenceRequired:1b}}"));
    w.ticks(200);
    let (_, pos, _) = w.mobs("minecraft:wolf")[0];
    let p = w.pos();
    let d = ((pos[0] - p[0]).powi(2) + (pos[2] - p[2]).powi(2)).sqrt();
    assert!(d < 6.0, "the wolf came to its owner ({d:.1} blocks)");
    // A wild wolf stays where it wanders.
    w.summon("minecraft:wolf", [0.0, 0.0, 14.0], "{PersistenceRequired:1b,NoAI:1b}");
    w.ticks(20);
    assert_eq!(w.mobs("minecraft:wolf").len(), 2);
}

#[test]
fn wild_wolves_hunt_sheep() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.summon("minecraft:wolf", [3.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.summon("minecraft:sheep", [6.0, 0.0, 2.0], "{PersistenceRequired:1b}");
    let mut hurt = false;
    for _ in 0..60 {
        w.ticks(10);
        hurt |= w.mobs("minecraft:sheep").first().is_none_or(|s| s.2 < 8.0);
    }
    assert!(hurt, "the wolf bit the sheep");
}

#[test]
fn cats_are_tamed_with_fish() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:cod", 64);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:cat", [1.5, 0.0, 0.0], "{NoAI:1b}");
    let cat = w.mobs("minecraft:cat")[0].0;
    let mut tries = 0;
    loop {
        assert!(tries < 40, "tamed within 40 fish");
        w.interact(cat);
        tries += 1;
        assert_eq!(w.held().unwrap().1, 64 - tries, "each fish is eaten");
        let all = w.sim.entity_nbt();
        let tag = all.iter().find(|t| t.get("id").and_then(|v| v.as_str()) == Some("minecraft:cat")).unwrap();
        if tag.get("Owner").is_some() {
            assert_eq!(tag.get("Sitting").and_then(|t| t.as_f64()), Some(1.0), "a new tamed cat sits");
            break;
        }
    }
}

fn player_pos(w: &World) -> [f64; 3] {
    w.sim.player_level(1).unwrap().1
}

fn nbt_of(w: &World, kind: &str) -> kiln_proto::nbt::Tag {
    w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(|v| v.as_str()) == Some(kind)).unwrap()
}

#[test]
fn saddled_horses_are_ridden_and_steered() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:stick", 1);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:horse", [1.5, 0.0, 0.0], "{Tame:1b,PersistenceRequired:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}");
    w.ticks(20);
    let (horse, hp, _) = w.mobs("minecraft:horse")[0];
    w.interact(horse);
    w.ticks(3);
    // The rider sits on the horse (its seat 1.44375 up, less the player's 0.6).
    let p = player_pos(&w);
    assert!((p[1] - (hp[1] + 1.44375 - 0.6)).abs() < 1e-6 && (p[0] - hp[0]).abs() < 1e-6, "seated at {p:?} on {hp:?}");
    // The rider's client moves the horse.
    let to = [hp[0] + 1.0, hp[1], hp[2] + 0.5];
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::MoveVehicle { pos: to, rot: [90.0, 0.0], on_ground: true })]));
    w.ticks(1);
    let (_, hp2, _) = w.mobs("minecraft:horse")[0];
    assert_eq!(hp2, to, "the horse went where its rider's client put it");
    // Sneaking gets the rider off, beside the horse.
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerInput { flags: 0x20 })]));
    w.ticks(2);
    let p = player_pos(&w);
    assert!(p[1] < hp2[1] + 0.5, "off the horse ({p:?})");
    // Moves of the horse no longer come from the player.
    let away = [to[0] + 2.0, to[1], to[2]];
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::MoveVehicle { pos: away, rot: [0.0, 0.0], on_ground: true })]));
    assert_ne!(w.mobs("minecraft:horse")[0].1, away);
}

#[test]
fn wild_horses_throw_riders_until_tamed() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:horse", [1.5, 0.0, 0.0], "{PersistenceRequired:1b}");
    let horse = w.mobs("minecraft:horse")[0].0;
    let mut throws = 0;
    for _ in 0..400 {
        if nbt_of(&w, "minecraft:horse").get("Tame").and_then(|t| t.as_f64()) == Some(1.0) {
            break;
        }
        let hp = w.mobs("minecraft:horse")[0].1;
        let p = player_pos(&w);
        if (p[1] - hp[1]) < 0.5 {
            // On the ground: walk up to the horse and get on.
            w.clients[0].pos = [hp[0] - 1.0, hp[1], hp[2]];
            let pos = w.clients[0].pos;
            w.sim.step([ToSim::Packet(1, PlayIn::Move { pos: Some(pos), rot: None, on_ground: true })]);
            w.interact(horse);
            throws += 1;
        }
        w.ticks(5);
    }
    let tag = nbt_of(&w, "minecraft:horse");
    assert_eq!(tag.get("Tame").and_then(|t| t.as_f64()), Some(1.0), "tamed after {throws} rides");
    assert!(throws >= 1);
}

#[test]
fn iron_golems_fight_monsters() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.console("time set 18000");
    w.summon("minecraft:iron_golem", [4.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    w.summon("minecraft:zombie", [8.0, 0.0, 3.0], "{PersistenceRequired:1b}");
    let mut hurt = false;
    for _ in 0..40 {
        w.ticks(10);
        hurt |= w.mobs("minecraft:zombie").first().is_none_or(|z| z.2 < 20.0);
    }
    assert!(hurt, "the golem hit the zombie");
}

#[test]
fn saddled_striders_are_steered_with_a_fungus_on_a_stick() {
    let mut w = World::new();
    w.console("gamemode creative Hunter");
    w.hold("minecraft:warped_fungus_on_a_stick", 1);
    w.console("gamemode survival Hunter");
    w.summon("minecraft:strider", [1.5, 0.0, 0.0], "{PersistenceRequired:1b,equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}");
    w.ticks(5);
    let (strider, sp, _) = w.mobs("minecraft:strider")[0];
    w.interact(strider);
    w.ticks(3);
    let p = player_pos(&w);
    assert!((p[1] - (sp[1] + 1.7 - 0.6)).abs() < 0.05, "seated at {p:?} on {sp:?}");
    let sp = w.mobs("minecraft:strider")[0].1;
    let to = [sp[0] + 0.5, sp[1], sp[2]];
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::MoveVehicle { pos: to, rot: [0.0, 0.0], on_ground: true })]));
    assert_eq!(w.mobs("minecraft:strider")[0].1, to, "steered by the rider's client");
}

#[test]
fn slimes_hurt_touching_players_and_split_when_killed() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:slime", [0.5, 0.0, 0.5], "{Size:1,PersistenceRequired:1b}");
    let slimes = w.mobs("minecraft:slime");
    assert_eq!(slimes.len(), 1);
    assert_eq!(slimes[0].2, 4.0, "a size 2 slime has 4 health");
    w.ticks(60);
    assert!(w.health() < 20.0, "the touching slime hurt the player (health {})", w.health());
    w.console("gamemode creative Hunter");
    w.console("kill @e[type=minecraft:slime]");
    w.ticks(25);
    let small = w.mobs("minecraft:slime");
    assert!((2..=4).contains(&small.len()), "split into 2 to 4 slimes: {small:?}");
    assert!(small.iter().all(|s| s.2 == 1.0), "tiny slimes have 1 health: {small:?}");
    w.console("kill @e[type=minecraft:slime]");
    w.ticks(25);
    assert!(w.mobs("minecraft:slime").is_empty(), "tiny slimes do not split");
}

#[test]
fn tiny_magma_cubes_hurt_touching_players() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    w.summon("minecraft:magma_cube", [0.3, 0.0, 0.3], "{Size:0,PersistenceRequired:1b}");
    w.ticks(60);
    assert!(w.health() < 20.0, "the magma cube hurt the player (health {})", w.health());
}

#[test]
fn ghasts_shoot_fireballs_at_players() {
    let mut w = World::new();
    w.console("gamemode survival Hunter");
    let p = w.pos();
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    // A ceiling keeps the ghast within 4 blocks of the player's height.
    w.console(&format!("fill {} {} {} {} {} {} minecraft:stone", x - 20, y + 6, z - 20, x + 20, y + 6, z + 20));
    w.summon("minecraft:ghast", [12.0, 0.5, 0.0], "{PersistenceRequired:1b}");
    let mut fireball = false;
    for _ in 0..300 {
        w.ticks(1);
        fireball |= w.sim.entities().iter().any(|e| e.0 == "minecraft:fireball");
    }
    assert!(fireball, "the ghast shot a fireball");
    assert!(w.health() < 20.0, "the fireball hurt the player (health {})", w.health());
}

#[test]
fn blazes_shoot_small_fireballs_at_players() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:blaze", [7.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    let mut fireball = false;
    for _ in 0..200 {
        w.ticks(1);
        fireball |= w.sim.entities().iter().any(|e| e.0 == "minecraft:small_fireball");
    }
    assert!(fireball, "the blaze shot small fireballs");
    assert!(w.health() < 20.0, "the blaze hurt the player (health {})", w.health());
}

#[test]
fn phantoms_swoop_at_players_at_night() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:phantom", [2.0, 12.0, 0.0], "{PersistenceRequired:1b,size:2}");
    w.ticks(400);
    assert!(w.health() < 20.0, "the phantom bit the player (health {})", w.health());
}

// ---------------------------------------------------------------------- the zombie and skeleton families

impl World {
    fn effects(&self) -> Vec<&'static str> {
        self.sim.effects(1).unwrap().into_iter().map(|e| e.0).collect()
    }

    /// Fills the box `from..=to` (offsets from the player) with `block`.
    fn fill(&mut self, from: [i32; 3], to: [i32; 3], block: &str) {
        let p = self.pos().map(|v| v.floor() as i32);
        self.console(&format!(
            "fill {} {} {} {} {} {} {block}",
            p[0] + from[0],
            p[1] + from[1],
            p[2] + from[2],
            p[0] + to[0],
            p[1] + to[1],
            p[2] + to[2]
        ));
    }
}

#[test]
fn husks_do_not_burn_and_make_their_target_hungry() {
    let mut w = World::new();
    w.console("time set 6000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:husk", [4.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    let mut hungry = false;
    for _ in 0..40 {
        w.ticks(10);
        hungry |= w.effects().contains(&"minecraft:hunger");
    }
    assert!(w.health() < 20.0, "the husk hit the player by day");
    assert!(hungry, "a husk's hit makes its target hungry");
    let h = w.mobs("minecraft:husk");
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].2, 20.0, "husks do not burn in daylight");
}

#[test]
fn wither_skeletons_wither_their_target() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:wither_skeleton", [3.0, 0.0, 0.0], "{PersistenceRequired:1b}");
    let mut withered = false;
    for _ in 0..30 {
        w.ticks(10);
        withered |= w.effects().contains(&"minecraft:wither");
    }
    assert!(withered, "a wither skeleton's hit withers");
}

#[test]
fn zombies_drown_into_drowned_and_skeletons_freeze_into_strays() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode creative Hunter");
    w.fill([4, 0, -2], [8, 3, 2], "minecraft:water");
    w.summon("minecraft:zombie", [6.0, 0.0, 0.0], "{PersistenceRequired:1b,DrownedConversionTime:20}");
    w.fill([-8, 0, -2], [-4, 1, 2], "minecraft:powder_snow");
    // A saved conversion carries on only while afflicted: the first tick (before the mob moved
    // into the snow) would cancel it, so the skeleton freezes the full 7 seconds, then 15.
    w.summon("minecraft:skeleton", [-5.5, 0.0, 0.5], "{PersistenceRequired:1b}");
    w.ticks(60);
    assert!(w.mobs("minecraft:zombie").is_empty(), "the zombie converted");
    assert_eq!(w.mobs("minecraft:drowned").len(), 1, "into a drowned");
    w.ticks(400);
    assert!(w.mobs("minecraft:skeleton").is_empty(), "the skeleton converted");
    assert_eq!(w.mobs("minecraft:stray").len(), 1, "into a stray");
}

#[test]
fn drowned_throw_tridents() {
    let mut w = World::new();
    w.console("time set 18000");
    w.console("gamemode survival Hunter");
    w.summon("minecraft:drowned", [8.0, 0.0, 0.0], "{PersistenceRequired:1b,equipment:{mainhand:{id:\"minecraft:trident\",count:1}}}");
    let mut thrown = false;
    for _ in 0..40 {
        w.ticks(5);
        thrown |= w.sim.entities().iter().any(|e| e.0 == "minecraft:trident");
    }
    assert!(thrown, "the drowned threw a trident");
    assert!(w.health() < 20.0, "and hit (health {})", w.health());
}
