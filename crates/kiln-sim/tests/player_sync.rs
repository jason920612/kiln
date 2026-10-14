//! wp63: what the server tells a player and its viewers about the player's entity data
//! (`ServerEntity.sendDirtyEntityData`: only the entries that changed, health included, to the viewers and to the
//! player), and the abilities and command packets that go out on join, respawn, dimension change, `/gamemode`,
//! `/op` and `/deop` (`PlayerList.sendPlayerPermissionLevel`, `ServerPlayer.onUpdateAbilities`). The expected
//! sequences were seen on a vanilla 26.3 server with two real clients (tools/wp58/p63_meta.py).

use bytes::Bytes;
use kiln_data::packets::play::clientbound as id;
use kiln_link::{PlayIn, ToSim};
use kiln_data::entities::data;
use kiln_proto::codec::Reader;
use kiln_proto::packets::entity::{DataValue, EntityData};
use kiln_sim::testing::{Client, SinkStats, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;

struct World {
    sim: Sim,
    actor: Client,
    watcher: Client,
}

fn log_of(stats: &Arc<SinkStats>) {
    *stats.log.lock().unwrap() = Some(Vec::new());
}

fn take(stats: &Arc<SinkStats>) -> Vec<Bytes> {
    stats.log.lock().unwrap().as_mut().unwrap().drain(..).collect()
}

fn packet_id(p: &Bytes) -> i32 {
    Reader::new(p).varint().unwrap()
}

fn ids(log: &[Bytes]) -> Vec<i32> {
    log.iter().map(packet_id).collect()
}

/// The entity data packets for `entity`: the entries after the entity id, the end marker left off.
fn data_of(log: &[Bytes], entity: i32) -> Vec<Vec<u8>> {
    log.iter()
        .filter(|p| packet_id(p) == id::SET_ENTITY_DATA)
        .filter_map(|p| {
            let mut r = Reader::new(p);
            r.varint().ok()?;
            (r.varint().ok()? == entity).then(|| {
                let rest = p[p.len() - r.remaining()..].to_vec();
                assert_eq!(rest.last(), Some(&0xff), "terminated");
                rest[..rest.len() - 1].to_vec()
            })
        })
        .collect()
}

/// The entries of one entity data packet for `entity` as the encoder writes them.
fn entries(d: &EntityData) -> Vec<u8> {
    d.entries().to_vec()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

impl World {
    fn new() -> Self {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (m1, s1) = join(1, "Actor", 2);
        let (m2, s2) = join(2, "Watcher", 2);
        log_of(&s1);
        log_of(&s2);
        assert!(sim.step([m1, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival Actor".into())]));
        assert!(sim.step([m2, ToSim::Console("gamemode creative Watcher".into())]));
        let mut w = Self { sim, actor: Client::new(1, s1), watcher: Client::new(2, s2) };
        w.ticks(8);
        w
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.actor.tick(None, &mut inbox);
            self.watcher.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, command: &str) {
        assert!(self.sim.step([ToSim::Console(command.into())]));
    }

    fn actor_log(&self) -> Vec<Bytes> {
        take(&self.actor.stats)
    }

    fn watcher_log(&self) -> Vec<Bytes> {
        take(&self.watcher.stats)
    }

    fn entity(&self) -> i32 {
        self.sim.entity_id(1).unwrap()
    }

    fn respawn(&mut self) {
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::ClientCommand(kiln_proto::packets::serverbound::ClientCommand::PerformRespawn))]));
        self.ticks(3);
    }
}

/// `ServerEntity.sendDirtyEntityData`: the watcher and the player get health when it changes (id 9), and nothing else
/// while nothing else changed.
#[test]
fn health_goes_to_the_viewers_and_the_player_alone() {
    let mut w = World::new();
    w.run("gamerule minecraft:natural_health_regeneration false");
    w.ticks(2);
    let (_, _) = (w.actor_log(), w.watcher_log());
    w.run("damage Actor 4 minecraft:generic");
    w.ticks(3);
    let e = w.entity();
    let mut want = EntityData::new();
    want.set(data::living_entity::HEALTH, &DataValue::Float(16.0));
    for (who, log) in [("watcher", w.watcher_log()), ("player", w.actor_log())] {
        assert_eq!(data_of(&log, e), [entries(&want)], "{who}: one packet, only the health");
    }
    // A quiet second: no entity data at all.
    w.ticks(20);
    assert!(data_of(&w.watcher_log(), e).is_empty());
    assert!(data_of(&w.actor_log(), e).is_empty());
}

/// Only what changed: a glowing effect with hidden particles changes the shared flags and the ambience flag, not the
/// pose or the living-entity flags (the old code sent the shared flags, pose and living flags together every time).
#[test]
fn only_the_dirty_entries_are_sent() {
    let mut w = World::new();
    w.watcher_log();
    w.actor_log();
    w.run("effect give Actor minecraft:glowing 100 0 true");
    w.ticks(3);
    let e = w.entity();
    let mut glowing = EntityData::new();
    glowing.set(data::entity::SHARED_FLAGS, &DataValue::Byte(64));
    glowing.set(data::living_entity::EFFECT_AMBIENCE, &DataValue::Boolean(true));
    for log in [w.watcher_log(), w.actor_log()] {
        assert_eq!(data_of(&log, e), [entries(&glowing)]);
    }
    w.run("effect clear Actor");
    w.ticks(3);
    let mut cleared = EntityData::new();
    cleared.set(data::entity::SHARED_FLAGS, &DataValue::Byte(0));
    cleared.set(data::living_entity::EFFECT_AMBIENCE, &DataValue::Boolean(false));
    assert_eq!(data_of(&w.watcher_log(), e), [entries(&cleared)]);
    // Absorption is an entry of its own (id 17, a float): 8 points for level II.
    w.actor_log();
    w.run("effect give Actor minecraft:absorption 100 1");
    w.ticks(3);
    let data = data_of(&w.watcher_log(), e);
    assert!(data.iter().any(|d| contains(d, &[17, 3, 0x41, 0, 0, 0])), "absorption: {data:?}");
}

/// The air supply goes to the viewers too, not only to the player.
#[test]
fn air_is_seen_by_the_viewers() {
    let mut w = World::new();
    let e = w.entity();
    w.watcher_log();
    w.actor_log();
    let p = w.actor.pos;
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:water", x - 1, y, z - 1, x + 1, y + 3, z + 1));
    w.ticks(10);
    // One entry, the air supply (id 1, an int of one or two bytes).
    let short = |d: &Vec<u8>| d.len() <= 4 && d[0] == 1;
    assert!(data_of(&w.watcher_log(), e).iter().any(short), "air supply for the viewer");
    assert!(data_of(&w.actor_log(), e).iter().any(short), "and the player");
}

/// `ServerEntity.sendPairingData`: a viewer that comes later gets the non-default entries with the entity, among them
/// the real health (not a fixed 20).
#[test]
fn a_new_viewer_gets_the_real_health_with_the_player() {
    let mut w = World::new();
    w.run("gamerule minecraft:natural_health_regeneration false");
    w.run("damage Actor 13 minecraft:generic");
    w.ticks(3);
    let (msg, stats) = join(3, "Late", 2);
    log_of(&stats);
    assert!(w.sim.step([msg]));
    let mut late = Client::new(3, stats.clone());
    for _ in 0..8 {
        let mut inbox = Vec::new();
        late.tick(None, &mut inbox);
        w.actor.tick(None, &mut inbox);
        w.watcher.tick(None, &mut inbox);
        assert!(w.sim.step(inbox));
    }
    let log = take(&stats);
    let data = data_of(&log, w.entity());
    assert!(!data.is_empty(), "the actor was paired");
    let mut health = EntityData::new();
    health.set(data::living_entity::HEALTH, &DataValue::Float(7.0));
    assert!(contains(&data[0], &entries(&health)), "the health is among the entries: {:?}", data[0]);
}

fn abilities(p: &Bytes) -> u8 {
    let mut r = Reader::new(p);
    r.varint().unwrap();
    r.u8().unwrap()
}

/// `/gamemode`: abilities, the tab list update, the game event, the abilities again; nothing for the same mode.
#[test]
fn a_game_mode_change_sends_the_abilities_twice() {
    let mut w = World::new();
    w.actor_log();
    w.run("gamemode creative Actor");
    w.ticks(2);
    let log = w.actor_log();
    let want = [id::PLAYER_ABILITIES, id::PLAYER_INFO_UPDATE, id::GAME_EVENT, id::PLAYER_ABILITIES];
    let seen: Vec<i32> = ids(&log).into_iter().filter(|i| want.contains(i)).collect();
    assert_eq!(seen, want);
    let flags: Vec<u8> = log.iter().filter(|p| packet_id(p) == id::PLAYER_ABILITIES).map(abilities).collect();
    // Invulnerable (1), may fly (4), instant build (8).
    assert_eq!(flags, [13, 13]);
    w.run("gamemode spectator Actor");
    w.ticks(2);
    let flags: Vec<u8> = w.actor_log().iter().filter(|p| packet_id(p) == id::PLAYER_ABILITIES).map(abilities).collect();
    // Invulnerable, flying (2), may fly: a spectator flies.
    assert_eq!(flags, [7, 7]);
    w.run("gamemode survival Actor");
    w.ticks(2);
    let flags: Vec<u8> = w.actor_log().iter().filter(|p| packet_id(p) == id::PLAYER_ABILITIES).map(abilities).collect();
    assert_eq!(flags, [0, 0]);
    // The same mode again: nothing.
    w.run("gamemode survival Actor");
    w.ticks(2);
    assert!(!ids(&w.actor_log()).contains(&id::PLAYER_ABILITIES));
    // The viewer sees the tab list entry, nothing of the abilities.
    w.watcher_log();
    w.run("gamemode creative Actor");
    w.ticks(2);
    let seen = ids(&w.watcher_log());
    assert!(seen.contains(&id::PLAYER_INFO_UPDATE));
    assert!(!seen.contains(&id::PLAYER_ABILITIES));
}

/// `PlayerList.placeNewPlayer`: the abilities follow the difficulty in the join sequence.
#[test]
fn joining_sends_the_abilities_after_the_difficulty() {
    let w = World::new();
    // (World::new started the log before the join.)
    let _ = &w;
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (msg, stats) = join(1, "Joiner", 2);
    log_of(&stats);
    assert!(sim.step([msg]));
    let seen = ids(&take(&stats));
    let at = |i: i32| seen.iter().position(|x| *x == i).unwrap_or_else(|| panic!("packet {i} missing in {seen:?}"));
    assert!(at(id::LOGIN) < at(id::CHANGE_DIFFICULTY));
    assert!(at(id::CHANGE_DIFFICULTY) < at(id::PLAYER_ABILITIES));
    assert!(at(id::PLAYER_ABILITIES) < at(id::PLAYER_POSITION));
    assert_eq!(seen.iter().filter(|i| **i == id::COMMANDS).count(), 1);
}

/// `/op` and `/deop`: the level event and the command tree together, once per change.
#[test]
fn op_and_deop_send_the_level_and_the_commands() {
    let mut w = World::new();
    w.actor_log();
    let count = |log: &[Bytes], i: i32| ids(log).iter().filter(|x| **x == i).count();
    w.run("op Actor");
    w.ticks(2);
    let log = w.actor_log();
    assert_eq!((count(&log, id::ENTITY_EVENT), count(&log, id::COMMANDS)), (1, 1), "/op");
    w.run("op Actor");
    w.ticks(2);
    let log = w.actor_log();
    assert_eq!((count(&log, id::ENTITY_EVENT), count(&log, id::COMMANDS)), (0, 0), "already an operator");
    w.run("deop Actor");
    w.ticks(2);
    let log = w.actor_log();
    assert_eq!((count(&log, id::ENTITY_EVENT), count(&log, id::COMMANDS)), (1, 1), "/deop");
}

/// `PlayerList.respawn`: the abilities before the Respawn packet, then the difficulty, the level event and the commands.
#[test]
fn a_respawn_sends_the_abilities_the_difficulty_and_the_commands() {
    let mut w = World::new();
    w.run("op Actor");
    w.ticks(2);
    w.run("kill Actor");
    w.ticks(3);
    w.actor_log();
    w.respawn();
    let log = w.actor_log();
    let seen = ids(&log);
    let at = |i: i32| seen.iter().position(|x| *x == i).unwrap_or_else(|| panic!("packet {i} missing in {seen:?}"));
    assert!(at(id::PLAYER_ABILITIES) < at(id::RESPAWN), "abilities first: {seen:?}");
    assert!(at(id::RESPAWN) < at(id::CHANGE_DIFFICULTY));
    assert!(at(id::CHANGE_DIFFICULTY) < at(id::COMMANDS));
    assert_eq!(seen.iter().filter(|i| **i == id::COMMANDS).count(), 1);
    assert_eq!(seen.iter().filter(|i| **i == id::PLAYER_ABILITIES).count(), 1);
    // The client's new player has the default entity data: the first pack sends what differs (health 20, skin parts).
    let e = w.entity();
    let data = data_of(&log, e);
    assert!(data.iter().any(|d| contains(d, &[9, 3])), "health after the respawn: {data:?}");
}

/// `ServerPlayer.teleport` to another level: the Respawn packet, the difficulty, the level event, the commands, the
/// position and the abilities; the same level is the position alone.
#[test]
fn a_dimension_change_sends_the_commands_and_the_abilities() {
    let mut w = World::new();
    w.run("gamemode creative Actor");
    w.ticks(2);
    w.actor_log();
    w.run("execute in minecraft:the_nether run tp Actor 0 100 0");
    w.ticks(6);
    let seen = ids(&w.actor_log());
    let at = |i: i32| seen.iter().position(|x| *x == i).unwrap_or_else(|| panic!("packet {i} missing in {seen:?}"));
    assert!(at(id::RESPAWN) < at(id::CHANGE_DIFFICULTY));
    assert!(at(id::CHANGE_DIFFICULTY) < at(id::COMMANDS));
    assert!(at(id::COMMANDS) < at(id::PLAYER_POSITION));
    assert!(at(id::PLAYER_POSITION) < at(id::PLAYER_ABILITIES));
    assert_eq!(seen.iter().filter(|i| **i == id::COMMANDS).count(), 1);
    // The same level: only the position.
    w.run("execute in minecraft:the_nether run tp Actor 5 100 5");
    w.ticks(3);
    let seen = ids(&w.actor_log());
    assert!(seen.contains(&id::PLAYER_POSITION));
    assert!(!seen.contains(&id::PLAYER_ABILITIES) && !seen.contains(&id::COMMANDS) && !seen.contains(&id::RESPAWN));
}

/// `LivingEntity.tickDeath`: twenty ticks after a player died its body poofs (entity event 60) and its viewers forget it;
/// it comes back with the respawn.
#[test]
fn a_dead_players_body_poofs_and_goes_after_twenty_ticks() {
    let mut w = World::new();
    let e = w.entity();
    let events = |log: &[Bytes]| -> Vec<u8> {
        log.iter()
            .filter(|p| packet_id(p) == id::ENTITY_EVENT)
            .filter_map(|p| {
                let mut r = Reader::new(p);
                r.varint().ok()?;
                (r.i32().ok()? == e).then(|| r.u8().unwrap())
            })
            .collect()
    };
    w.watcher_log();
    w.actor_log();
    w.run("kill Actor");
    w.ticks(10);
    let early = w.watcher_log();
    assert_eq!(events(&early), [3], "the death animation, no poof yet");
    assert!(!ids(&early).contains(&id::REMOVE_ENTITIES));
    w.ticks(15);
    let late = w.watcher_log();
    assert_eq!(events(&late), [60], "the poof");
    assert!(ids(&late).contains(&id::REMOVE_ENTITIES), "the viewer forgets the body");
    assert_eq!(events(&w.actor_log()), [3, 60], "the player sees both");
    // Back with the respawn.
    w.respawn();
    w.ticks(3);
    assert!(ids(&w.watcher_log()).contains(&id::ADD_ENTITY));
}
