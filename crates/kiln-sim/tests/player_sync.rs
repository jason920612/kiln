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

/// wp67: a start-gliding command the server refuses (on the ground, no elytra) still sends the shared flags
/// (`stopFallFlying` sets the gliding flag and clears it): the client started gliding when it sent the command, and
/// without them it keeps gliding on its own (and later refuses to send the command again).
#[test]
fn a_refused_glide_start_sends_the_flags_again() {
    let mut w = World::new();
    let e = w.entity();
    w.watcher_log();
    w.actor_log();
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 6 })]));
    w.ticks(2);
    let mut flags = EntityData::new();
    flags.set(data::entity::SHARED_FLAGS, &DataValue::Byte(0));
    for (who, log) in [("watcher", w.watcher_log()), ("player", w.actor_log())] {
        assert_eq!(data_of(&log, e), [entries(&flags)], "{who}: the flags, not gliding");
    }
    // Once: nothing more while nothing changes.
    w.ticks(20);
    assert!(data_of(&w.actor_log(), e).is_empty());
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

/// `Player.giveExperiencePoints` raises the score (entity data id 18, which the death screen shows) for the viewers and the
/// player; levels do not; it is saved as `Score`.
#[test]
fn experience_points_raise_the_score() {
    let mut w = World::new();
    let e = w.entity();
    w.watcher_log();
    w.actor_log();
    w.run("xp add Actor 7 levels");
    w.ticks(2);
    assert!(data_of(&w.watcher_log(), e).is_empty(), "levels leave the score alone");
    w.run("xp add Actor 7 points");
    w.ticks(2);
    let mut score = EntityData::new();
    score.set(data::player::SCORE, &DataValue::Int(7));
    assert_eq!(data_of(&w.watcher_log(), e), [entries(&score)]);
    assert_eq!(data_of(&w.actor_log(), e), [entries(&score)]);
    let saved = w.sim.saved_player_nbt(1).unwrap();
    assert_eq!(saved.get("Score").and_then(kiln_proto::nbt::Tag::as_i64), Some(7));
}

/// A new player's viewers are paired with its data, and the first pack (everything that differs from the defaults,
/// as the constructor's sets left it dirty) follows: vanilla sends the entries twice at a join.
#[test]
fn a_joining_player_is_paired_with_its_data_and_then_packs_it_again() {
    let mut sim = Sim::new(SimConfig::new(4, 4, None));
    let (m1, s1) = join(1, "Watcher", 2);
    log_of(&s1);
    assert!(sim.step([m1, ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
    let mut watcher = Client::new(1, s1.clone());
    let (m2, s2) = join(2, "Actor", 2);
    assert!(sim.step([m2]));
    let mut actor = Client::new(2, s2);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        watcher.tick(None, &mut inbox);
        actor.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let e = sim.entity_id(2).unwrap();
    let data = data_of(&take(&s1), e);
    assert_eq!(data.len(), 2, "pairing, then the first pack: {data:?}");
    assert_eq!(data[0], data[1]);
    let mut want = EntityData::new();
    want.set(data::living_entity::HEALTH, &DataValue::Float(20.0));
    want.set(data::avatar::PLAYER_MODE_CUSTOMISATION, &DataValue::Byte(127));
    assert_eq!(data[0], entries(&want).into_iter().collect::<Vec<u8>>(), "the health and the skin parts (the client's default)");
}

/// `LivingEntity.setSprinting` changes the sprint speed modifier, a syncable attribute: the viewers get an Update Attributes
/// with the flag; an invisibility effect (a modifier of the locator bar's range, which clients are not told) sends none.
#[test]
fn sprinting_sends_the_attributes_and_invisibility_does_not() {
    let mut w = World::new();
    let e = w.entity();
    let count = |log: &[Bytes], i: i32| ids(log).iter().filter(|x| **x == i).count();
    w.watcher_log();
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 1 })]));
    w.ticks(2);
    let log = w.watcher_log();
    let mut sprint = EntityData::new();
    sprint.set(data::entity::SHARED_FLAGS, &DataValue::Byte(8));
    assert_eq!(data_of(&log, e), [entries(&sprint)]);
    assert_eq!(count(&log, id::UPDATE_ATTRIBUTES), 1, "the sprint speed modifier");
    assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerCommand { action: 2 })]));
    w.ticks(2);
    assert_eq!(count(&w.watcher_log(), id::UPDATE_ATTRIBUTES), 1, "and its removal");
    w.run("effect give Actor minecraft:invisibility 100 0 true");
    w.ticks(2);
    assert_eq!(count(&w.watcher_log(), id::UPDATE_ATTRIBUTES), 0);
    w.run("effect give Actor minecraft:speed 100 0 true");
    w.ticks(2);
    assert_eq!(count(&w.watcher_log(), id::UPDATE_ATTRIBUTES), 1, "speed is a movement speed modifier");
}

// ---- wp64: arrows and stingers in the body, spectators, sneaking, instant effects, powder snow ----

impl World {
    /// `n` ticks from `start` (which runs the first one), each with the entity data packets of the actor the watcher and the
    /// actor got in it.
    fn data_ticks(&mut self, n: usize, start: impl FnOnce(&mut Self)) -> Vec<(Vec<Vec<u8>>, Vec<Vec<u8>>)> {
        let e = self.entity();
        let mut out = Vec::new();
        start(self);
        out.push((data_of(&self.watcher_log(), e), data_of(&self.actor_log(), e)));
        for _ in 1..n {
            self.ticks(1);
            out.push((data_of(&self.watcher_log(), e), data_of(&self.actor_log(), e)));
        }
        out
    }
}

fn one(field: kiln_data::entities::DataField, value: DataValue) -> Vec<u8> {
    let mut d = EntityData::new();
    d.set(field, &value);
    entries(&d)
}

/// `AbstractArrow.onHitEntity`: a landed arrow (not a piercing one, none on an invulnerable player) raises the arrow count
/// (entity data 12) for the viewers and the player; `LivingEntity.tick` drops one after `20 * (30 - count)` ticks, the time
/// worked out when the count is above 0 and the timer has run out.
#[test]
fn an_arrow_sticks_in_the_body_and_falls_out_in_time() {
    use kiln_data::entities::data::living_entity::ARROW_COUNT;
    let mut w = World::new();
    w.run("gamerule minecraft:natural_health_regeneration false");
    w.ticks(2);
    let p = w.actor.pos;
    let (w1, a1) = (one(ARROW_COUNT, DataValue::Int(1)), one(ARROW_COUNT, DataValue::Int(0)));
    w.watcher_log();
    w.actor_log();
    let summon = format!("summon minecraft:arrow {} {} {} {{Motion:[0.0,-1.5,0.0]}}", p[0], p[1] + 2.5, p[2]);
    let ticks = w.data_ticks(700, |w| w.run(&summon));
    let at = |want: &[u8], who: usize| ticks.iter().position(|t| [&t.0, &t.1][who].iter().any(|d| contains(d, want)));
    let added = at(&w1, 0).expect("the watcher is told of the arrow");
    assert_eq!(at(&w1, 1), Some(added), "the player at the same time");
    assert!(added < 5, "the arrow hit at once: tick {added}");
    let gone = at(&a1, 0).expect("and of the arrow falling out");
    assert_eq!(at(&a1, 1), Some(gone));
    assert_eq!(gone - added, 580, "20 * (30 - 1) ticks");
    // A creative player is not hurt: no arrow in the body.
    w.run("gamemode creative Actor");
    w.ticks(2);
    w.watcher_log();
    let ticks = w.data_ticks(5, |w| w.run(&summon));
    assert!(!ticks.iter().any(|t| t.0.iter().any(|d| contains(d, &w1))), "an invulnerable player takes none");
}

/// wp75: the same for a mob (`LivingEntity` keeps the counts, not `Player`): the viewers are told of the arrow, and when it falls
/// out the count goes back to 0 (a mob's entity data lists non-default values only, so the reset has to be sent).
#[test]
fn an_arrow_sticks_in_a_mobs_body_and_falls_out_in_time() {
    use kiln_data::entities::data::living_entity::ARROW_COUNT;
    let mut w = World::new();
    w.run("gamerule minecraft:natural_health_regeneration false");
    w.ticks(2);
    let p = w.actor.pos;
    w.run(&format!("summon minecraft:pig {} {} {} {{NoAI:1b,Health:100f,Attributes:[{{id:\"minecraft:max_health\",base:100.0d}}]}}", p[0] + 3.0, p[1], p[2]));
    w.ticks(3);
    let pig = w.sim.entity_ids_of("minecraft:pig")[0];
    let (one, none) = (one(ARROW_COUNT, DataValue::Int(1)), one(ARROW_COUNT, DataValue::Int(0)));
    w.watcher_log();
    let summon = format!("summon minecraft:arrow {} {} {} {{Motion:[0.0,-1.5,0.0]}}", p[0] + 3.0, p[1] + 2.5, p[2]);
    let mut ticks = Vec::new();
    w.run(&summon);
    ticks.push(data_of(&w.watcher_log(), pig));
    for _ in 1..700 {
        w.ticks(1);
        ticks.push(data_of(&w.watcher_log(), pig));
    }
    let at = |want: &[u8]| ticks.iter().position(|t| t.iter().any(|d| contains(d, want)));
    let added = at(&one).expect("the viewer is told of the arrow in the pig");
    assert!(added < 5, "the arrow hit at once: tick {added}");
    let gone = at(&none).expect("and of the arrow falling out");
    assert_eq!(gone - added, 580, "20 * (30 - 1) ticks");
}

/// `Bee.doHurtTarget`: a sting raises the stinger count (entity data 13), which falls out like the arrows do.
#[test]
fn a_sting_leaves_a_stinger_in_the_body() {
    use kiln_data::entities::data::living_entity::STINGER_COUNT;
    let mut w = World::new();
    w.run("difficulty normal");
    w.ticks(2);
    let p = w.actor.pos;
    let one_stinger = one(STINGER_COUNT, DataValue::Int(1));
    w.watcher_log();
    w.actor_log();
    w.run(&format!("summon minecraft:bee {} {} {} {{}}", p[0] + 0.5, p[1], p[2] + 0.5));
    w.ticks(2);
    // Hit by the player, the bee is angry at it (`HurtByTargetGoal`) and comes for it.
    let ticks = w.data_ticks(200, |w| w.run("damage @e[type=minecraft:bee,limit=1] 1 minecraft:player_attack by Actor"));
    let added = ticks.iter().position(|t| t.0.iter().any(|d| contains(d, &one_stinger))).expect("the watcher is told of the stinger");
    assert!(ticks[added].1.iter().any(|d| contains(d, &one_stinger)), "the player at the same time");
    // One sting only: the bee dies of it.
    let more = ticks.iter().flat_map(|t| &t.0).filter(|d| contains(d, &one_stinger)).count();
    assert_eq!(more, 1);
}

/// `ServerPlayer.broadcastToPlayer`: a spectator is hidden from the players that are not spectators and seen by the
/// spectators; changing the game mode works out who sees whom again before the data of the tick goes out.
#[test]
fn spectators_are_hidden_from_everyone_but_spectators() {
    let mut w = World::new();
    let e = w.entity();
    let watcher_id = w.sim.entity_id(2).unwrap();
    w.watcher_log();
    w.actor_log();
    w.run("gamemode spectator Actor");
    w.ticks(3);
    let log = w.watcher_log();
    let seen = ids(&log);
    assert!(seen.contains(&id::PLAYER_INFO_UPDATE), "the tab list entry stays: {seen:?}");
    assert!(seen.contains(&id::REMOVE_ENTITIES), "a non-spectator forgets the spectator");
    assert!(data_of(&log, e).is_empty(), "no data of the spectator reaches it: the spectator went before the pack");
    // The spectator's own data (the invisible flag) reaches it.
    let flag = one(data::entity::SHARED_FLAGS, DataValue::Byte(32));
    assert!(data_of(&w.actor_log(), e).iter().any(|d| contains(d, &flag)), "the player is told it is invisible");
    // Back to survival: paired again.
    w.run("gamemode survival Actor");
    w.ticks(3);
    assert!(ids(&w.watcher_log()).contains(&id::ADD_ENTITY), "seen again");
    // Both spectators see each other.
    w.run("gamemode spectator Actor");
    w.ticks(3);
    w.run("gamemode spectator Watcher");
    w.ticks(3);
    let seen = ids(&w.watcher_log());
    assert!(seen.contains(&id::ADD_ENTITY), "a spectator sees the other: {seen:?}");
    // The actor leaves spectator mode: the watcher (still a spectator) disappears for it, not the other way round.
    w.actor_log();
    w.watcher_log();
    w.run("gamemode survival Actor");
    w.ticks(3);
    let (actor_saw, watcher_saw) = (w.actor_log(), w.watcher_log());
    let removes = |log: &[Bytes], who: i32| {
        log.iter().filter(|p| packet_id(p) == id::REMOVE_ENTITIES).any(|p| {
            let mut r = Reader::new(p);
            r.varint().unwrap();
            let n = r.varint().unwrap();
            (0..n).any(|_| r.varint().unwrap() == who)
        })
    };
    assert!(removes(&actor_saw, watcher_id), "the actor no longer sees the spectating watcher");
    assert!(!removes(&watcher_saw, e), "the spectating watcher still sees the actor");
}

/// Vanilla's input packet changes the shift flag at once and the player's tick the pose: the viewers get the flag in one
/// tick and the pose in the next.
#[test]
fn sneaking_sends_the_flag_and_the_pose_a_tick_apart() {
    use kiln_data::entities::pose;
    let mut w = World::new();
    w.watcher_log();
    w.actor_log();
    let (flag_on, pose_on) = (one(data::entity::SHARED_FLAGS, DataValue::Byte(2)), one(data::entity::POSE, DataValue::Pose(pose::CROUCHING)));
    let ticks = w.data_ticks(3, |w| assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerInput { flags: 0x20 })])));
    assert_eq!(ticks[0].0, [flag_on.clone()], "the flag, alone");
    assert_eq!(ticks[1].0, [pose_on.clone()], "the pose a tick later");
    assert!(ticks[2].0.is_empty());
    assert_eq!((ticks[0].1.clone(), ticks[1].1.clone()), (ticks[0].0.clone(), ticks[1].0.clone()), "the player gets the same");
    let (flag_off, pose_off) = (one(data::entity::SHARED_FLAGS, DataValue::Byte(0)), one(data::entity::POSE, DataValue::Pose(pose::STANDING)));
    let ticks = w.data_ticks(3, |w| assert!(w.sim.step([ToSim::Packet(1, PlayIn::PlayerInput { flags: 0 })])));
    assert_eq!((&ticks[0].0[..], &ticks[1].0[..]), (&[flag_off][..], &[pose_off][..]));
}

/// `/effect give` of an instant effect: the particle goes out with the pack before the tick that applies the effect and
/// removes it, and the removal (with the health) with the next pack; no other packet carries the particle.
#[test]
fn an_instant_effect_shows_its_particle_for_a_tick() {
    use kiln_data::entities::data::living_entity::{EFFECT_PARTICLES, HEALTH};
    let mut w = World::new();
    w.run("gamerule minecraft:natural_health_regeneration false");
    w.run("damage Actor 8 minecraft:generic");
    w.ticks(3);
    w.watcher_log();
    w.actor_log();
    let ticks = w.data_ticks(3, |w| w.run("effect give Actor minecraft:instant_health 1 0"));
    let empty = one(EFFECT_PARTICLES, DataValue::Particles(Vec::new()));
    let prefix = &empty[..2];
    // Tick 0: one entry, the particle list (not empty); the health is not in it yet.
    let first = &ticks[0].0;
    assert_eq!(first.len(), 1);
    assert!(first[0].starts_with(prefix) && first[0] != empty, "the particle: {:?}", first[0]);
    assert!(!contains(&first[0], &one(HEALTH, DataValue::Float(16.0))[..2]));
    // Tick 1: the effect ended and healed (4 hearts' worth is 4 points): the health and the empty list.
    let second = &ticks[1].0;
    assert_eq!(second.len(), 1);
    assert!(contains(&second[0], &empty), "the particle is gone: {:?}", second[0]);
    assert!(contains(&second[0], &one(HEALTH, DataValue::Float(16.0))[..2]), "with the health: {:?}", second[0]);
    assert!(ticks[2].0.is_empty());
    assert_eq!((ticks[0].1.clone(), ticks[1].1.clone()), (ticks[0].0.clone(), ticks[1].0.clone()));
}

/// `PowderSnowBlock.entityInside`: a player that moves in powder snow throws snowflakes (`nextBoolean` for each pass of the
/// movements the tick records through the block: about 1.4 a tick, as many as a vanilla server sent in the same walk), seen by
/// every player within 32 blocks and by the player itself.
#[test]
fn walking_in_powder_snow_throws_snowflakes() {
    let mut w = World::new();
    let p = w.actor.pos;
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:powder_snow", x - 7, y, z - 7, x + 7, y + 1, z + 7));
    w.ticks(2);
    w.watcher_log();
    w.actor_log();
    let snowflake = kiln_data::builtin_id("minecraft:particle_type", "minecraft:snowflake").unwrap();
    let flakes = |log: &[Bytes]| {
        log.iter()
            .filter(|p| packet_id(p) == id::LEVEL_PARTICLES)
            .filter(|p| {
                let mut r = Reader::new(p);
                r.varint().unwrap();
                r.varint().unwrap() == snowflake
            })
            .count()
    };
    let mut pos = p;
    for tick in 0..80 {
        pos[0] += if (tick / 30) % 2 == 0 { 0.1 } else { -0.1 };
        let mut inbox = Vec::new();
        w.actor.tick(Some(pos), &mut inbox);
        w.watcher.tick(None, &mut inbox);
        assert!(w.sim.step(inbox));
    }
    let (seen, own) = (flakes(&w.watcher_log()), flakes(&w.actor_log()));
    assert_eq!(seen, own, "the player sees its own snowflakes");
    assert!((90..=140).contains(&seen), "a vanilla server sent 101 in 80 ticks of the same walk: {seen}");
    // Standing still: none.
    w.ticks(2);
    w.watcher_log();
    w.ticks(40);
    assert_eq!(flakes(&w.watcher_log()), 0);
}

/// `removeFrost` and `tryAddFrost` mark the movement speed attribute changed every tick a player is frozen, and the
/// Update Attributes lists only the attributes that changed (one here, with the frost modifier), to the viewers and the player.
#[test]
fn a_frozen_player_gets_the_speed_attribute_every_tick() {
    let mut w = World::new();
    // (No regeneration: a change of the health is entity data, which would open the tick for the attributes.)
    w.run("gamerule minecraft:natural_health_regeneration false");
    let p = w.actor.pos;
    let (x, y, z) = (p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32);
    w.run(&format!("fill {} {} {} {} {} {} minecraft:powder_snow", x - 2, y, z - 2, x + 2, y + 1, z + 2));
    w.ticks(3);
    w.watcher_log();
    w.actor_log();
    // The number of attributes in each Update Attributes for the actor.
    let e = w.entity();
    let listed = |log: &[Bytes]| -> Vec<i32> {
        log.iter()
            .filter(|p| packet_id(p) == id::UPDATE_ATTRIBUTES)
            .filter_map(|p| {
                let mut r = Reader::new(p);
                r.varint().unwrap();
                (r.varint().unwrap() == e).then(|| r.varint().unwrap())
            })
            .collect()
    };
    w.ticks(10);
    for (who, log) in [("watcher", w.watcher_log()), ("player", w.actor_log())] {
        assert_eq!(listed(&log), [1; 10], "{who}: one attribute, every tick");
    }
    // Fully frozen and standing still nothing else changes: the attribute goes out in the update ticks only (every second one),
    // as a vanilla server did (25 packets a second for two frozen players, one every 100 ms each).
    w.ticks(150);
    w.watcher_log();
    w.ticks(40);
    let n = listed(&w.watcher_log()).len();
    assert!((19..=22).contains(&n), "every second tick: {n} in 40");
    // Out of the snow: the thaw takes the modifier off (one more packet) and then it is quiet.
    w.run(&format!("fill {} {} {} {} {} {} minecraft:air", x - 2, y, z - 2, x + 2, y + 1, z + 2));
    w.ticks(120);
    let log = w.watcher_log();
    let n = listed(&log);
    assert!(n.iter().all(|c| *c == 1), "{n:?}");
    w.ticks(20);
    assert!(listed(&w.watcher_log()).is_empty(), "nothing once the frost is gone");
}
