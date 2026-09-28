//! Villages, bad omen and raids end to end: villagers claim beds and a bell (points of
//! interest), a player with bad omen in the village gets raid omen, the raid starts when it runs
//! out, waves spawn with a banner-carrying leader, and the raid ends in victory once every wave
//! is gone. Raider behaviour itself is checked tick by tick against vanilla by kiln-entity's
//! `mob_parity` test.

use kiln_link::ToSim;
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
        w.console("gamerule minecraft:spawn_mobs false");
        w.console("difficulty normal");
        w.console("gamemode creative Hunter");
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

    fn pos(&self) -> [i32; 3] {
        let p = self.clients[0].pos;
        [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32]
    }

    fn mobs(&self, kind: &str) -> Vec<i32> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| m.0).collect()
    }

    /// Beds (head halves) and a bell around the player, and a villager for each bed, walled in
    /// so they stay near.
    fn village(&mut self) {
        self.village_blocks();
        self.villagers();
    }

    fn village_blocks(&mut self) {
        let [x, y, z] = self.pos();
        for i in 0..3 {
            self.console(&format!("setblock {} {y} {} minecraft:red_bed[part=head]", x + 3 + 2 * i, z + 4));
        }
        self.console(&format!("setblock {} {y} {} minecraft:bell", x - 3, z + 4));
    }

    fn villagers(&mut self) {
        let [x, y, z] = self.pos();
        for i in 0..3 {
            self.console(&format!("summon minecraft:villager {} {y} {}", x + 3 + 2 * i, z + 6));
        }
    }
}

#[test]
fn villagers_claim_points_of_interest_and_make_a_village() {
    let mut w = World::new();
    let p = w.pos();
    assert_eq!(w.sim.sections_to_village("minecraft:overworld", p), 7, "no village yet");
    w.village_blocks();
    // Beds alone are no village: someone has to sleep in them.
    w.ticks(40);
    assert_eq!(w.sim.sections_to_village("minecraft:overworld", p), 7);
    w.villagers();
    w.ticks(40);
    assert!(w.sim.sections_to_village("minecraft:overworld", p) <= 1, "villagers claimed the beds and the bell");
    // A broken bed is forgotten and no longer counts.
    let [x, y, z] = p;
    for i in 0..3 {
        w.console(&format!("setblock {} {y} {} minecraft:air", x + 3 + 2 * i, z + 4));
    }
    w.console(&format!("setblock {} {y} {} minecraft:air", x - 3, z + 4));
    w.ticks(2);
    assert_eq!(w.sim.sections_to_village("minecraft:overworld", p), 7);
}

#[test]
fn bad_omen_in_a_village_starts_a_raid_with_waves_until_victory() {
    let mut w = World::new();
    w.village();
    w.ticks(40);
    assert!(w.sim.sections_to_village("minecraft:overworld", w.pos()) <= 1);
    w.console("gamemode survival Hunter");
    w.console("effect give Hunter minecraft:bad_omen 100 1");
    w.ticks(2);
    let effects = w.sim.effects(1).unwrap();
    assert!(effects.iter().any(|e| e.0 == "minecraft:raid_omen"), "bad omen turned into raid omen: {effects:?}");
    assert!(!effects.iter().any(|e| e.0 == "minecraft:bad_omen"));
    // The raid omen lasts 30 seconds, then the raid starts.
    w.ticks(600);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids.len(), 1, "{raids:?}");
    assert_eq!(raids[0].1, "ongoing");
    assert_eq!(raids[0].3, 2, "omen level: amplifier 1 absorbed");
    assert_eq!(raids[0].2, 0, "no wave before the countdown");
    // Fifteen seconds of countdown, then the first wave: pillagers, one of them the leader
    // with the ominous banner.
    w.console("gamemode creative Hunter");
    w.ticks(305);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids[0].2, 1, "first wave spawned: {raids:?}");
    let pillagers = w.mobs("minecraft:pillager");
    assert!(pillagers.len() >= 4, "{pillagers:?}");
    let leaders: Vec<_> = pillagers.iter().filter_map(|&id| w.sim.raider(id)).filter(|r| r.2 && r.4).collect();
    assert_eq!(leaders.len(), 1, "one leader with the banner");
    assert!(pillagers.iter().all(|&id| w.sim.raider(id).is_some_and(|r| r.0 == Some(raids[0].0) && r.1 == 1)));
    assert!(raids[0].5 >= 4 && raids[0].6 > 0.0, "raiders alive and the bar full: {raids:?}");
    // Each wave gone brings the next after its countdown, until the last (five waves on normal,
    // one more for the omen level); then victory.
    for wave in 1..=6 {
        w.console("kill @e[type=!minecraft:villager,type=!minecraft:player]");
        w.ticks(30);
        let raids = w.sim.raids("minecraft:overworld");
        if raids[0].1 != "ongoing" {
            break;
        }
        assert_eq!(raids[0].2, wave, "{raids:?}");
        w.ticks(300);
    }
    w.ticks(60);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids[0].1, "victory", "{raids:?}");
    assert_eq!(raids[0].2, 6, "five waves and the bonus wave");
    // The celebration runs half a minute, then the raid is gone.
    w.ticks(620);
    assert!(w.sim.raids("minecraft:overworld").is_empty());
}
