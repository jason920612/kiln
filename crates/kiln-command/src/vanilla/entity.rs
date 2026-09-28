//! `rotate`, `spectate`, `swing`, `ride` and `damage` (vanilla `RotateCommand`,
//! `SpectateCommand`, `SwingCommand`, `RideCommand`, `DamageCommand`).

use super::tracker::{Counted, Tracker};
use super::{LEVEL_GAMEMASTERS, source_entity, source_player};
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, look_at};
use crate::selector::SelectorTarget;
use crate::text::Arg;
use crate::tr;
use crate::types::Anchor;

type Result<T> = std::result::Result<T, CommandError>;

pub fn rotate<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn done<S: Host>(s: &mut S, target: &S::Entity, rot: [f32; 2]) -> Result<i32> {
        s.rotate_entity(target, rot);
        s.send_success(tr!("commands.rotate.success", target.display_name()), true);
        Ok(1)
    }
    /// `LookAt.LookAtEntity` / `LookAtPosition`: `Entity.lookAt(source anchor, target)`, from
    /// the target's feet or (anchored eyes) its eyes.
    fn face<S: Host>(s: &mut S, target: &S::Entity, to: [f64; 3]) -> Result<i32> {
        let from = if s.stack().anchor == Anchor::Eyes {
            let [x, y, z] = target.position();
            [x, y + target.eye_height(), z]
        } else {
            target.position()
        };
        done(s, target, look_at(from, to))
    }
    let facing_entity = |c: &CommandContext<S>, s: &mut S, anchor: Anchor| {
        let target = c.selector("target").entity(s)?;
        let other = c.selector("facingEntity").entity(s)?;
        let [x, y, z] = other.position();
        let to = if anchor == Anchor::Eyes { [x, y + other.eye_height(), z] } else { [x, y, z] };
        face(s, &target, to)
    };
    d.register(
        literal("rotate").requires(LEVEL_GAMEMASTERS).then(
            argument("target", ArgumentType::entity())
                .then(argument("rotation", ArgumentType::Rotation).executes(|c, s: &mut S| {
                    let target = c.selector("target").entity(s)?;
                    let [yaw, pitch] = c.coordinates("rotation").rotation(s.source_rotation());
                    done(s, &target, [yaw, pitch])
                }))
                .then(
                    literal("facing")
                        .then(
                            literal("entity").then(
                                argument("facingEntity", ArgumentType::entity())
                                    .executes(move |c, s: &mut S| facing_entity(c, s, Anchor::Feet))
                                    .then(
                                        argument("facingAnchor", ArgumentType::EntityAnchor)
                                            .executes(move |c, s: &mut S| facing_entity(c, s, c.anchor("facingAnchor"))),
                                    ),
                            ),
                        )
                        .then(argument("facingLocation", ArgumentType::vec3()).executes(|c, s: &mut S| {
                            let target = c.selector("target").entity(s)?;
                            let to = s.stack().resolve(c.coordinates("facingLocation"));
                            face(s, &target, to)
                        })),
                ),
        ),
    );
}

pub fn spectate<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(s: &mut S, target: Option<S::Entity>, player: S::Entity) -> Result<i32> {
        if target.as_ref().is_some_and(|t| t.uuid() == player.uuid()) {
            return Err(CommandError::new(tr!("commands.spectate.self")));
        }
        if player.game_mode() != Some(crate::types::GameMode::Spectator) {
            return Err(CommandError::new(tr!("commands.spectate.not_spectator", player.display_name())));
        }
        if let Some(t) = &target
            && !s.can_spectate(t)
        {
            return Err(CommandError::new(tr!("commands.spectate.cannot_spectate", t.display_name())));
        }
        s.set_camera(&player, target.as_ref());
        match target {
            Some(t) => s.send_success(tr!("commands.spectate.success.started", t.display_name()), false),
            None => s.send_success(tr!("commands.spectate.success.stopped"), false),
        }
        Ok(1)
    }
    d.register(
        literal("spectate")
            .requires(LEVEL_GAMEMASTERS)
            .executes(|_, s: &mut S| {
                let player = source_player(s)?;
                run(s, None, player)
            })
            .then(
                argument("target", ArgumentType::entity())
                    .executes(|c, s: &mut S| {
                        let target = c.selector("target").entity(s)?;
                        let player = source_player(s)?;
                        run(s, Some(target), player)
                    })
                    .then(argument("player", ArgumentType::player()).executes(|c, s: &mut S| {
                        let target = c.selector("target").entity(s)?;
                        let player = c.selector("player").player(s)?;
                        run(s, Some(target), player)
                    })),
            ),
    );
}

/// `SwingAnimation.DEFAULT`.
const DEFAULT_SWING: (&str, i32) = ("whack", 6);

pub fn swing<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(s: &mut S, targets: Vec<S::Entity>, offhand: bool, animation: &str, duration: i32) -> Result<i32> {
        let mut t = Tracker::new();
        for e in &targets {
            if s.swing_arm(e, offhand, animation, duration) {
                t.track(e, 1);
            }
        }
        t.send(
            s,
            true,
            Counted::NonZero,
            Some(CommandError::new(tr!("commands.swing.failed.notliving"))),
            |e, _| tr!("commands.swing.success.single", e.display_name()),
            |count, _| tr!("commands.swing.success.multiple", count),
        )
    }
    let hand = |name: &'static str, offhand: bool| {
        literal(name)
            .executes(move |c, s: &mut S| {
                let targets = c.selector("targets").entities(s)?;
                run(s, targets, offhand, DEFAULT_SWING.0, DEFAULT_SWING.1)
            })
            .then(
                argument("animation", ArgumentType::SwingAnimation)
                    .executes(move |c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        run(s, targets, offhand, c.string("animation"), DEFAULT_SWING.1)
                    })
                    .then(argument("duration", ArgumentType::time_min(1)).executes(move |c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        run(s, targets, offhand, c.string("animation"), c.time("duration"))
                    })),
            )
    };
    d.register(
        literal("swing")
            .requires(LEVEL_GAMEMASTERS)
            .executes(|_, s: &mut S| {
                let e = source_entity(s)?;
                run(s, vec![e], false, DEFAULT_SWING.0, DEFAULT_SWING.1)
            })
            .then(
                argument("targets", ArgumentType::entities())
                    .executes(|c, s: &mut S| {
                        let targets = c.selector("targets").entities(s)?;
                        run(s, targets, false, DEFAULT_SWING.0, DEFAULT_SWING.1)
                    })
                    .then(hand("mainhand", false))
                    .then(hand("offhand", true)),
            ),
    );
}

pub fn ride<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("ride").requires(LEVEL_GAMEMASTERS).then(
            argument("target", ArgumentType::entity())
                .then(literal("mount").then(argument("vehicle", ArgumentType::entity()).executes(|c, s: &mut S| {
                    let target = c.selector("target").entity(s)?;
                    let vehicle = c.selector("vehicle").entity(s)?;
                    if let Some(current) = s.vehicle_of(&target) {
                        return Err(CommandError::new(tr!("commands.ride.already_riding", target.display_name(), current.display_name())));
                    }
                    if vehicle.is_player() {
                        return Err(CommandError::new(tr!("commands.ride.mount.failure.cant_ride_players")));
                    }
                    if s.self_and_passengers(&target).iter().any(|e| e.uuid() == vehicle.uuid()) {
                        return Err(CommandError::new(tr!("commands.ride.mount.failure.loop")));
                    }
                    if target.dimension() != vehicle.dimension() {
                        return Err(CommandError::new(tr!("commands.ride.mount.failure.wrong_dimension")));
                    }
                    if !s.start_riding(&target, &vehicle) {
                        return Err(CommandError::new(tr!("commands.ride.mount.failure.generic", target.display_name(), vehicle.display_name())));
                    }
                    s.send_success(tr!("commands.ride.mount.success", target.display_name(), vehicle.display_name()), true);
                    Ok(1)
                })))
                .then(literal("dismount").executes(|c, s: &mut S| {
                    let target = c.selector("target").entity(s)?;
                    let Some(vehicle) = s.vehicle_of(&target) else {
                        return Err(CommandError::new(tr!("commands.ride.not_riding", target.display_name())));
                    };
                    s.stop_riding(&target);
                    s.send_success(tr!("commands.ride.dismount.success", target.display_name(), vehicle.display_name()), true);
                    Ok(1)
                })),
        ),
    );
}

pub fn damage<S: Host + 'static>(d: &mut Dispatcher<S>) {
    fn run<S: Host>(c: &CommandContext<S>, s: &mut S, at: bool, by: bool, from: bool) -> Result<i32> {
        let target = c.selector("target").entity(s)?;
        let amount = c.float("amount");
        let damage_type = if c.has("damageType") { c.identifier("damageType").to_string() } else { "minecraft:generic".to_owned() };
        let location = if at { Some(s.stack().resolve(c.coordinates("location"))) } else { None };
        let by = if by { Some(c.selector("entity").entity(s)?) } else { None };
        let from = if from { Some(c.selector("cause").entity(s)?) } else { None };
        if !s.damage_entity(&target, amount, &damage_type, location, by.as_ref(), from.as_ref()) {
            return Err(CommandError::new(tr!("commands.damage.invulnerable")));
        }
        s.send_success(tr!("commands.damage.success", Arg::Float(amount), target.display_name()), true);
        Ok(1)
    }
    d.register(
        literal("damage").requires(LEVEL_GAMEMASTERS).then(
            argument("target", ArgumentType::entity()).then(
                argument("amount", ArgumentType::float_range(0.0, f32::MAX))
                    .executes(|c, s: &mut S| run(c, s, false, false, false))
                    .then(
                        argument("damageType", ArgumentType::resource("minecraft:damage_type"))
                            .executes(|c, s: &mut S| run(c, s, false, false, false))
                            .then(literal("at").then(
                                argument("location", ArgumentType::vec3()).executes(|c, s: &mut S| run(c, s, true, false, false)),
                            ))
                            .then(literal("by").then(
                                argument("entity", ArgumentType::entity())
                                    .executes(|c, s: &mut S| run(c, s, false, true, false))
                                    .then(literal("from").then(
                                        argument("cause", ArgumentType::entity()).executes(|c, s: &mut S| run(c, s, false, true, true)),
                                    )),
                            )),
                    ),
            ),
        ),
    );
}
