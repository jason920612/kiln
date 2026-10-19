//! wp84 (the item x mob matrix with a real client on both servers): a lead goes on a camel husk, a zombie horse and a zombie nautilus (`Mob.canBeLeashed` is
//! "not an Enemy"), a saddle goes on an untamed skeleton horse (`SkeletonHorse.canUseSlot` is true), and the allay's "item given" sound is for everyone but the player
//! who gave it (`level.playSound(player, this, ...)`).

use kiln_link::{PlayIn, ToSim};
use kiln_proto::nbt::Tag;
use kiln_proto::packets::serverbound::Hand;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};

struct World {
    sim: Sim,
    client: Client,
    stats: std::sync::Arc<kiln_sim::testing::SinkStats>,
}

impl World {
    fn new() -> Self {
        if std::env::var_os("KILN_DATAPACK").is_none() {
            let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/generated");
            if dir.join("data/minecraft/recipe").is_dir() {
                unsafe { std::env::set_var("KILN_DATAPACK", dir) };
            }
        }
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Keeper", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        let keep = stats.clone();
        assert!(sim.step([msg, ToSim::Console("gamerule minecraft:spawn_mobs false".into()), ToSim::Console("gamemode survival Keeper".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        World { sim, client, stats: keep }
    }

    fn ticks(&mut self, n: usize) {
        for _ in 0..n {
            let mut inbox = Vec::new();
            self.client.tick(None, &mut inbox);
            assert!(self.sim.step(inbox));
        }
    }

    fn run(&mut self, cmd: &str) {
        assert!(self.sim.step([ToSim::Console(cmd.into())]));
    }

    /// Gives `item`, summons `mob` (no AI) 1.5 blocks away, clicks it; returns its data.
    fn click(&mut self, item: &str, mob: &str) -> Tag {
        let p = self.client.pos;
        self.run(&format!("give Keeper {item} 2"));
        self.run(&format!("summon minecraft:{mob} {} {} {} {{NoAI:1b,PersistenceRequired:1b}}", p[0] + 1.5, p[1], p[2]));
        self.ticks(3);
        let id = self.sim.entity_ids_of(&format!("minecraft:{mob}"))[0];
        assert!(self.sim.step([ToSim::Packet(1, PlayIn::Interact { entity_id: id, hand: Hand::Main, location: [0.0, 0.5, 0.0], sneaking: false })]));
        self.ticks(3);
        self.sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some(&format!("minecraft:{mob}"))).unwrap_or(Tag::Compound(vec![]))
    }
}

#[test]
fn a_lead_goes_on_the_undead_mounts() {
    for mob in ["camel_husk", "zombie_horse", "zombie_nautilus"] {
        let mut w = World::new();
        let t = w.click("minecraft:lead", mob);
        assert!(t.get("leash").is_some(), "{mob} is led: {t:?}");
    }
}

#[test]
fn a_saddle_goes_on_an_untamed_skeleton_horse() {
    let mut w = World::new();
    let t = w.click("minecraft:saddle", "skeleton_horse");
    let saddled = t.get("equipment").and_then(|e| e.get("saddle")).is_some();
    assert!(saddled, "the skeleton horse wears the saddle: {t:?}");
}

#[test]
fn the_allay_item_given_sound_is_not_sent_to_the_player_who_gave() {
    let mut w = World::new();
    let _ = w.click("minecraft:cod", "allay");
    let id = kiln_data::builtin_id("minecraft:sound_event", "minecraft:entity.allay.item_given").unwrap();
    let packets = w.stats.log.lock().unwrap().clone().unwrap();
    let heard = packets
        .iter()
        .filter(|p| {
            let mut r = kiln_proto::codec::Reader::new(p);
            let pid = r.varint().ok();
            (pid == Some(kiln_data::packets::play::clientbound::SOUND) || pid == Some(kiln_data::packets::play::clientbound::SOUND_ENTITY)) && r.varint().ok() == Some(id + 1)
        })
        .count();
    assert_eq!(heard, 0, "the giving player plays the sound itself");
}

#[test]
fn a_sheared_mooshroom_puffs_an_explosion_particle() {
    let mut w = World::new();
    let _ = w.click("minecraft:shears", "mooshroom");
    let packets = w.stats.log.lock().unwrap().clone().unwrap();
    let puffs = packets.iter().filter(|p| kiln_proto::codec::Reader::new(p).varint().ok() == Some(kiln_data::packets::play::clientbound::LEVEL_PARTICLES)).count();
    assert!(puffs >= 1, "the explosion particle of the conversion");
}
