//! `teleport`/`tp`, `gamemode`, `kill`, `give`, `kick` and `spawnpoint`.

use super::{LEVEL_ADMINS, LEVEL_GAMEMASTERS, format_double, source_entity, source_player};
use crate::arguments::ArgumentType;
use crate::coords::{Coordinates, WorldCoordinate, wrap_degrees};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, SpawnPoint, Teleport};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use crate::types::{Anchor, GameMode};

type Result<T> = std::result::Result<T, CommandError>;

pub fn teleport<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let to_location = |facing: fn(&CommandContext<S>) -> Option<Anchor>, rotation: bool| {
        move |c: &CommandContext<S>, s: &mut S| {
            let targets = c.selector("targets").entities(s)?;
            let rot = rotation.then(|| c.coordinates("rotation"));
            let look = match facing(c) {
                None if c.has("facingLocation") => Some(position(c.coordinates("facingLocation"), s)),
                None => None,
                Some(anchor) => {
                    let e = c.selector("facingEntity").entity(s)?;
                    Some(anchor_position(&e, anchor))
                }
            };
            teleport_to_pos(s, targets, c.coordinates("location"), rot, look)
        }
    };
    let teleport = d.register(
        literal("teleport")
            .requires(LEVEL_GAMEMASTERS)
            .then(argument("location", ArgumentType::vec3()).executes(|c, s: &mut S| {
                let me = source_entity(s)?;
                teleport_to_pos(s, vec![me], c.coordinates("location"), None, None)
            }))
            .then(argument("destination", ArgumentType::entity()).executes(|c, s: &mut S| {
                let me = source_entity(s)?;
                let dest = c.selector("destination").entity(s)?;
                teleport_to_entity(s, vec![me], &dest)
            }))
            .then(
                argument("targets", ArgumentType::entities())
                    .then(
                        argument("location", ArgumentType::vec3())
                            .executes(to_location(|_| None, false))
                            .then(argument("rotation", ArgumentType::Rotation).executes(to_location(|_| None, true)))
                            .then(
                                literal("facing")
                                    .then(
                                        literal("entity").then(
                                            argument("facingEntity", ArgumentType::entity())
                                                .executes(to_location(|_| Some(Anchor::Feet), false))
                                                .then(
                                                    argument("facingAnchor", ArgumentType::EntityAnchor).executes(
                                                        to_location(|c| Some(c.anchor("facingAnchor")), false),
                                                    ),
                                                ),
                                        ),
                                    )
                                    .then(
                                        argument("facingLocation", ArgumentType::vec3())
                                            .executes(to_location(|_| None, false)),
                                    ),
                            ),
                    )
                    .then(argument("destination", ArgumentType::entity()).executes(|c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        let dest = c.selector("destination").entity(s)?;
                        teleport_to_entity(s, targets, &dest)
                    })),
            ),
    );
    d.register(literal("tp").requires(LEVEL_GAMEMASTERS).redirect(teleport));
}

fn position<S: Host>(c: &Coordinates, s: &S) -> [f64; 3] {
    c.position(s.origin(), s.source_rotation())
}

fn anchor_position<E: SelectorTarget>(e: &E, anchor: Anchor) -> [f64; 3] {
    let [x, y, z] = e.position();
    match anchor {
        Anchor::Feet => [x, y, z],
        Anchor::Eyes => [x, y + e.eye_height(), z],
    }
}

fn check_bounds<S: Host>(s: &S, pos: [f64; 3]) -> Result<()> {
    if s.is_in_spawnable_bounds(pos.map(|v| v.floor() as i32)) {
        Ok(())
    } else {
        Err(CommandError::new(tr!("commands.teleport.invalidPosition")))
    }
}

fn teleport_to_entity<S: Host>(s: &mut S, targets: Vec<S::Entity>, dest: &S::Entity) -> Result<i32> {
    let to = Teleport {
        dimension: dest.dimension().to_owned(),
        pos: dest.position(),
        relative: [false; 3],
        rotation: Some(dest.rotation().map(wrap_degrees)),
        relative_rotation: [false; 2],
        facing: None,
    };
    for t in &targets {
        check_bounds(s, to.pos)?;
        s.teleport(t, &to)?;
    }
    let text = match targets.as_slice() {
        [one] => tr!("commands.teleport.success.entity.single", one.display_name(), dest.display_name()),
        _ => tr!("commands.teleport.success.entity.multiple", targets.len() as i32, dest.display_name()),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

fn teleport_to_pos<S: Host>(
    s: &mut S,
    targets: Vec<S::Entity>,
    location: &Coordinates,
    rotation: Option<&Coordinates>,
    facing: Option<[f64; 3]>,
) -> Result<i32> {
    let pos = position(location, s);
    let rot = rotation.map(|r| r.rotation(s.source_rotation()));
    let relative_rotation = match rotation {
        Some(Coordinates::World([pitch, yaw, _])) => [yaw.relative, pitch.relative],
        Some(Coordinates::Local { .. }) => [true; 2],
        None => [true; 2],
    };
    let dimension = s.dimension().to_owned();
    for t in &targets {
        let same_dimension = t.dimension() == dimension;
        let to = Teleport {
            dimension: dimension.clone(),
            pos,
            relative: if same_dimension { location.relative() } else { [false; 3] },
            rotation: rot.map(|r| r.map(wrap_degrees)),
            relative_rotation,
            facing,
        };
        check_bounds(s, pos)?;
        s.teleport(t, &to)?;
    }
    let [x, y, z] = pos.map(format_double);
    let text = match targets.as_slice() {
        [one] => tr!("commands.teleport.success.location.single", one.display_name(), x, y, z),
        _ => tr!("commands.teleport.success.location.multiple", targets.len() as i32, x, y, z),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

pub fn gamemode<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("gamemode").requires(LEVEL_GAMEMASTERS).then(
            argument("gamemode", ArgumentType::GameMode)
                .executes(|c, s: &mut S| {
                    let me = source_player(s)?;
                    set_mode(s, vec![me], c.game_mode("gamemode"))
                })
                .then(argument("target", ArgumentType::players()).executes(|c, s: &mut S| {
                    let targets = c.selector("target").players(s)?;
                    set_mode(s, targets, c.game_mode("gamemode"))
                })),
        ),
    );
}

fn set_mode<S: Host>(s: &mut S, players: Vec<S::Entity>, mode: GameMode) -> Result<i32> {
    let mut changed = 0;
    let source = s.source_entity().map(|e| e.uuid());
    for p in &players {
        if !s.set_game_mode(p, mode) {
            continue;
        }
        changed += 1;
        if source == Some(p.uuid()) {
            s.send_success(tr!("commands.gamemode.success.self", mode.display_name()), true);
        } else {
            if s.send_command_feedback() {
                s.send_system(p, tr!("gameMode.changed", mode.display_name()));
            }
            s.send_success(tr!("commands.gamemode.success.other", p.display_name(), mode.display_name()), true);
        }
    }
    Ok(changed)
}

pub fn kill<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("kill")
            .requires(LEVEL_GAMEMASTERS)
            .executes(|_, s: &mut S| {
                let me = source_entity(s)?;
                kill_all(s, vec![me])
            })
            .then(argument("targets", ArgumentType::entities()).executes(|c, s: &mut S| {
                let targets = c.selector("targets").entities(s)?;
                kill_all(s, targets)
            })),
    );
}

fn kill_all<S: Host>(s: &mut S, targets: Vec<S::Entity>) -> Result<i32> {
    for t in &targets {
        s.kill(t);
    }
    let text = match targets.as_slice() {
        [one] => tr!("commands.kill.success.single", one.display_name()),
        _ => tr!("commands.kill.success.multiple", targets.len() as i32),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

pub fn give<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let give = |c: &CommandContext<S>, s: &mut S, count: i32| -> Result<i32> {
        let item = c.item("item");
        let max = s.max_stack_size(item).saturating_mul(100);
        if count > max {
            return Err(CommandError::new(tr!("commands.give.failed.toomanyitems", max, item.display_name())));
        }
        let targets = c.selector("targets").players(s)?;
        for p in &targets {
            s.give(p, item, count);
        }
        let text = match targets.as_slice() {
            [one] => tr!("commands.give.success.single", count, item.display_name(), one.display_name()),
            _ => tr!("commands.give.success.multiple", count, item.display_name(), targets.len() as i32),
        };
        s.send_success(text, true);
        Ok(targets.len() as i32)
    };
    d.register(
        literal("give").requires(LEVEL_GAMEMASTERS).then(
            argument("targets", ArgumentType::players()).then(
                argument("item", ArgumentType::ItemStack).executes(move |c, s: &mut S| give(c, s, 1)).then(
                    argument("count", ArgumentType::integer_min(1))
                        .executes(move |c, s: &mut S| give(c, s, c.integer("count"))),
                ),
            ),
        ),
    );
}

pub fn kick<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let kick = |c: &CommandContext<S>, s: &mut S, reason: Text| -> Result<i32> {
        let targets = c.selector("targets").players(s)?;
        for p in &targets {
            s.kick(p, reason.clone());
            s.send_success(tr!("commands.kick.success", p.display_name(), reason.clone()), true);
        }
        Ok(targets.len() as i32)
    };
    d.register(
        literal("kick").requires(LEVEL_ADMINS).then(
            argument("targets", ArgumentType::players())
                .executes(move |c, s: &mut S| kick(c, s, tr!("multiplayer.disconnect.kicked")))
                .then(argument("reason", ArgumentType::Message).executes(move |c, s: &mut S| {
                    let reason = c.message("reason").resolve(s)?;
                    kick(c, s, reason)
                })),
        ),
    );
}

const ZERO_ROTATION: Coordinates = Coordinates::World([WorldCoordinate { relative: false, value: 0.0 }; 3]);

/// `BlockPosArgument.getSpawnablePos`, or the source's block when `name` is absent.
pub(super) fn spawn_pos<S: Host>(c: &CommandContext<S>, s: &S, name: &str) -> Result<[i32; 3]> {
    match c.get(name) {
        None => Ok(s.origin().map(|v| v.floor() as i32)),
        Some(_) => {
            let pos = c.coordinates(name).block_pos(s.origin(), s.source_rotation());
            if s.is_in_spawnable_bounds(pos) { Ok(pos) } else { Err(CommandError::pos_out_of_bounds()) }
        }
    }
}

/// `RespawnData.of`: yaw wrapped, pitch clamped.
pub(super) fn spawn_point<S: Host>(c: &CommandContext<S>, s: &S, pos: [i32; 3]) -> SpawnPoint {
    let rotation = if c.has("rotation") { c.coordinates("rotation") } else { &ZERO_ROTATION };
    let [yaw, pitch] = rotation.rotation(s.source_rotation());
    SpawnPoint { dimension: s.dimension().to_owned(), pos, yaw: wrap_degrees(yaw), pitch: pitch.clamp(-90.0, 90.0) }
}

pub fn spawnpoint<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let set = |c: &CommandContext<S>, s: &mut S| -> Result<i32> {
        let targets = if c.has("targets") { c.selector("targets").players(s)? } else { vec![source_player(s)?] };
        let pos = spawn_pos(c, s, "pos")?;
        let spawn = spawn_point(c, s, pos);
        for p in &targets {
            s.set_spawn_point(p, &spawn);
        }
        let [x, y, z] = spawn.pos;
        let dim = Text::literal(&spawn.dimension);
        let text = match targets.as_slice() {
            [one] => {
                tr!("commands.spawnpoint.success.single", x, y, z, spawn.yaw, spawn.pitch, dim, one.display_name())
            }
            _ => {
                tr!("commands.spawnpoint.success.multiple", x, y, z, spawn.yaw, spawn.pitch, dim, targets.len() as i32)
            }
        };
        s.send_success(text, true);
        Ok(targets.len() as i32)
    };
    d.register(
        literal("spawnpoint").requires(LEVEL_GAMEMASTERS).executes(set).then(
            argument("targets", ArgumentType::players()).executes(set).then(
                argument("pos", ArgumentType::BlockPos)
                    .executes(set)
                    .then(argument("rotation", ArgumentType::Rotation).executes(set)),
            ),
        ),
    );
}
