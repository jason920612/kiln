//! wp34: mobs spawn with their equipment enchanted by the datapack's providers
//! (`Mob.populateDefaultEquipmentEnchantments`, raids' crossbows and axes), end to end through the
//! simulation. The draws and results are compared with vanilla by kiln-entity's
//! `finalize_parity` (`finalize_hard`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

/// The datapack (`work/generated` unless `KILN_DATAPACK` says otherwise); false when there is none.
fn datapack() -> bool {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let work = std::env::var_os("KILN_WORK").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data/minecraft/enchantment_provider").is_dir() {
            return false;
        }
        unsafe { std::env::set_var("KILN_DATAPACK", dir) };
    }
    true
}

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(6, 4, None));
        let (msg, stats) = join(1, "Smith", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        for c in ["gamerule minecraft:spawn_mobs false", "gamemode creative Smith", "difficulty hard"] {
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
}

/// The enchantments of an entity's main hand (`id` -> level), if it holds a stack.
fn main_hand_enchantments(t: &Tag) -> Option<Vec<(String, i64)>> {
    let item = t.get("equipment")?.get("mainhand")?;
    Some(match item.get("components").and_then(|c| c.get("minecraft:enchantments")) {
        Some(Tag::Compound(levels)) => levels.iter().map(|(k, v)| (k.clone(), v.as_i64().unwrap_or(0))).collect(),
        _ => Vec::new(),
    })
}

#[test]
fn summoned_pillagers_get_enchanted_crossbows_from_the_datapack() {
    if !datapack() {
        eprintln!("no datapack: skipped");
        return;
    }
    let mut w = World::new();
    let p = w.client.pos;
    // On hard at the start of a world the special multiplier is 0.125: about 3% of the crossbows.
    for i in 0..600 {
        w.console(&format!("summon minecraft:pillager {} {} {}", p[0] + 3.0 + (i % 40) as f64 * 0.5, p[1], p[2] + 3.0 + (i / 40) as f64 * 0.5));
        if i % 20 == 19 {
            w.ticks(1);
        }
    }
    w.ticks(3);
    let pillagers: Vec<Tag> = w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:pillager")).collect();
    assert!(pillagers.len() > 400, "{} pillagers", pillagers.len());
    let enchanted: Vec<Vec<(String, i64)>> = pillagers.iter().filter_map(main_hand_enchantments).filter(|e| !e.is_empty()).collect();
    assert!(!enchanted.is_empty(), "some of {} crossbows are enchanted", pillagers.len());
    // What a crossbow can get: never a sword's or a bow's enchantments.
    let valid = ["minecraft:quick_charge", "minecraft:multishot", "minecraft:piercing", "minecraft:unbreaking", "minecraft:mending", "minecraft:vanishing_curse"];
    for e in &enchanted {
        assert!(e.iter().all(|(id, level)| valid.contains(&id.as_str()) && *level >= 1), "{e:?}");
    }
}
