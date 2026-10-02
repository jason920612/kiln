//! What sits on a player's shoulders (`ShoulderEntityLeft`, `ShoulderEntityRight`): the saved
//! compounds of up to two parrots, kept and written back as they were, shown to other players
//! through the shoulder parrot variant fields of the player's entity data, and let go
//! (`ServerPlayer.removeEntitiesOnShoulder`) when the player falls, swims, flies, sleeps, stands
//! in powder snow, is hurt or starts a riptide spin, twenty ticks after they sat down at the
//! earliest. What is let go is saved as an entity in the chunk it lands in, where a parrot comes to
//! life at once (`respawnEntityOnShoulder`); a parrot on the ground takes a free shoulder
//! itself (`LandOnOwnersShoulderGoal`, [`mount`]).

use crate::Player;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

/// `Parrot.Variant`s (`LEGACY_CODEC` takes the id, clamped).
const PARROT_VARIANTS: i64 = 5;

/// The shoulder compound at `key` of the saved player data; an empty or absent one is `None`.
pub(crate) fn load(raw: &Tag, key: &str) -> Option<Tag> {
    match raw.get(key) {
        Some(Tag::Compound(fields)) if !fields.is_empty() => Some(Tag::Compound(fields.clone())),
        _ => None,
    }
}

/// `Player.extractParrotVariant`: the variant id of a parrot's compound.
fn variant(tag: &Option<Tag>) -> Option<i32> {
    let tag = tag.as_ref()?;
    if tag.get("id").and_then(Tag::as_str) != Some("minecraft:parrot") {
        return None;
    }
    tag.get("Variant").and_then(Tag::as_i64).map(|v| v.clamp(0, PARROT_VARIANTS - 1) as i32)
}

/// wp32 parrots: fills in what `LandOnOwnersShoulderGoal` and `ShoulderRidingEntity.setEntityOnShoulder`
/// read of each player in `views` (by entity id): whether a parrot may land (not a spectator, not
/// flying, not in water or powder snow) and whether it would be taken (a free shoulder, standing
/// on the ground, not riding).
pub(crate) fn mark_views(players: &[&mut Player], block: &dyn Fn(kiln_entity::math::BlockPos) -> u16, views: &mut [kiln_entity::level::PlayerView]) {
    for v in views.iter_mut() {
        let Some(p) = players.iter().find(|p| p.entity_id == v.id) else { continue };
        let (in_water, in_powder_snow) = footing(p, block);
        v.parrot_may_land = p.game_mode != 3 && !p.flying && !in_water && !in_powder_snow;
        v.parrot_can_sit = p.vehicle.is_none() && p.on_ground && !in_water && !in_powder_snow && p.shoulders.iter().any(Option::is_none);
    }
}

/// Whether the player is in water, and in powder snow.
fn footing(p: &Player, block: &dyn Fn(kiln_entity::math::BlockPos) -> u16) -> (bool, bool) {
    let in_water = p.fluids(block).in_water;
    let feet = kiln_entity::math::BlockPos::new(p.pos[0].floor() as i32, p.pos[1].floor() as i32, p.pos[2].floor() as i32);
    (in_water, block(feet) == kiln_data::blocks::default_state::POWDER_SNOW)
}

/// `ServerPlayer.setEntityOnShoulder` for a parrot that already left its world: the saved parrot
/// takes a free shoulder. If the player cannot carry it after all (another parrot was quicker
/// this tick), the compound comes back, to live in the world again.
pub(crate) fn mount(p: &mut Player, tag: Tag, game_time: i64, block: &dyn Fn(kiln_entity::math::BlockPos) -> u16) -> Option<Tag> {
    let (in_water, in_powder_snow) = footing(p, block);
    if p.set_entity_on_shoulder(tag.clone(), game_time, in_water, in_powder_snow) { None } else { Some(tag) }
}

impl Player {
    /// The two shoulder parrot fields of the player's entity data.
    pub(crate) fn shoulder_data(&self, d: &mut EntityData) {
        d.set(kiln_data::entities::data::player::SHOULDER_PARROT_LEFT, &DataValue::OptionalUnsignedInt(variant(&self.shoulders[0]).map(|v| v as u32)));
        d.set(kiln_data::entities::data::player::SHOULDER_PARROT_RIGHT, &DataValue::OptionalUnsignedInt(variant(&self.shoulders[1]).map(|v| v as u32)));
    }

    /// `ServerPlayer.setEntityOnShoulder`: a parrot sits on the left shoulder, or the right one
    /// when that is taken; nothing happens while riding, in the air, in water or in powder snow.
    pub(crate) fn set_entity_on_shoulder(&mut self, tag: Tag, game_time: i64, in_water: bool, in_powder_snow: bool) -> bool {
        if self.vehicle.is_some() || !self.on_ground || in_water || in_powder_snow {
            return false;
        }
        for i in 0..2 {
            if self.shoulders[i].is_none() {
                self.shoulders[i] = Some(tag);
                self.shoulder_time = game_time;
                self.shoulder_dirty = true;
                self.meta_dirty = true;
                return true;
            }
        }
        false
    }

    /// `ServerPlayer.handleShoulderEntities` (the end of `Player.aiStep`): an ambient call now and
    /// then, and the parrots fly off when the player falls, swims, flies, sleeps or stands in
    /// powder snow.
    pub(crate) fn handle_shoulder_entities(&mut self, game_time: i64, in_water: bool, in_powder_snow: bool) {
        for i in 0..2 {
            // `playShoulderEntityAmbientSound`: a parrot's call is one in 200 ticks (the sound
            // itself, with its imitations, is not simulated; the draw is).
            let silent = self.shoulders[i].as_ref().is_none_or(|t| t.get("Silent").and_then(Tag::as_i64) == Some(1));
            if !silent {
                self.entity_rng.next_int_bounded(200);
            }
        }
        if self.fall_distance > 0.5 || in_water || self.flying || self.sleep.pos.is_some() || in_powder_snow {
            self.remove_entities_on_shoulder(game_time);
        }
    }

    /// `ServerPlayer.removeEntitiesOnShoulder` and `respawnEntityOnShoulder`: both shoulders'
    /// entities are set down at the player's feet (a little above them), owned by the player.
    pub(crate) fn remove_entities_on_shoulder(&mut self, game_time: i64) {
        if self.shoulder_time + 20 >= game_time {
            return;
        }
        for i in 0..2 {
            let Some(Tag::Compound(mut fields)) = self.shoulders[i].take() else { continue };
            self.shoulder_dirty = true;
            self.meta_dirty = true;
            let put = |fields: &mut Vec<(String, Tag)>, key: &str, value: Tag| match fields.iter_mut().find(|(k, _)| k == key) {
                Some((_, v)) => *v = value,
                None => fields.push((key.to_owned(), value)),
            };
            // `TamableAnimal.setOwner`, then `setPos(x, y + 0.7, z)`.
            if fields.iter().any(|(k, v)| k == "id" && v.as_str() == Some("minecraft:parrot")) {
                let uuid = self.uuid.as_u128();
                put(&mut fields, "Owner", kiln_entity::persist::uuid_to_tag(uuid));
                put(&mut fields, "Sitting", Tag::Byte(0));
            }
            put(&mut fields, "Pos", Tag::List(vec![Tag::Double(self.pos[0]), Tag::Double(self.pos[1] + 0.699999988079071), Tag::Double(self.pos[2])]));
            self.released_shoulders.push(Tag::Compound(fields));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{Client, join};
    use crate::{Sim, SimConfig};
    use kiln_link::ToSim;
    use kiln_proto::nbt::Tag;
    use kiln_proto::packets::entity::{DataValue, EntityData};

    fn parrot(variant: i32) -> Tag {
        Tag::Compound(vec![("id".into(), Tag::String("minecraft:parrot".into())), ("Variant".into(), Tag::Int(variant))])
    }

    fn world() -> (Sim, Client) {
        let mut sim = Sim::new(SimConfig::new(4, 4, None));
        let (msg, stats) = join(1, "Perch", 2);
        assert!(sim.step([msg, ToSim::Console("gamemode survival Perch".into()), ToSim::Console("gamerule minecraft:spawn_mobs false".into())]));
        let mut client = Client::new(1, stats);
        for _ in 0..5 {
            let mut inbox = Vec::new();
            client.tick(None, &mut inbox);
            assert!(sim.step(inbox));
        }
        (sim, client)
    }

    #[test]
    fn a_parrot_flies_off_when_the_player_falls_but_not_in_its_first_second() {
        let (mut sim, _client) = world();
        {
            let p = sim.players.get_mut(&1).unwrap();
            p.shoulders = [Some(parrot(2)), None];
            // Sat down just now: kept for twenty ticks.
            p.shoulder_time = 1_000_000;
            p.flying = true;
        }
        assert!(sim.step([]));
        assert!(sim.players[&1].shoulders[0].is_some(), "twenty ticks have not passed");
        assert_eq!(sim.kept_entity_count(), 0);
        {
            let p = sim.players.get_mut(&1).unwrap();
            p.shoulder_time = -100;
            p.flying = true;
        }
        assert!(sim.step([]));
        assert!(sim.players[&1].shoulders[0].is_none(), "it left the shoulder");
        assert_eq!(sim.kept_entity_count(), 0, "and became a parrot of the chunk, which is simulated");
        assert_eq!(sim.entity_ids_of("minecraft:parrot").len(), 1);
    }

    /// Summons a parrot owned by player 1 at the player's feet.
    fn summon_owned_parrot(sim: &mut Sim) {
        let uuid = sim.players[&1].uuid.as_u128();
        let p = sim.players[&1].pos;
        let cmd = format!(
            "summon minecraft:parrot {} {} {} {{Owner:[I;{},{},{},{}],Variant:2,PersistenceRequired:1b}}",
            p[0],
            p[1],
            p[2],
            (uuid >> 96) as i32,
            (uuid >> 64) as i32,
            (uuid >> 32) as i32,
            uuid as i32
        );
        assert!(sim.step([ToSim::Console(cmd)]));
    }

    #[test]
    fn a_tame_parrot_lands_on_its_owners_shoulder_after_100_ticks_and_comes_back_when_the_player_flies() {
        let (mut sim, _client) = world();
        summon_owned_parrot(&mut sim);
        assert_eq!(sim.entity_ids_of("minecraft:parrot").len(), 1);
        for _ in 0..90 {
            assert!(sim.step([]));
        }
        assert!(sim.players[&1].shoulders[0].is_none(), "too young to land");
        for _ in 0..40 {
            assert!(sim.step([]));
        }
        let on_shoulder = sim.players[&1].shoulders[0].clone().expect("the parrot sits on the left shoulder");
        assert_eq!(on_shoulder.get("id").and_then(Tag::as_str), Some("minecraft:parrot"));
        assert_eq!(on_shoulder.get("Variant").and_then(Tag::as_i64), Some(2));
        assert!(sim.entity_ids_of("minecraft:parrot").is_empty(), "it is no longer in the world");
        // The viewers see it through the player's entity data.
        let mut d = EntityData::new();
        sim.players[&1].shoulder_data(&mut d);
        let mut want = EntityData::new();
        want.set(kiln_data::entities::data::player::SHOULDER_PARROT_LEFT, &DataValue::OptionalUnsignedInt(Some(2)));
        want.set(kiln_data::entities::data::player::SHOULDER_PARROT_RIGHT, &DataValue::OptionalUnsignedInt(None));
        assert_eq!(d.entries(), want.entries());
        // The player flies: the parrot is set down, a parrot of its own again, owned by the player.
        sim.players.get_mut(&1).unwrap().flying = true;
        assert!(sim.step([]));
        assert!(sim.players[&1].shoulders[0].is_none());
        assert_eq!(sim.entity_ids_of("minecraft:parrot").len(), 1);
        let nbt = sim.entity_nbt().into_iter().find(|t| t.get("id").and_then(Tag::as_str) == Some("minecraft:parrot")).unwrap();
        assert_eq!(nbt.get("Variant").and_then(Tag::as_i64), Some(2));
        assert!(nbt.get("Owner").is_some());
    }

    #[test]
    fn a_parrot_does_not_land_on_a_flying_players_shoulder() {
        let (mut sim, _client) = world();
        sim.players.get_mut(&1).unwrap().flying = true;
        summon_owned_parrot(&mut sim);
        for _ in 0..140 {
            assert!(sim.step([]));
        }
        assert!(sim.players[&1].shoulders[0].is_none());
        assert_eq!(sim.entity_ids_of("minecraft:parrot").len(), 1);
    }

    #[test]
    fn shoulder_parrots_are_saved_and_shown() {
        let (mut sim, _client) = world();
        sim.players.get_mut(&1).unwrap().shoulders = [None, Some(parrot(4))];
        let saved = sim.player_nbt(&sim.players[&1]);
        assert_eq!(saved.get("ShoulderEntityRight").and_then(|t| t.get("Variant")).and_then(Tag::as_i64), Some(4));
        assert!(saved.get("ShoulderEntityLeft").is_none());
        let mut d = EntityData::new();
        sim.players[&1].shoulder_data(&mut d);
        let mut want = EntityData::new();
        want.set(kiln_data::entities::data::player::SHOULDER_PARROT_LEFT, &DataValue::OptionalUnsignedInt(None));
        want.set(kiln_data::entities::data::player::SHOULDER_PARROT_RIGHT, &DataValue::OptionalUnsignedInt(Some(4)));
        assert_eq!(d.entries(), want.entries());
    }
}
