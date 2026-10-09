//! An ominous trial spawner rains items: while its trial is active it hangs an `ominous_item_spawner` over a player or
//! a mob of the trial (60 to 120 ticks, a sound 36 ticks before), the item flies down as a projectile or drops, and
//! the next waits 160 ticks. The spawner entity itself is compared with vanilla by `container_parity`
//! (`ominous50_` scenarios); this is the trial spawner's side (needs the vanilla datapack: `KILN_DATAPACK`).

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        assert!(sim.step([
            msg,
            ToSim::Console("gamemode survival Keeper".into()),
            ToSim::Console("difficulty normal".into()),
            ToSim::Console("time set 18000".into()),
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

    fn count(&self, kind: &str) -> usize {
        self.sim.entities().into_iter().filter(|e| e.0 == kind).count()
    }
}

#[test]
fn an_ominous_trial_hangs_item_spawners_over_the_player_and_throws_their_items() {
    if std::env::var_os("KILN_DATAPACK").is_none() {
        eprintln!("no vanilla datapack (KILN_DATAPACK): the ominous loot table is missing; skipped");
        return;
    }
    let mut w = World::new();
    let g = w.ground;
    // A trial that stays active for long (many mobs that stand still), already ominous.
    let ominous = "ominous_config:{total_mobs:50.0f,simultaneous_mobs:1.0f,ticks_between_spawn:20,spawn_potentials:[{data:{entity:{id:\"minecraft:zombie\",NoAI:1b,Invulnerable:1b}},weight:1}]}";
    w.run(&format!(
        "setblock {} {} {} minecraft:trial_spawner[trial_spawner_state=waiting_for_players,ominous=true]{{normal_config:{{}},{ominous}}}",
        g[0] + 3,
        g[1] + 1,
        g[2]
    ));
    let (mut spawners, mut shot, mut seen_spawner_at) = (0usize, 0usize, None);
    for tick in 0..900 {
        w.ticks(1);
        let now = w.count("minecraft:ominous_item_spawner");
        if now > 0 && seen_spawner_at.is_none() {
            seen_spawner_at = Some(tick);
        }
        spawners = spawners.max(now);
        if tick % 100 == 0 {
            eprintln!("tick {tick}: be {:?} mobs {}", w.sim.block_entity_nbt(g[0] + 3, g[1] + 1, g[2]).map(|t| format!("{t:?}").chars().take(300).collect::<String>()), w.sim.mobs().len());
        }
        shot = shot.max(w.count("minecraft:arrow") + w.count("minecraft:lingering_potion") + w.count("minecraft:small_fireball") + w.count("minecraft:wind_charge"));
    }
    eprintln!("first item spawner at tick {seen_spawner_at:?}, at most {spawners} at once, {shot} projectiles");
    assert!(seen_spawner_at.is_some(), "the ominous trial hung an item spawner over the player");
    assert!(shot > 0, "the items were thrown (projectiles flew)");
}
