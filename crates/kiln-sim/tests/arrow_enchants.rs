//! wp78 (found with two real clients side by side, a Power V bow against a Wither: vanilla's arrows did 15..23 each, Kiln's 6..10): the
//! enchantments of a bow read the arrow that hit as the `direct_attacker` (`#minecraft:arrows`), which an item context cannot, so Power and Punch
//! did nothing. They are worked out when the arrow hits now, the way `AbstractArrow.onHitEntity` / `doKnockback` do.

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

fn datapack() -> bool {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        let work = std::env::var_os("KILN_WORK").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work"));
        let dir = work.join("generated");
        if !dir.join("data/minecraft/enchantment").is_dir() {
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
        let (msg, stats) = join(1, "Archer", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        for c in ["gamerule minecraft:spawn_mobs false", "gamemode creative Archer", "difficulty normal"] {
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

    /// Health of the golem after an arrow (fired from `bow` NBT, or none) flew into it.
    fn golem_after_arrow(&mut self, weapon: &str) -> (f32, f64) {
        let p = self.client.pos;
        let (x, y, z) = (p[0] + 12.0, p[1], p[2]);
        self.console(&format!("summon minecraft:iron_golem {x} {y} {z} {{NoAI:1b,PersistenceRequired:1b}}"));
        self.ticks(2);
        self.console(&format!("summon minecraft:arrow {} {} {z} {{Motion:[3.0d,0.0d,0.0d],crit:0b,damage:2.0d{weapon}}}", x - 6.0, y + 1.0));
        self.ticks(8);
        let hp = self.sim.mobs().into_iter().find(|m| m.1 == "minecraft:iron_golem").map(|m| m.3).expect("the golem");
        let motion_x = self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:iron_golem")).and_then(|t| match t.get("Motion") {
            Some(Tag::List(l)) => l.first().and_then(|d| match d {
                Tag::Double(v) => Some(*v),
                _ => None,
            }),
            _ => None,
        });
        (hp, motion_x.unwrap_or(0.0))
    }
}

fn bow(enchantments: &str) -> String {
    format!(",weapon:{{id:\"minecraft:bow\",count:1,components:{{\"minecraft:enchantments\":{{{enchantments}}}}}}}")
}

#[test]
fn power_adds_to_the_arrows_damage_when_it_hits() {
    if !datapack() {
        eprintln!("no datapack: skipped");
        return;
    }
    // 2.0 base * speed ~3 = 6; Power V adds 1 + 4 * 0.5 = 3 to the base: 5.0 * ~3 = 15.
    let (plain, _) = World::new().golem_after_arrow("");
    let (power5, _) = World::new().golem_after_arrow(&bow("\"minecraft:power\":5"));
    let (power1, _) = World::new().golem_after_arrow(&bow("\"minecraft:power\":1"));
    assert!((100.0 - plain - 6.0).abs() < 1.5, "an arrow from nowhere: {plain}");
    assert!((100.0 - power5 - 15.0).abs() < 2.0, "Power V: {power5}");
    assert!(power1 > power5 && power1 < plain, "Power I ({power1}) is between: plain {plain}, V {power5}");
}

#[test]
fn punch_pushes_the_target_along_the_arrow() {
    if !datapack() {
        eprintln!("no datapack: skipped");
        return;
    }
    let (_, none) = World::new().golem_after_arrow("");
    let (_, punch) = World::new().golem_after_arrow(&bow("\"minecraft:punch\":2"));
    assert!(punch > none + 0.05, "knocked along +x: plain {none}, Punch II {punch}");
}
