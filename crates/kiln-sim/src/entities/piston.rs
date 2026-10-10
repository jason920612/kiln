//! Moving pistons pushing what is in their way: `PistonMovingBlockEntity.tick`'s
//! `moveCollidedEntities` and `moveStuckEntities`, for the region's entities (through
//! kiln-entity's [`kiln_entity::piston`]) and its players (their server bodies, see
//! [`crate::phantom`]). The block entity ticks after the entities, so this runs once they have.

use super::*;
use kiln_entity::piston::MovingPistonView;

/// A moving piston that advanced this tick, with the progress it advanced to.
pub(crate) struct Push {
    pub pos: kiln_blocks::BlockPos,
    pub piston: kiln_blocks::MovingPiston,
    pub progress: f32,
}

pub(crate) fn view_of(m: &kiln_blocks::MovingPiston) -> MovingPistonView {
    MovingPistonView { moved: m.moved, direction: kiln_entity::math::Direction::ALL[m.direction.index()], extending: m.extending, source: m.source, progress: m.progress }
}

/// Pushes the entities and players for every piston of `pushes`, in order.
pub(crate) fn push(entities: &mut Entities, level: &mut RegionLevel, players: &mut [&mut Player], spawns: &mut Vec<Spawn>, deaths: &mut Vec<health::Death>, pushes: &[Push]) {
    let live = |p: &Player| !p.disconnected && !p.dead;
    let proxies: Vec<Proxy> = players.iter().filter(|p| live(p) && p.game_mode != 3).map(|p| Proxy::of(p)).collect();
    let views: Vec<PlayerView> = players.iter().filter(|p| live(p)).map(|p| view(p, level.env.game_time)).collect();
    let rng = entity_level_random(level.env.seed, level.env.game_time ^ 0x7069, 0);
    let (game_time, min_y, fast_lava) = (level.env.game_time, level.env.min_y, level.env.rules.fast_lava);
    let mut sim = SimLevel {
        level: World::Region(level),
        list: &mut entities.list,
        players,
        deaths,
        proxies,
        views,
        spawns,
        events: Vec::new(),
        next_placeholder: -1_000_000,
        current: 0,
        seeds: 0x7069 << 8,
        current_source: None,
        rng,
        grid: Grid::default(),
        proxy_at: Default::default(),
        proxy_grid: Default::default(),
        view_index: Default::default(),
        despawn: None,
        player_writes: 0,
        touched: None,
        current_info: None,
    };
    sim.grid = Grid::build(sim.list);
    sim.index_players();
    for push in pushes {
        let view = view_of(&push.piston);
        let pos = BlockPos::new(push.pos.x, push.pos.y, push.pos.z);
        let (collided, stuck) = kiln_entity::piston::move_entities(&mut sim, pos, &view, push.progress);
        for p in sim.players.iter_mut().filter(|p| live(p) && p.game_mode != 3) {
            let bb = p.bounding_box();
            let collides = collided.as_ref().is_some_and(|plan| bb.intersects(&plan.query));
            let sticks = stuck.as_ref().is_some_and(|plan| bb.intersects(&plan.area));
            if !collides && !sticks {
                continue;
            }
            let cells = sim.level.cells();
            let pistons = sim.level.pistons();
            p.piston_push(cells, pistons, game_time, min_y, fast_lava, |body, level| {
                if let Some(plan) = collided.as_ref().filter(|_| collides) {
                    kiln_entity::piston::push_entity(plan, body, level);
                }
                if let Some(plan) = stuck.as_ref().filter(|_| sticks)
                    && plan.matches(body)
                {
                    kiln_entity::piston::move_entity_by_piston(body, level, plan.dir, plan.movement, plan.dir);
                }
            });
        }
    }
    for e in sim.list.iter_mut() {
        e.sync();
    }
    let SimLevel { level, list, events, spawns, players, deaths, .. } = sim;
    let level = level.into_region();
    for (n, event) in keyed(events) {
        carry_out(event, n, level, list, players, spawns, deaths);
    }
}
