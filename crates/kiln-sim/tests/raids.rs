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

/// `Raider.hurtServer` updates the raid's boss bar before the blow lands and `Raid.removeFromRaid` after a death: the bar shows the
/// living raiders' health over the wave's total, one blow behind while they are only hurt, exact once one of them dies. (It stayed
/// full until the wave was dead.)
#[test]
fn the_raid_bar_follows_the_raiders_health() {
    let mut w = World::new();
    w.village();
    w.ticks(40);
    w.console("gamemode survival Hunter");
    w.console("effect give Hunter minecraft:bad_omen 100 1");
    w.ticks(2 + 600);
    w.console("gamemode creative Hunter");
    w.ticks(305);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids[0].2, 1, "first wave: {raids:?}");
    let bar = || w.sim.raids("minecraft:overworld")[0].6;
    assert_eq!(bar(), 1.0);
    let total = w.mobs("minecraft:pillager").len() as f32 * 24.0 + w.mobs("minecraft:vindicator").len() as f32 * 24.0;
    w.console("damage @e[type=minecraft:pillager,limit=1] 8 minecraft:generic");
    w.ticks(2);

    assert_eq!(w.sim.raids("minecraft:overworld")[0].6, 1.0, "the bar was told before the blow");
    w.ticks(15);
    w.console("damage @e[type=minecraft:pillager,limit=1] 4 minecraft:generic");
    w.ticks(2);
    let p = w.sim.raids("minecraft:overworld")[0].6;
    assert!((p - (total - 8.0) / total).abs() < 0.002, "the first blow shows with the second: {p} of {total}");
    // A raider dying updates it with everything as it is.
    w.console("kill @e[type=minecraft:pillager,limit=1]");
    w.ticks(2);
    let p = w.sim.raids("minecraft:overworld")[0].6;
    assert!((p - (total - 24.0) / total).abs() < 0.002, "{p}");
}

/// `CelebrateVillagersSurvivedRaid.tick`: after a victory the villagers under the open sky launch firework rockets (one chance in 200 a
/// tick each, a burst of one colour). Kiln's villagers danced about without launching any.
#[test]
fn villagers_launch_fireworks_after_a_victory() {
    let mut w = World::new();
    w.village();
    w.ticks(40);
    w.console("gamemode survival Hunter");
    w.console("effect give Hunter minecraft:bad_omen 100 1");
    w.ticks(2 + 600);
    w.console("gamemode creative Hunter");
    w.ticks(305);
    for _ in 0..8 {
        w.console("kill @e[type=!minecraft:villager,type=!minecraft:player]");
        w.ticks(330);
        if w.sim.raids("minecraft:overworld")[0].1 == "victory" {
            break;
        }
    }
    assert_eq!(w.sim.raids("minecraft:overworld")[0].1, "victory");
    let mut seen = 0;
    for _ in 0..560 {
        w.ticks(1);
        seen += w.sim.entities().iter().filter(|e| e.0 == "minecraft:firework_rocket").count().min(1);
    }
    assert!(seen > 0, "no firework in the celebration");
}

/// `BellBlockEntity`: a bell that is rung shakes for 50 ticks; with a raider within 32 blocks it resonates 5 ticks in and 40 ticks later every
/// raider within 48 blocks glows for 60 ticks. (Vanilla measured: a pillager 8 blocks from a bell glows from about 2.3 s after the ring.)
#[test]
fn a_rung_bell_makes_the_raiders_around_it_glow() {
    let mut w = World::new();
    let [x, y, z] = w.pos();
    w.console(&format!("setblock {} {y} {z} minecraft:bell[attachment=floor,facing=east]", x + 3));
    w.console(&format!("summon minecraft:pillager {}.5 {y} {z}.5 {{NoAI:1b,PersistenceRequired:1b}}", x + 11));
    w.console(&format!("summon minecraft:pillager {}.5 {y} {z}.5 {{NoAI:1b,PersistenceRequired:1b}}", x + 3 + 47));
    w.console(&format!("summon minecraft:pillager {}.5 {y} {z}.5 {{NoAI:1b,PersistenceRequired:1b}}", x + 3 + 60));
    w.ticks(5);
    let pillagers = w.mobs("minecraft:pillager");
    assert_eq!(pillagers.len(), 3);
    let glows = |w: &World, id: i32| w.sim.mob_state(id).unwrap().effects.iter().any(|e| e.0.contains("glowing"));
    assert!(pillagers.iter().all(|&id| !glows(&w, id)));
    // Power from beside the bell rings it.
    w.console(&format!("setblock {} {y} {} minecraft:redstone_block", x + 3, z + 1));
    let mut first = [None; 3];
    for t in 0..140 {
        w.ticks(1);
        for (i, &id) in pillagers.iter().enumerate() {
            if first[i].is_none() && glows(&w, id) {
                first[i] = Some(t);
            }
        }
    }
    // Resonation 5 + 40 ticks after the ring; the one 60 blocks away (beyond 48) never glows.
    assert!(first[0].is_some_and(|t| (40..=55).contains(&t)), "the near one glows about 45 ticks after the ring: {first:?}");
    assert!(first[1].is_some(), "47 blocks away glows: {first:?}");
    assert!(first[2].is_none(), "60 blocks away does not: {first:?}");
}

/// `Raid.tick`: when the raid's center is no longer a village (every bed and the bell broken) and a wave has come, the raid is lost: the bar says
/// so (`Raid - Defeat`), and after the 600 ticks of celebration the raid is over. (Vanilla measured on the reference server: "Raid - Defeat" two
/// seconds after the last point of interest of the village was broken, gone 29 s later.)
#[test]
fn a_raid_is_lost_when_the_village_is_gone() {
    let mut w = World::new();
    w.village();
    w.ticks(40);
    w.console("gamemode survival Hunter");
    w.console("effect give Hunter minecraft:bad_omen 100 1");
    w.ticks(2 + 600);
    w.console("gamemode creative Hunter");
    w.ticks(305);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids[0].1, "ongoing");
    assert_eq!(raids[0].2, 1, "first wave: {raids:?}");
    let [x, y, z] = w.pos();
    for i in 0..3 {
        w.console(&format!("setblock {} {y} {} minecraft:air", x + 3 + 2 * i, z + 4));
    }
    w.console(&format!("setblock {} {y} {} minecraft:air", x - 3, z + 4));
    w.console("kill @e[type=minecraft:villager]");
    w.ticks(60);
    let raids = w.sim.raids("minecraft:overworld");
    assert_eq!(raids[0].1, "loss", "{raids:?}");
    w.ticks(620);
    assert!(w.sim.raids("minecraft:overworld").is_empty(), "the celebration is over");
}

/// wp75: a raid in progress survives a restart (`raids.dat`: id, waves spawned, omen level, status, center; the boss bar and the
/// raiders' membership are rebuilt: a raider finds the raid at its position every second, `Raider.aiStep`).
#[test]
fn a_raid_in_progress_survives_a_restart() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("raid-restart");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let start = |dir: &std::path::Path| {
        let mut sim = Sim::new(SimConfig::new(8, 4, Some(dir.to_owned())));
        let (msg, stats) = join(1, "Hunter", 2);
        assert!(sim.step([msg]));
        World { sim, clients: vec![Client::new(1, stats)] }
    };
    let mut w = start(&dir);
    for c in ["gamerule minecraft:natural_health_regeneration false", "gamerule minecraft:spawn_mobs false", "difficulty normal", "gamemode creative Hunter"] {
        w.console(c);
    }
    w.ticks(5);
    // (A saved world has no ground by itself.)
    let [x, y, z] = w.pos();
    w.console(&format!("fill {} {} {} {} {} {} minecraft:stone", x - 12, y - 1, z - 12, x + 14, y - 1, z + 14));
    w.village();
    w.ticks(40);
    assert!(w.sim.sections_to_village("minecraft:overworld", w.pos()) <= 1, "a village: {:?} at {:?}", w.sim.mobs(), w.pos());
    w.console("gamemode survival Hunter");
    w.console("effect give Hunter minecraft:bad_omen 100 1");
    w.ticks(2);
    let effects = w.sim.effects(1).unwrap();
    assert!(effects.iter().any(|e| e.0 == "minecraft:raid_omen"), "raid omen: {effects:?}");
    w.ticks(600);
    w.console("gamemode creative Hunter");
    w.ticks(305);
    let before = w.sim.raids("minecraft:overworld");
    assert_eq!(before.len(), 1, "{before:?}");
    assert_eq!(before[0].2, 1, "the first wave is out");
    let raiders = w.mobs("minecraft:pillager").len();
    assert!(raiders >= 4);
    // Hurt one, so the bar is not full.
    w.console("damage @e[type=minecraft:pillager,limit=1] 3 minecraft:generic");
    w.ticks(2);
    let hurt = w.sim.raids("minecraft:overworld");
    let (done, _wait) = std::sync::mpsc::channel();
    assert!(!w.sim.step([ToSim::Shutdown { done }]));
    drop(w);
    let mut w = start(&dir);
    w.ticks(60);
    let after = w.sim.raids("minecraft:overworld");
    assert_eq!(after.len(), 1, "the raid is still there: {after:?}");
    assert_eq!((after[0].0, after[0].1, after[0].2, after[0].3, after[0].4), (before[0].0, "ongoing", 1, 2, before[0].4), "id, status, waves, omen level, center");
    let pillagers = w.mobs("minecraft:pillager");
    assert_eq!(pillagers.len(), raiders, "the raiders came back");
    assert!(
        pillagers.iter().all(|&id| w.sim.raider(id).is_some_and(|r| r.0 == Some(after[0].0) && r.1 == 1)),
        "and found their raid again: {:?}",
        pillagers.iter().map(|&id| w.sim.raider(id)).collect::<Vec<_>>()
    );
    assert_eq!(after[0].5, before[0].5, "all of them count");
    // `Raid.joinRaid` adds the health of each raider that finds the raid again to the wave's total (which was saved) and updates the bar:
    // about half of what it was.
    assert!(after[0].6 > 0.3 && after[0].6 < 0.7, "the bar after the raiders found the raid again: {} then {}", hurt[0].6, after[0].6);
    // The raid goes on: kill the wave and the next one comes.
    w.console("kill @e[type=minecraft:pillager]");
    w.ticks(400);
    let later = w.sim.raids("minecraft:overworld");
    assert_eq!(later[0].2, 2, "the second wave: {later:?}");
}
