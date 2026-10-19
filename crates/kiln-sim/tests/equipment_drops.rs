//! wp85: the `equipment_drops` enchantment effect (looting is +1% a level of the chance that a killed mob drops its equipment): a data
//! pack's enchantment with the effect makes every zombie killed with it drop its sword, where a plain sword leaves them at the usual 8.5%.

use kiln_link::{PlayIn, ToSim};
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::path::PathBuf;

const PLUNDER: &str = r#"{
  "description": {"text": "Plunder"},
  "supported_items": "#minecraft:enchantable/melee_weapon",
  "weight": 1,
  "max_level": 1,
  "min_cost": {"base": 1, "per_level_above_first": 0},
  "max_cost": {"base": 50, "per_level_above_first": 0},
  "anvil_cost": 1,
  "slots": ["mainhand"],
  "effects": {
    "minecraft:equipment_drops": [{
      "effect": {"type": "minecraft:add", "value": {"type": "minecraft:linear", "base": 1.0, "per_level_above_first": 0.0}},
      "enchanted": "attacker",
      "requirements": {"condition": "minecraft:entity_properties", "entity": "attacker", "predicate": {"type": "minecraft:player"}}
    }]
  }
}"#;

struct World {
    sim: Sim,
    client: Client,
    dir: PathBuf,
}

impl World {
    fn new() -> World {
        let dir = std::env::temp_dir().join(format!("kiln-equipment-drops-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pack = dir.join("datapacks/p85");
        std::fs::create_dir_all(pack.join("data/kiln85/enchantment")).unwrap();
        std::fs::write(pack.join("data/kiln85/enchantment/plunder.json"), PLUNDER).unwrap();
        std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"description":"p85","min_format":121,"max_format":121}}"#).unwrap();
        let mut sim = Sim::new(SimConfig::new(4, 3, Some(dir.clone())));
        let (msg, stats) = join(1, "User", 2);
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut w = World { sim, client: Client::new(1, stats), dir };
        w.ticks(5);
        w.sim.console_now("gamemode survival User");
        w.sim.console_now("gamerule minecraft:natural_health_regeneration false");
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    /// Zombies with an iron sword killed one after the other with the diamond sword the player holds: how many dropped the iron sword.
    fn kills(&mut self, n: usize) -> usize {
        let p = self.client.pos;
        let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
        let mut swords = 0;
        for _ in 0..n {
            self.sim.console_now("kill @e[type=minecraft:item]");
            self.sim.console_now(&format!(
                "summon minecraft:zombie {} {y} {z} {{NoAI:1b,Health:1.0f,equipment:{{mainhand:{{id:\"minecraft:iron_sword\"}}}},Invulnerable:0b}}",
                x + 2
            ));
            self.ticks(25);
            let id = self.sim.entity_ids_of("minecraft:zombie")[0];
            assert!(self.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id })]));
            self.ticks(3);
            swords += self.sim.item_stacks().iter().filter(|s| s.item_name() == "minecraft:iron_sword").count();
            self.sim.console_now("kill @e[type=minecraft:zombie]");
            self.ticks(1);
        }
        swords
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn an_enchantment_with_equipment_drops_raises_the_chance_a_killed_mob_drops_its_equipment() {
    let _x = kiln_data::live::exclusive();
    let mut w = World::new();
    w.sim.console_now("give User minecraft:diamond_sword");
    w.ticks(2);
    let plain = w.kills(12);
    assert!(plain < 12, "8.5% each: {plain} of 12");
    w.sim.console_now("clear User");
    w.sim.console_now("give User minecraft:diamond_sword[enchantments={\"kiln85:plunder\":1}]");
    w.ticks(2);
    let plunder = w.kills(12);
    assert_eq!(plunder, 12, "the effect adds 1.0 to the chance (a plain sword dropped {plain})");
}
