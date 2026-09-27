//! `effect give` and `effect clear` (vanilla `EffectCommands`).

use super::{LEVEL_GAMEMASTERS, source_entity};
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use crate::types::Identifier;

type Result<T> = std::result::Result<T, CommandError>;

const MOB_EFFECT: &str = "minecraft:mob_effect";
/// `MobEffect.isInstantaneous`: instant health, instant damage and saturation.
const INSTANTANEOUS: [&str; 3] = ["minecraft:instant_health", "minecraft:instant_damage", "minecraft:saturation"];

/// `MobEffect.getDisplayName`: `effect.<namespace>.<path>`.
fn display_name(effect: &Identifier) -> Text {
    tr!(format!("effect.{}.{}", effect.namespace(), effect.path()))
}

/// `computeDurationInTicks`: seconds times 20 (-1 is infinite; instantaneous effects take the
/// number as ticks), by default 30 seconds or one tick.
fn duration_ticks(seconds: Option<i32>, effect: &Identifier) -> i32 {
    let instant = INSTANTANEOUS.contains(&effect.as_str());
    match seconds {
        Some(s) if instant => s,
        Some(-1) => -1,
        Some(s) => s.wrapping_mul(20),
        None if instant => 1,
        None => 600,
    }
}

/// `CommandResponseTracker` over the living targets with `NON_ZERO` feedback: the count of
/// targets something happened to, and the only one when it is one.
struct Tracker<E> {
    changed: i32,
    only: Option<E>,
}

impl<E: Clone> Tracker<E> {
    fn new() -> Self {
        Tracker { changed: 0, only: None }
    }

    fn track(&mut self, e: &E, changed: Option<bool>) {
        if changed == Some(true) {
            self.changed += 1;
            self.only = (self.changed == 1).then(|| e.clone());
        }
    }
}

fn give<S: Host>(c: &CommandContext<S>, s: &mut S, seconds: Option<i32>, amplifier: i32, hide_particles: bool) -> Result<i32> {
    let targets = c.selector("targets").entities(s)?;
    let effect = c.identifier("effect").clone();
    let duration = duration_ticks(seconds, &effect);
    let mut t = Tracker::new();
    for e in &targets {
        let changed = s.add_effect(e, &effect, duration, amplifier, !hide_particles);
        t.track(e, changed);
    }
    let text = match (t.changed, &t.only) {
        (0, _) => return Err(CommandError::new(tr!("commands.effect.give.failed"))),
        (_, Some(one)) => tr!("commands.effect.give.success.single", display_name(&effect), one.display_name(), duration / 20),
        (n, None) => tr!("commands.effect.give.success.multiple", display_name(&effect), n, duration / 20),
    };
    s.send_success(text, true);
    Ok(t.changed)
}

fn clear_all<S: Host>(s: &mut S, targets: Vec<S::Entity>) -> Result<i32> {
    let mut t = Tracker::new();
    for e in &targets {
        let changed = s.clear_effects(e);
        t.track(e, changed);
    }
    let text = match (t.changed, &t.only) {
        (0, _) => return Err(CommandError::new(tr!("commands.effect.clear.everything.failed"))),
        (_, Some(one)) => tr!("commands.effect.clear.everything.success.single", one.display_name()),
        (n, None) => tr!("commands.effect.clear.everything.success.multiple", n),
    };
    s.send_success(text, true);
    Ok(t.changed)
}

fn clear_one<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let targets = c.selector("targets").entities(s)?;
    let effect = c.identifier("effect").clone();
    let mut t = Tracker::new();
    for e in &targets {
        let changed = s.remove_effect(e, &effect);
        t.track(e, changed);
    }
    let text = match (t.changed, &t.only) {
        (0, _) => return Err(CommandError::new(tr!("commands.effect.clear.specific.failed"))),
        (_, Some(one)) => tr!("commands.effect.clear.specific.success.single", display_name(&effect), one.display_name()),
        (n, None) => tr!("commands.effect.clear.specific.success.multiple", display_name(&effect), n),
    };
    s.send_success(text, true);
    Ok(t.changed)
}

pub fn effect<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let amplifier_then = |seconds: fn(&CommandContext<S>) -> Option<i32>| {
        argument("amplifier", ArgumentType::Integer { min: 0, max: 255 })
            .executes(move |c, s: &mut S| give(c, s, seconds(c), c.integer("amplifier"), false))
            .then(
                argument("hideParticles", ArgumentType::Bool)
                    .executes(move |c, s: &mut S| give(c, s, seconds(c), c.integer("amplifier"), c.bool("hideParticles"))),
            )
    };
    d.register(
        literal("effect")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("clear")
                    .executes(|_, s: &mut S| {
                        let me = source_entity(s)?;
                        clear_all(s, vec![me])
                    })
                    .then(
                        argument("targets", ArgumentType::entities())
                            .executes(|c, s: &mut S| {
                                let targets = c.selector("targets").entities(s)?;
                                clear_all(s, targets)
                            })
                            .then(argument("effect", ArgumentType::Resource { registry: MOB_EFFECT }).executes(clear_one)),
                    ),
            )
            .then(
                literal("give").then(
                    argument("targets", ArgumentType::entities()).then(
                        argument("effect", ArgumentType::Resource { registry: MOB_EFFECT })
                            .executes(|c, s: &mut S| give(c, s, None, 0, false))
                            .then(
                                argument("seconds", ArgumentType::Integer { min: 1, max: 1_000_000 })
                                    .executes(|c, s: &mut S| give(c, s, Some(c.integer("seconds")), 0, false))
                                    .then(amplifier_then(|c| Some(c.integer("seconds")))),
                            )
                            .then(
                                literal("infinite")
                                    .executes(|c, s: &mut S| give(c, s, Some(-1), 0, false))
                                    .then(amplifier_then(|_| Some(-1))),
                            ),
                    ),
                ),
            ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        let speed = Identifier::parse("minecraft:speed").unwrap();
        let heal = Identifier::parse("minecraft:instant_health").unwrap();
        assert_eq!(duration_ticks(None, &speed), 600);
        assert_eq!(duration_ticks(Some(5), &speed), 100);
        assert_eq!(duration_ticks(Some(-1), &speed), -1);
        assert_eq!(duration_ticks(None, &heal), 1);
        assert_eq!(duration_ticks(Some(5), &heal), 5);
    }
}
