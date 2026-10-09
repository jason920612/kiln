//! Game events, vibrations and sculk in the running simulation: sculk sensors hearing blocks
//! broken nearby (travel time, redstone power by distance, comparator frequency, cooldown),
//! wool blocking vibrations, calibrated sensors filtering by frequency, shriekers set off by
//! players (warning levels, darkness), catalysts blooming and spreading sculk when mobs die.
//! Timing against vanilla is checked tick by tick by `sculk_parity` (tools/sculk_vectors.py).

use kiln_blocks::state;
use kiln_link::ToSim;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    /// The block the player stands on.
    ground: [i32; 3],
}

impl World {
    fn new(mode: &str) -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Listener", 2);
        assert!(sim.step([msg, ToSim::Console(format!("gamemode {mode} Listener")), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
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

    fn setblock(&mut self, p: [i32; 3], block: &str) {
        self.run(&format!("setblock {} {} {} {block}", p[0], p[1], p[2]));
    }

    fn block(&self, p: [i32; 3]) -> u16 {
        self.sim.block_at(p[0], p[1], p[2]).expect("loaded")
    }

    /// A shrieker is half a block high: the player (whose client here never moves by itself)
    /// is put down onto it, as the fall would have done.
    fn stand_on_shrieker(&mut self) {
        let p = self.client.pos;
        self.run(&format!("tp Listener {} {} {}", p[0], p[1] - 0.5, p[2]));
    }

    fn phase(&self, p: [i32; 3]) -> &'static str {
        state::get(self.block(p), "sculk_sensor_phase").unwrap_or("?")
    }
}

#[test]
fn sensors_hear_broken_blocks_and_power_redstone() {
    let mut w = World::new("creative");
    let (sensor, lamp, target) = (w.at(6, 1, 0), w.at(7, 1, 0), w.at(6, 1, 3));
    w.setblock(sensor, "minecraft:sculk_sensor");
    w.setblock(lamp, "minecraft:redstone_lamp");
    w.setblock(target, "minecraft:stone");
    w.ticks(2);
    assert_eq!(w.phase(sensor), "inactive");
    // Breaking the stone 3 blocks away: the vibration travels 3 ticks.
    w.setblock(target, "air destroy");
    let mut active_after = None;
    for t in 1..=10 {
        w.ticks(1);
        if w.phase(sensor) == "active" {
            active_after = Some(t);
            break;
        }
    }
    // Picked in the tick after the command (vanilla runs commands before its tick), then
    // three ticks of travel counted down from that tick.
    assert_eq!(active_after, Some(2), "the vibration travels three ticks");
    let s = w.block(sensor);
    // `getRedstoneStrengthForDistance(3, 8)`: 15 - floor(15 / 8 * 3) = 10.
    assert_eq!(state::get_int(s, "power"), 10);
    assert!(state::get_bool(w.block(lamp), "lit"), "the lamp beside the sensor lights");
    // Active for 30 ticks, then cooldown for 10, then ready again.
    w.ticks(29);
    assert_eq!(w.phase(sensor), "active");
    w.ticks(1);
    assert_eq!(w.phase(sensor), "cooldown");
    w.ticks(10);
    assert_eq!(w.phase(sensor), "inactive");
}

#[test]
fn wool_blocks_vibrations_and_dampens_steps() {
    let mut w = World::new("creative");
    let (sensor, target) = (w.at(6, 1, 0), w.at(6, 1, 4));
    w.setblock(sensor, "minecraft:sculk_sensor");
    // A wool wall between the sensor and the block.
    w.run(&format!("fill {} {} {} {} {} {} minecraft:white_wool", sensor[0] - 2, sensor[1] - 1, sensor[2] + 2, sensor[0] + 2, sensor[1] + 2, sensor[2] + 2));
    w.setblock(target, "minecraft:stone");
    w.ticks(2);
    w.setblock(target, "air destroy");
    w.ticks(10);
    assert_eq!(w.phase(sensor), "inactive", "the wool wall occludes the vibration");
    // Breaking wool itself makes no vibration (`#dampens_vibrations`).
    let wool = [sensor[0], sensor[1], sensor[2] + 2];
    w.setblock(wool, "air destroy");
    w.ticks(10);
    assert_eq!(w.phase(sensor), "inactive", "broken wool is silent");
    // With a hole in the wall, a broken stone is heard.
    w.setblock(target, "minecraft:stone");
    w.ticks(1);
    w.setblock(target, "air destroy");
    w.ticks(8);
    assert_eq!(w.phase(sensor), "active");
}

#[test]
fn calibrated_sensors_listen_for_one_frequency() {
    let mut w = World::new("creative");
    // Facing north: the back is south. A redstone block behind it asks for frequency 15.
    let (sensor, back, target) = (w.at(6, 1, 0), w.at(6, 1, 1), w.at(3, 1, 0));
    w.setblock(sensor, "minecraft:calibrated_sculk_sensor[facing=north]");
    w.setblock(back, "minecraft:redstone_block");
    w.setblock(target, "minecraft:stone");
    w.ticks(2);
    w.setblock(target, "air destroy");
    w.ticks(8);
    assert_eq!(w.phase(sensor), "inactive", "a broken block (frequency 12) is not 15");
    // Without the filter it hears everything within 16 blocks.
    w.setblock(back, "minecraft:air");
    let far = w.at(6, 1, -12);
    w.setblock(far, "minecraft:stone");
    w.ticks(2);
    w.setblock(far, "air destroy");
    w.ticks(16);
    assert_ne!(w.phase(sensor), "inactive", "12 blocks away is within a calibrated sensor's 16");
}

#[test]
fn comparators_read_the_last_frequency() {
    let mut w = World::new("creative");
    let (sensor, comparator, dust, target) = (w.at(6, 1, 0), w.at(7, 1, 0), w.at(8, 1, 0), w.at(6, 1, 2));
    w.setblock(sensor, "minecraft:sculk_sensor");
    // A comparator reading the sensor behind it (facing west, toward the sensor).
    w.setblock(comparator, "minecraft:comparator[facing=west]");
    w.setblock(dust, "minecraft:redstone_wire");
    w.setblock(target, "minecraft:stone");
    w.ticks(2);
    w.setblock(target, "air destroy");
    w.ticks(6);
    assert_eq!(w.phase(sensor), "active");
    let power = state::get_int(w.block(dust), "power");
    // Block destroy is frequency 12; the wire right after the comparator carries it.
    assert_eq!(power, 12, "the comparator outputs the vibration frequency");
}

#[test]
fn shriekers_warn_players_and_can_summon_darkness() {
    let mut w = World::new("survival");
    w.run("difficulty normal");
    // A summoning shrieker right under the player: standing on it sets it off.
    let under = w.ground;
    w.setblock(under, "minecraft:sculk_shrieker[can_summon=true]");
    w.stand_on_shrieker();
    w.ticks(2);
    assert!(state::get_bool(w.block(under), "shrieking"), "the player standing on it makes it shriek");
    // The shriek lasts 90 ticks; then it answers with darkness.
    w.ticks(95);
    assert!(!state::get_bool(w.block(under), "shrieking"));
    let effects = w.sim.effects(1).unwrap();
    assert!(effects.iter().any(|e| e.0 == "minecraft:darkness"), "darkness after the shriek: {effects:?}");
}

#[test]
fn sensors_save_their_vibration_state() {
    let mut w = World::new("creative");
    let (sensor, target) = (w.at(6, 1, 0), w.at(6, 1, 7));
    w.setblock(sensor, "minecraft:sculk_sensor");
    w.setblock(target, "minecraft:stone");
    w.ticks(2);
    w.setblock(target, "air destroy");
    w.ticks(3);
    // Mid-travel, `/data` sees the listener's vibration.
    let data = w.sim.block_entity_nbt(sensor[0], sensor[1], sensor[2]).expect("a block entity");
    let listener = data.get("listener").expect("listener saved");
    assert!(listener.get("event").is_some(), "the travelling vibration is saved: {listener:?}");
}

impl World {
    fn mobs(&self, kind: &str) -> Vec<(i32, [f64; 3], f32)> {
        self.sim.mobs().into_iter().filter(|m| m.1 == kind).map(|m| (m.0, m.2, m.3)).collect()
    }
}

#[test]
fn shriekers_summon_a_warden_at_the_fourth_warning() {
    let mut w = World::new("survival");
    w.run("difficulty normal");
    let under = w.ground;
    w.setblock(under, "minecraft:sculk_shrieker[can_summon=true]");
    w.stand_on_shrieker();
    // Each shriek raises the warning level once the 200-tick cooldown is over; the fourth
    // shriek's answer is a warden digging out nearby.
    let mut summoned = None;
    for t in 0..1200 {
        w.ticks(1);
        if !w.mobs("minecraft:warden").is_empty() {
            summoned = Some(t);
            break;
        }
    }
    assert!(summoned.is_some_and(|t| t > 600), "a warden after four warnings (tick {summoned:?})");
    let (_, pos, health) = w.mobs("minecraft:warden")[0];
    assert_eq!(health, 500.0);
    let d = ((pos[0] - under[0] as f64 - 0.5).powi(2) + (pos[2] - under[2] as f64 - 0.5).powi(2)).sqrt();
    assert!(d <= 5.0 * 2f64.sqrt() + 0.01, "within five blocks of the shrieker ({d})");
}

#[test]
fn wardens_hunt_whoever_hurts_them_and_pulse_darkness() {
    use kiln_link::PlayIn;
    let mut w = World::new("survival");
    w.run("difficulty normal");
    let p = w.client.pos;
    // Summoned without data, so `finalizeSpawn` gives it its dig cooldown (with data it
    // digs straight back down, as in vanilla).
    w.run(&format!("summon minecraft:warden {} {} {}", p[0] + 3.0, p[1], p[2]));
    w.ticks(2);
    let (id, _, _) = w.mobs("minecraft:warden")[0];
    // Darkness reaches the player within the pulse interval.
    w.ticks(121);
    let effects = w.sim.effects(1).unwrap();
    assert!(effects.iter().any(|e| e.0 == "minecraft:darkness"), "darkness around the warden: {effects:?}");
    // A punch makes it angry enough to fight at once.
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::Attack { entity_id: id }), ToSim::Packet(1, PlayIn::Punch)]));
    let mut hit = false;
    for _ in 0..200 {
        w.ticks(1);
        if w.sim.health(1).is_some_and(|h| h.0 < 20.0 || h.1) {
            hit = true;
            break;
        }
    }
    assert!(hit, "the warden hits back");
}

#[test]
fn calm_wardens_dig_back_down() {
    let mut w = World::new("creative");
    let p = w.client.pos;
    w.run(&format!("summon minecraft:warden {} {} {}", p[0] + 12.0, p[1], p[2]));
    w.ticks(2);
    assert_eq!(w.mobs("minecraft:warden").len(), 1);
    // 1200 ticks without a disturbance (creative players are not sniffed out), then, once it
    // stands still, 100 of digging.
    w.ticks(1150);
    assert_eq!(w.mobs("minecraft:warden").len(), 1, "still there within the cooldown");
    let mut gone = None;
    for t in 0..600 {
        w.ticks(1);
        if w.mobs("minecraft:warden").is_empty() {
            gone = Some(t);
            break;
        }
    }
    assert!(gone.is_some_and(|t| t >= 100), "dug away after digging for 100 ticks ({gone:?})");
}
