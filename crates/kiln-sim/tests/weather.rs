//! Weather, the day cycle and sleeping in the running simulation: the weather cycle and
//! `/weather`, sleeping through the night (and the thunderstorm), the bed messages, the respawn
//! point, and beds exploding in the nether.

use kiln_blocks::state;
use kiln_data::blocks::default_state as d;
use kiln_link::{PlayIn, ToSim};
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    ground: [i32; 3],
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Sleeper", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Sleeper")), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
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

    /// A red bed with its foot at `foot`, head toward +x.
    fn bed(&mut self, foot: [i32; 3]) -> [i32; 3] {
        let head = [foot[0] + 1, foot[1], foot[2]];
        self.run(&format!("setblock {} {} {} minecraft:red_bed[part=head,facing=east]", head[0], head[1], head[2]));
        self.run(&format!("setblock {} {} {} minecraft:red_bed[part=foot,facing=east]", foot[0], foot[1], foot[2]));
        head
    }

    fn click(&mut self, pos: [i32; 3]) {
        let pkt = PlayIn::UseItemOn { hand: 0, pos, face: 1, cursor: [0.5, 0.5, 0.5], inside: false, sequence: 1 };
        assert!(self.sim.step([ToSim::Packet(1, pkt)]));
    }

    fn sleeping(&self) -> Option<[i32; 3]> {
        self.sim.sleep_state(1).unwrap().0
    }
}

#[test]
fn weather_command_starts_rain_and_levels_climb() {
    let mut w = World::new("survival");
    w.run("gamerule minecraft:advance_weather false");
    w.run("weather rain 6000");
    assert_eq!(w.sim.weather_counters(), (0, 6000, 6000, true, false));
    // 0.01 per tick (the command's tick included): raining once past 0.2.
    w.ticks(9);
    let (raining, _, rain, _) = w.sim.overworld_weather();
    assert!(!raining && (rain - 0.1).abs() < 1e-5, "{rain}");
    w.ticks(15);
    assert!(w.sim.overworld_weather().0);
    w.run("weather thunder");
    w.ticks(100);
    let (raining, thundering, rain, thunder) = w.sim.overworld_weather();
    assert!(raining && thundering && rain == 1.0 && thunder == 1.0);
    w.run("weather clear 100");
    w.ticks(100);
    assert_eq!(w.sim.overworld_weather(), (false, false, 0.0, 0.0));
}

#[test]
fn weather_cycle_counts_down_and_flips() {
    let mut w = World::new("survival");
    w.run("weather rain 5");
    w.ticks(3);
    assert!(w.sim.weather_counters().3);
    // The rain time runs out and the rain stops; the next tick draws a new delay.
    w.ticks(2);
    let (clear, rain_time, _, raining, _) = w.sim.weather_counters();
    assert!(!raining && clear == 0 && (12000..=180000).contains(&rain_time), "{:?}", w.sim.weather_counters());
}

#[test]
fn sleeping_skips_the_night_and_the_storm() {
    let mut w = World::new("survival");
    let head = w.bed(w.at(1, 1, 0));
    w.run("time set 18000");
    w.run("weather thunder");
    w.ticks(2);
    w.click(head);
    assert_eq!(w.sleeping(), Some(head));
    assert_eq!(w.sim.respawn_point(1), Some((Some(head), "minecraft:overworld")));
    assert!(state::get_bool(w.sim.block_at(head[0], head[1], head[2]).unwrap(), "occupied"));
    // 100 ticks asleep, then the clock moves to the next morning and everyone wakes.
    w.ticks(99);
    assert_eq!(w.sleeping(), Some(head));
    w.ticks(2);
    assert_eq!(w.sleeping(), None);
    assert_eq!(w.sim.day_time() % 24000, 1);
    assert!(w.sim.day_time() >= 24000);
    let (_, rain_time, thunder_time, raining, thundering) = w.sim.weather_counters();
    assert!(!raining && !thundering, "the storm stops");
    assert!(rain_time > 0 && thunder_time > 0, "new delays drawn: {rain_time} {thunder_time}");
    assert!(!state::get_bool(w.sim.block_at(head[0], head[1], head[2]).unwrap(), "occupied"));
    let since_rest = w.sim.sleep_state(1).unwrap().2;
    assert!((1..=2).contains(&since_rest), "insomnia counts again from waking: {since_rest}");
}

#[test]
fn no_sleeping_by_day_but_the_spawn_point_is_set() {
    let mut w = World::new("survival");
    let head = w.bed(w.at(-3, 1, 2));
    w.run("time set 6000");
    w.ticks(1);
    w.click(head);
    assert_eq!(w.sleeping(), None);
    assert_eq!(w.sim.respawn_point(1).unwrap().0, Some(head));
    // A thunderstorm darkens the sky enough to sleep.
    w.run("weather thunder");
    w.ticks(100);
    w.click(head);
    assert_eq!(w.sleeping(), Some(head));
}

#[test]
fn beds_explode_in_the_nether() {
    let mut w = World::new("survival");
    let pos = w.at(0, 0, 0);
    w.run(&format!("execute in minecraft:the_nether run tp Sleeper {} 100 {}", pos[0], pos[2]));
    w.ticks(10);
    let base = [pos[0], 99, pos[2]];
    w.run(&format!("execute in minecraft:the_nether run fill {} 99 {} {} 99 {} minecraft:obsidian", base[0] - 3, base[2] - 3, base[0] + 3, base[2] + 3));
    w.run(&format!("execute in minecraft:the_nether run fill {} 100 {} {} 102 {} minecraft:air", base[0] - 3, base[2] - 3, base[0] + 3, base[2] + 3));
    let (foot, head) = ([base[0] + 1, 100, base[2]], [base[0] + 2, 100, base[2]]);
    w.run(&format!("execute in minecraft:the_nether run setblock {} {} {} minecraft:red_bed[part=head,facing=east]", head[0], head[1], head[2]));
    w.run(&format!("execute in minecraft:the_nether run setblock {} {} {} minecraft:red_bed[part=foot,facing=east]", foot[0], foot[1], foot[2]));
    w.ticks(2);
    let (health_before, _) = w.sim.health(1).unwrap();
    w.click(foot);
    w.ticks(2);
    assert_eq!(w.sleeping(), None);
    let block = w.sim.block_in("minecraft:the_nether", head[0], head[1], head[2]).unwrap();
    assert!(!state::is(block, d::RED_BED), "the bed is gone");
    assert!(w.sim.health(1).unwrap().0 < health_before, "the explosion hurts");
}

#[test]
fn lightning_charges_creepers_converts_pigs_and_lights_fire() {
    let mut w = World::new("creative");
    let at = w.at(6, 1, 6);
    w.run(&format!("summon minecraft:creeper {} {} {}", at[0] as f64 + 0.5, at[1], at[2] as f64 + 0.5));
    w.run(&format!("summon minecraft:pig {} {} {}", at[0] as f64 + 1.5, at[1], at[2] as f64 + 0.5));
    w.ticks(2);
    w.run(&format!("summon minecraft:lightning_bolt {} {} {}", at[0] as f64 + 0.5, at[1], at[2] as f64 + 0.5));
    w.ticks(3);
    let names: Vec<&str> = w.sim.mobs().iter().map(|m| m.1).collect();
    assert!(names.contains(&"minecraft:zombified_piglin") && !names.contains(&"minecraft:pig"), "{names:?}");
    let creeper = w.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(|i| i.as_str()) == Some("minecraft:creeper")).expect("creeper");
    assert_eq!(creeper.get("powered").and_then(|p| p.as_i64()), Some(1));
    // Normal difficulty: fire where the bolt struck, burning out after a while.
    assert!(state::is(w.sim.block_at(at[0], at[1], at[2]).unwrap(), d::FIRE));
    w.ticks(20);
    assert!(!w.sim.entities().iter().any(|e| e.0 == "minecraft:lightning_bolt"), "the bolt is gone");
    w.ticks(600);
    assert!(!state::is(w.sim.block_at(at[0], at[1], at[2]).unwrap(), d::FIRE), "the fire burnt out");
}
