//! wp36: leftovers of the mobs and containers packages, in the whole simulation. A zombie that
//! kills a villager turns it into a zombie villager (`Zombie.killedEntity`), an enderman's carried
//! block drops by its loot table.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new(difficulty: &str) -> World {
        let mut sim = Sim::new(SimConfig::new(8, 4, None));
        let (msg, stats) = join(1, "Target", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        let difficulty = format!("difficulty {difficulty}");
        for c in ["gamerule minecraft:spawn_mobs false", difficulty.as_str(), "time set 18000", "gamerule minecraft:advance_time false", "gamemode creative Target"] {
            w.console(c);
        }
        w.ticks(5);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn console(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    fn summon(&mut self, entity: &str, offset: [f64; 3], nbt: &str) {
        let p = self.client.pos;
        self.console(&format!("summon {entity} {} {} {} {nbt}", p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]));
        self.ticks(1);
    }

    fn count(&self, kind: &str) -> usize {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).count()
    }

    fn nbt_of(&self, kind: &str) -> Vec<Tag> {
        self.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some(kind)).collect()
    }
}

const VILLAGER: &str = r#"{NoAI:1b,PersistenceRequired:1b,Health:1f,Xp:70,VillagerData:{profession:"minecraft:librarian",type:"minecraft:desert",level:3},Offers:{Recipes:[{buy:{id:"minecraft:paper",count:24},sell:{id:"minecraft:emerald",count:1},maxUses:16,uses:3,xp:2,priceMultiplier:0.05f,demand:1,specialPrice:0}]},Gossips:[{Type:"major_positive",Value:20,Target:[I;1,2,3,4]}]}"#;

/// On hard difficulty every villager a zombie kills becomes a zombie villager that keeps its data,
/// trades, gossips and experience.
#[test]
fn a_zombie_turns_the_villager_it_kills_into_a_zombie_villager_on_hard() {
    let mut w = World::new("hard");
    w.summon("minecraft:villager", [0.0, 0.0, 0.0], VILLAGER);
    assert_eq!(w.count("minecraft:villager"), 1);
    w.summon("minecraft:zombie", [3.0, 0.0, 0.0], r#"{PersistenceRequired:1b}"#);
    for _ in 0..200 {
        w.ticks(1);
        if w.count("minecraft:villager") == 0 {
            break;
        }
    }
    w.ticks(2);
    assert_eq!(w.count("minecraft:villager"), 0, "the villager died");
    let zv = w.nbt_of("minecraft:zombie_villager");
    assert_eq!(zv.len(), 1, "and came back as a zombie villager");
    let zv = &zv[0];
    let data = zv.get("VillagerData").expect("VillagerData");
    assert_eq!(data.get("profession").and_then(Tag::as_str), Some("minecraft:librarian"));
    assert_eq!(data.get("type").and_then(Tag::as_str), Some("minecraft:desert"));
    assert_eq!(data.get("level").and_then(Tag::as_i64), Some(3));
    assert_eq!(zv.get("Xp").and_then(Tag::as_i64), Some(70));
    let recipes = zv.get("Offers").and_then(|o| o.get("Recipes")).and_then(Tag::as_list).expect("Offers");
    assert_eq!(recipes.len(), 1);
    assert_eq!(recipes[0].get("uses").and_then(Tag::as_i64), Some(3));
    let gossips = zv.get("Gossips").and_then(Tag::as_list).expect("Gossips");
    assert_eq!(gossips.len(), 1);
    assert_eq!(gossips[0].get("Type").and_then(Tag::as_str), Some("major_positive"));
    assert_eq!(gossips[0].get("Value").and_then(Tag::as_i64), Some(20));
    // Not a cured one: no countdown.
    assert_eq!(zv.get("ConversionTime").and_then(Tag::as_i64), Some(-1));
    // The killing zombie still stands, and no villager's loot lies around (no emeralds either).
    assert_eq!(w.count("minecraft:zombie"), 1);
}

/// On normal difficulty half of them do; on easy none.
#[test]
fn on_normal_about_half_and_on_easy_none() {
    for (difficulty, lo, hi) in [("normal", 1, 11), ("easy", 0, 0)] {
        let mut w = World::new(difficulty);
        for i in 0..12 {
            w.summon("minecraft:villager", [i as f64 * 2.0, 0.0, 6.0], r#"{NoAI:1b,PersistenceRequired:1b,Health:1f}"#);
            w.summon("minecraft:zombie", [i as f64 * 2.0, 0.0, 4.0], r#"{PersistenceRequired:1b}"#);
        }
        for _ in 0..300 {
            w.ticks(1);
            if w.count("minecraft:villager") == 0 {
                break;
            }
        }
        w.ticks(2);
        assert_eq!(w.count("minecraft:villager"), 0, "{difficulty}: all villagers died");
        let converted = w.count("minecraft:zombie_villager");
        assert!((lo..=hi).contains(&converted), "{difficulty}: {converted} of 12 converted");
    }
}

fn have_datapack() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        if let Some(dir) = std::env::var_os("KILN_DATAPACK") {
            return std::path::Path::new(&dir).join("data/minecraft/loot_table").is_dir();
        }
        eprintln!("no vanilla datapack (KILN_DATAPACK): skipped");
        false
    })
}

impl World {
    /// The `id`s of the item entities lying around.
    fn items(&self) -> Vec<String> {
        self.nbt_of("minecraft:item")
            .iter()
            .filter_map(|t| t.get("Item"))
            .flat_map(|i| {
                let id = i.get("id").and_then(Tag::as_str).unwrap_or("").to_owned();
                std::iter::repeat_n(id, i.get("count").and_then(Tag::as_i64).unwrap_or(1) as usize)
            })
            .collect()
    }
}

/// The carried block drops by its loot table as mined by a silk touch diamond axe
/// (`Enderman.dropCustomDeathLoot`): gravel as gravel, never as flint, and short grass (which wants
/// shears) not at all.
#[test]
fn an_endermans_carried_block_drops_by_its_loot_table() {
    if !have_datapack() {
        return;
    }
    let mut w = World::new("normal");
    for i in 0..30 {
        w.summon("minecraft:enderman", [i as f64 * 0.5, 0.0, 10.0], r#"{PersistenceRequired:1b,NoAI:1b,carriedBlockState:{Name:"minecraft:gravel"}}"#);
    }
    w.summon("minecraft:enderman", [0.0, 0.0, 14.0], r#"{PersistenceRequired:1b,NoAI:1b,carriedBlockState:{Name:"minecraft:short_grass"}}"#);
    w.console("kill @e[type=minecraft:enderman]");
    w.ticks(25);
    assert_eq!(w.count("minecraft:enderman"), 0);
    let items = w.items();
    let gravel = items.iter().filter(|i| *i == "minecraft:gravel").count();
    assert_eq!(gravel, 30, "{items:?}");
    assert!(!items.iter().any(|i| i == "minecraft:flint" || i == "minecraft:short_grass"), "{items:?}");
}
