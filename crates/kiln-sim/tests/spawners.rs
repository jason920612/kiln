//! Mob spawner blocks in the running simulation: a spawner with a player near spawns its mobs
//! (up to `MaxNearbyEntities`), counts its delay down and resets it, keeps its state in its
//! block entity, does nothing with no player in range or with `spawner_blocks_work` off, and a
//! spawn egg used on it changes what it spawns. The spawner's own draws and checks are compared
//! with vanilla tick by tick by kiln-entity's `mob_parity` (`tools/mob_vectors.py --filter spawner_`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([
            msg,
            ToSim::Console("gamemode creative Keeper".into()),
            ToSim::Console("difficulty normal".into()),
            ToSim::Console("time set 18000".into()),
            // Natural spawning off: only the spawners make mobs here.
            ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        let p = client.pos;
        let ground = [p[0].floor() as i32, p[1].floor() as i32 - 1, p[2].floor() as i32];
        Self { sim, client, ground }
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

    fn at(&self, dx: i32, dy: i32, dz: i32) -> [i32; 3] {
        [self.ground[0] + dx, self.ground[1] + dy, self.ground[2] + dz]
    }

    /// A spawner at `p` with `data` (SNBT fields of its block entity).
    fn spawner(&mut self, p: [i32; 3], data: &str) {
        self.run(&format!("setblock {} {} {} minecraft:spawner{{{data}}}", p[0], p[1], p[2]));
    }

    fn mobs_of(&self, kind: &str) -> usize {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).count()
    }

    fn nbt(&self, p: [i32; 3]) -> Tag {
        self.sim.block_entity_nbt(p[0], p[1], p[2]).expect("a spawner block entity")
    }

    fn delay(&self, p: [i32; 3]) -> i64 {
        self.nbt(p).get("Delay").and_then(Tag::as_i64).expect("Delay")
    }
}

/// Zombies that need no darkness (custom spawn rules: any light), standing still.
const ZOMBIES: &str = "SpawnData:{entity:{id:\"minecraft:zombie\",NoAI:1b},custom_spawn_rules:{block_light_limit:[0,15],sky_light_limit:[0,15]}},SpawnPotentials:[]";

#[test]
fn a_spawner_with_a_player_near_spawns_up_to_its_cap_and_resets_its_delay() {
    let mut w = World::new();
    let at = w.at(4, 1, 0);
    w.spawner(at, &format!("{ZOMBIES},Delay:2,MinSpawnDelay:20,MaxSpawnDelay:20,SpawnCount:2,MaxNearbyEntities:3"));
    // (The step that placed it already ticked it once.)
    assert!((0..=2).contains(&w.delay(at)), "delay {}", w.delay(at));
    w.ticks(10);
    let made = w.mobs_of("minecraft:zombie");
    assert!((1..=3).contains(&made), "the spawner made {made} zombies");
    // Spawning put the delay back to 20 (equal ends: no draw), counting down since.
    let delay = w.delay(at);
    assert!((10..=20).contains(&delay), "delay {delay}");
    // The cap: no more than three zombies stand around it, however long it runs.
    w.ticks(200);
    assert_eq!(w.mobs_of("minecraft:zombie"), 3);
}

#[test]
fn a_spawner_does_nothing_without_a_player_in_range() {
    let mut w = World::new();
    let at = w.at(40, 1, 0);
    w.spawner(at, &format!("{ZOMBIES},Delay:5,MinSpawnDelay:20,MaxSpawnDelay:20,SpawnCount:2,MaxNearbyEntities:3"));
    w.ticks(60);
    assert_eq!(w.mobs_of("minecraft:zombie"), 0);
    assert_eq!(w.delay(at), 5, "the delay does not run without a player near");
}

#[test]
fn spawner_blocks_work_off_stops_every_spawner() {
    let mut w = World::new();
    w.run("gamerule minecraft:spawner_blocks_work false");
    let at = w.at(4, 1, 0);
    w.spawner(at, &format!("{ZOMBIES},Delay:2,MinSpawnDelay:20,MaxSpawnDelay:20,SpawnCount:2,MaxNearbyEntities:3"));
    w.ticks(60);
    assert_eq!(w.mobs_of("minecraft:zombie"), 0);
    w.run("gamerule minecraft:spawner_blocks_work true");
    w.ticks(30);
    assert!(w.mobs_of("minecraft:zombie") > 0, "it works again once the rule is back");
}

#[test]
fn a_spawn_egg_changes_what_the_spawner_spawns() {
    let mut w = World::new();
    let at = w.at(4, 1, 0);
    // Out of its player range, so the egg lands before anything spawns.
    w.spawner(at, &format!("{ZOMBIES},Delay:2,MinSpawnDelay:20,MaxSpawnDelay:20,SpawnCount:1,MaxNearbyEntities:9,RequiredPlayerRange:1"));
    let egg = kiln_data::builtin_id("minecraft:item", "minecraft:skeleton_spawn_egg").unwrap();
    let stack = kiln_proto::packets::ItemStack { item: egg, count: 1, added: Vec::new(), removed: Vec::new() };
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::SetCreativeSlot { slot: 36, item: Some(stack) })]));
    let pkt = PlayIn::UseItemOn { hand: 0, pos: at, face: 1, cursor: [0.5, 1.0, 0.5], inside: false, sequence: 1 };
    assert!(w.sim.step([ToSim::Packet(1, pkt)]));
    let nbt = w.nbt(at);
    let id = nbt.get("SpawnData").and_then(|d| d.get("entity")).and_then(|e| e.get("id")).and_then(Tag::as_str);
    assert_eq!(id, Some("minecraft:skeleton"), "the spawn data now names the egg's mob: {nbt:?}");
    // The rest of its spawn data (the zombie's tag) stays.
    assert!(nbt.get("SpawnData").and_then(|d| d.get("entity")).and_then(|e| e.get("NoAI")).is_some());
    // And no mob stepped out of the egg itself: the egg went into the spawner.
    assert_eq!(w.mobs_of("minecraft:skeleton"), 0);
}
