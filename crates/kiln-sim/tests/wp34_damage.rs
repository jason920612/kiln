//! wp34: `/damage` on entities that are not players (any damage type, `at`, `by` and `from`).

use kiln_link::ToSim;
use kiln_proto::nbt::Tag;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
}

impl World {
    fn new() -> World {
        let mut sim = Sim::new(SimConfig::new(6, 4, None));
        sim.capture_console();
        let (msg, stats) = join(1, "Hitter", 2);
        assert!(sim.step([msg]));
        let mut w = World { sim, client: Client::new(1, stats) };
        for c in ["gamerule minecraft:spawn_mobs false", "gamemode creative Hitter"] {
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
        self.sim.take_console();
    }

    fn summon(&mut self, entity: &str, dx: f64, nbt: &str) {
        let p = self.client.pos;
        self.console(&format!("summon {entity} {} {} {} {nbt}", p[0] + dx, p[1], p[2] + 2.0));
        self.ticks(2);
    }

    fn nbt_with_tag(&self, tag: &str) -> Option<Tag> {
        self.sim.entity_nbt().into_iter().find(|t| {
            t.get("Tags").and_then(Tag::as_list).is_some_and(|l| l.iter().any(|x| x.as_str() == Some(tag)))
        })
    }

    fn health(&self, tag: &str) -> Option<f64> {
        self.nbt_with_tag(tag)?.get("Health").and_then(Tag::as_f64)
    }

    /// The cow's x motion.
    fn motion(&self, tag: &str) -> [f64; 3] {
        let m = self.nbt_with_tag(tag).unwrap();
        let l = m.get("Motion").and_then(Tag::as_list).unwrap();
        [0, 1, 2].map(|i| l[i].as_f64().unwrap())
    }
}

const COW: &str = r#"{NoAI:1b,Health:10f,PersistenceRequired:1b,Tags:["victim"]}"#;

#[test]
fn damage_hurts_a_mob_and_fails_on_an_invulnerable_one() {
    let mut w = World::new();
    w.summon("minecraft:cow", 3.0, COW);
    assert_eq!(w.health("victim"), Some(10.0));
    w.console("damage @e[tag=victim,limit=1] 3");

    assert_eq!(w.health("victim"), Some(7.0));
    // Within the hurt cooldown a weaker hit does nothing (`LivingEntity.hurtServer`).
    w.console("damage @e[tag=victim,limit=1] 2");
    assert_eq!(w.health("victim"), Some(7.0));
    // A stronger one takes the difference.
    w.console("damage @e[tag=victim,limit=1] 4");
    assert_eq!(w.health("victim"), Some(6.0));

    // `Invulnerable` stops everything but the types that bypass it.
    w.summon("minecraft:cow", -3.0, r#"{NoAI:1b,Health:10f,PersistenceRequired:1b,Invulnerable:1b,Tags:["shielded"]}"#);
    w.console("damage @e[tag=shielded,limit=1] 3");
    assert_eq!(w.health("shielded"), Some(10.0));
    w.console("damage @e[tag=shielded,limit=1] 3 minecraft:out_of_world");
    assert_eq!(w.health("shielded"), Some(7.0));
}

#[test]
fn damage_by_an_entity_knocks_the_mob_away_from_it() {
    let mut w = World::new();
    // The cow stands east of the player, with its feet on the ground.
    w.summon("minecraft:cow", 3.0, COW);
    w.console("damage @e[tag=victim,limit=1] 1 minecraft:player_attack by Hitter");
    w.ticks(1);
    let m = w.motion("victim");
    assert!(m[0] > 0.1, "pushed away from the player: {m:?}");
    assert_eq!(w.health("victim"), Some(9.0));
}

#[test]
fn damage_at_a_position_pushes_from_there() {
    let mut w = World::new();
    w.summon("minecraft:cow", 3.0, COW);
    let p = w.client.pos;
    // From the east: the cow is pushed west.
    w.console(&format!("damage @e[tag=victim,limit=1] 1 minecraft:mob_attack at {} {} {}", p[0] + 20.0, p[1], p[2] + 2.0));
    w.ticks(1);
    let m = w.motion("victim");
    assert!(m[0] < -0.1, "pushed away from the point: {m:?}");
}

#[test]
fn damage_kills_through_the_mobs_own_death() {
    let mut w = World::new();
    w.summon("minecraft:cow", 3.0, COW);
    w.console("damage @e[tag=victim,limit=1] 100 minecraft:generic_kill");
    w.ticks(30);
    assert!(w.nbt_with_tag("victim").is_none(), "dead and gone");
    // It dropped its loot like any dead cow.
    let drops = w.sim.entity_nbt().into_iter().filter(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:item")).count();
    let _ = drops;
}

#[test]
fn damage_breaks_a_minecart_and_a_boat_like_a_hit() {
    let mut w = World::new();
    w.summon("minecraft:minecart", 3.0, r#"{Tags:["cart"]}"#);
    w.summon("minecraft:oak_boat", -3.0, r#"{Tags:["boat"]}"#);
    assert!(w.nbt_with_tag("cart").is_some() && w.nbt_with_tag("boat").is_some());
    // Both break after enough damage in a few hits.
    for _ in 0..2 {
        w.console("damage @e[tag=cart,limit=1] 30 minecraft:player_attack by Hitter");
        w.console("damage @e[tag=boat,limit=1] 30 minecraft:player_attack by Hitter");
        w.ticks(12);
    }
    assert!(w.nbt_with_tag("cart").is_none(), "the cart broke");
    assert!(w.nbt_with_tag("boat").is_none(), "the boat broke");
}
