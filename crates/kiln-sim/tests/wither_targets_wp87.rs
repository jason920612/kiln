//! wp87: a wither's side heads choose their targets among the players around (`WitherBoss.aiStep`: `getNearbyEntities(LivingEntity, TARGETING_CONDITIONS,
//! ...)` and `nextInt(size)`). wp84 saw, with two players, one of them never hit by a Kiln wither while a vanilla one hit both: two survival players ten
//! blocks apart, the wither between them above, must both be among the candidates and both get chosen over a number of withers.

use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

#[test]
fn a_wither_chooses_among_the_players_around_it() {
    let _x = kiln_data::live::exclusive();
    let file = std::env::temp_dir().join(format!("kiln-wither-{}.trace", std::process::id()));
    let _ = std::fs::remove_file(&file);
    // (Read once, at the first pick of the process.)
    unsafe { std::env::set_var("KILN_WITHER_TRACE", &file) };
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (a, sa) = join(1, "Aa", 2);
    let (b, sb) = join(2, "Bb", 2);
    assert!(sim.step([
        a,
        b,
        ToSim::Console("gamerule minecraft:spawn_mobs false".into()),
        ToSim::Console("gamerule minecraft:spawn_monsters false".into()),
        ToSim::Console("difficulty normal".into()),
        ToSim::Console("gamemode survival Aa".into()),
        ToSim::Console("gamemode survival Bb".into()),
    ]));
    let (mut ca, mut cb) = (Client::new(1, sa), Client::new(2, sb));
    let mut ticks = |sim: &mut Sim, n: usize| {
        for _ in 0..n {
            let mut inbox = Vec::new();
            ca.tick(None, &mut inbox);
            cb.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
    };
    ticks(&mut sim, 40);
    sim.console_now("execute at Aa run tp Bb ~10 ~ ~");
    sim.console_now("effect give @a minecraft:resistance 99999 4 true");
    sim.console_now("effect give @a minecraft:regeneration 99999 30 true");
    ticks(&mut sim, 5);
    for _ in 0..12 {
        sim.console_now("execute at Aa run summon minecraft:wither ~5 ~4 ~");
        ticks(&mut sim, 300);
        sim.console_now("kill @e[type=minecraft:wither]");
        ticks(&mut sim, 5);
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let _ = std::fs::remove_file(&file);
    let picks: Vec<Vec<&str>> = text.lines().map(|l| l.split(' ').collect()).collect();
    let mut by_x = std::collections::BTreeMap::<&str, usize>::new();
    for p in &picks {
        *by_x.entry(p[3]).or_default() += 1;
    }
    eprintln!("picks by x: {by_x:?} ({} picks); candidates of the first: {:?}", picks.len(), picks.first().map(|p| p[5]));
    assert!(picks.iter().all(|p| p[5] == "2"), "both players are candidates: {picks:?}");
    assert_eq!(by_x.len(), 2, "both players are picked at some time: {by_x:?}");
}
