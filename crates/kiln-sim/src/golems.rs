//! Building golems (`CarvedPumpkinBlock.trySpawnGolem`): a carved pumpkin or jack o'lantern
//! placed on a T of iron blocks (arms either way) becomes a player-created iron golem; the
//! pattern's blocks break (level event 2001) and players within five blocks of the golem get
//! `summoned_entity`. Snow golems are not simulated yet.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Spawn};
use kiln_blocks::{BlockPos, Effect, Level, flags, state};
use kiln_data::blocks::default_state as d;

/// After a block was placed at `pos` by player `p`: builds an iron golem if the pumpkin
/// completes the pattern.
pub(crate) fn try_spawn_golem(p: &mut Player, level: &mut RegionLevel, pos: BlockPos, spawns: &mut Vec<Spawn>) {
    let head = level.block(pos);
    if !(state::same_block(head, d::CARVED_PUMPKIN) || state::same_block(head, d::JACK_O_LANTERN)) {
        return;
    }
    let iron = |l: &RegionLevel, at: BlockPos| state::same_block(l.block(at), d::IRON_BLOCK);
    let air = |l: &RegionLevel, at: BlockPos| kiln_data::blocks_types::is_air(l.block(at));
    let body = pos.below();
    let feet = body.below();
    if !iron(level, body) || !iron(level, feet) {
        return;
    }
    // Arms along x or z; the corners beside the feet must be empty (`BlockPatternBuilder`
    // `~` cells are air).
    let axes = [[1, 0], [0, 1]];
    let Some(axis) = axes.into_iter().find(|a| {
        let side = |s: i32, at: BlockPos| BlockPos::new(at.x + a[0] * s, at.y, at.z + a[1] * s);
        iron(level, side(1, body)) && iron(level, side(-1, body)) && air(level, side(1, feet)) && air(level, side(-1, feet))
    }) else {
        return;
    };
    let arms = [
        BlockPos::new(body.x + axis[0], body.y, body.z + axis[1]),
        BlockPos::new(body.x - axis[0], body.y, body.z - axis[1]),
    ];
    // `clearPatternBlocks`: every block of the pattern goes, with its break particles.
    for at in [pos, body, feet, arms[0], arms[1]] {
        let s = level.block(at);
        kiln_blocks::set_block(level, at, d::AIR, flags::CLIENTS);
        level.effect(Effect::LevelEvent { id: 2001, pos: at, data: s as i32 });
    }
    // `spawnGolemInWorld`: at the feet, facing the default way; player created.
    let at = [feet.x as f64 + 0.5, feet.y as f64 + 0.05, feet.z as f64 + 0.5];
    let mut golem = kiln_entity::mob::new(kiln_entity::mob::MobKind::IronGolem, 0, 0, p.entity_id as i64 ^ level.env.game_time);
    golem.set_pos(kiln_entity::math::Vec3::new(at[0], at[1], at[2]));
    golem.y_rot = 0.0;
    golem.set_old_pos_and_rot();
    if let Some(m) = kiln_entity::mob::data_mut(&mut golem)
        && let Some(s) = kiln_entity::mob::ext::state_mut::<kiln_entity::mob::kinds::iron_golem::State>(m)
    {
        s.player_created = true;
    }
    let seen = kiln_entity::level::Seen::of(&golem);
    spawns.push(Spawn { kind: &kiln_data::entities::types::IRON_GOLEM, pos: at, vel: [0.0; 3], body: Body::Ready(Box::new(golem)) });
    // `SummonedEntityTrigger` for players in the golem's box inflated by 5 (Kiln sees the
    // acting player).
    let near = (0..3).all(|k| (p.pos[k] - at[k]).abs() <= 5.0 + if k == 1 { 2.7 } else { 0.7 });
    if near {
        let dim = crate::DIMENSIONS[level.env.dim].0;
        let subject = crate::advancements::triggers::seen_subject(&seen, dim);
        p.fire_conds("minecraft:summoned_entity", None, |c, ok, _| c.cap("entity").is_none_or(|cap| ok(cap, &subject)));
    }
    // The golem's blocks updated their neighbours.
    for at in [feet, body, pos] {
        kiln_blocks::update::update_neighbors_at(level, at, kiln_blocks::BlockId::of(d::AIR));
    }
}
