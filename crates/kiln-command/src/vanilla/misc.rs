//! `stopwatch` (vanilla `StopwatchCommand`: named real-time stopwatches kept with the world)
//! and `posteffect` (`PostEffectCommand`: client post-processing shaders per player).

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, SuggestionProvider, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `Double.toString`: the shortest decimal that reads back, `1.0E7` style outside 1e-3..1e7.
pub fn java_double(v: f64) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    if !v.is_finite() {
        return if v.is_nan() { "NaN".into() } else if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let a = v.abs();
    if (1e-3..1e7).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        let s = format!("{v:e}");
        let (mantissa, exp) = s.split_once('e').expect("exponent");
        let mantissa = if mantissa.contains('.') { mantissa.to_owned() } else { format!("{mantissa}.0") };
        format!("{mantissa}E{exp}")
    }
}

pub fn stopwatch<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let existing = || {
        argument("id", ArgumentType::ResourceLocation).suggests_server(|_, s: &S, b| {
            let ids = s.stopwatch_ids();
            b.suggest_resources(ids.iter().map(String::as_str), "");
        })
    };
    let missing = |id: &str| CommandError::new(tr!("commands.stopwatch.does_not_exist", id));
    d.register(
        literal("stopwatch")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("create").then(argument("id", ArgumentType::ResourceLocation).executes(|c, s: &mut S| {
                let id = c.identifier("id").to_string();
                if !s.stopwatch_create(&id) {
                    return Err(CommandError::new(tr!("commands.stopwatch.already_exists", id.as_str())));
                }
                s.send_success(tr!("commands.stopwatch.create.success", id.as_str()), true);
                Ok(1)
            })))
            .then(literal("query").then({
                fn query<S: Host>(c: &CommandContext<S>, s: &mut S, scale: f64) -> Result<i32> {
                    let id = c.identifier("id").to_string();
                    let Some(seconds) = s.stopwatch_seconds(&id) else {
                        return Err(CommandError::new(tr!("commands.stopwatch.does_not_exist", id.as_str())));
                    };
                    s.send_success(tr!("commands.stopwatch.query", id.as_str(), java_double(seconds)), true);
                    Ok((seconds * scale) as i32)
                }
                existing()
                    .executes(|c, s: &mut S| query(c, s, 1.0))
                    .then(argument("scale", ArgumentType::double()).executes(|c, s: &mut S| query(c, s, c.double("scale"))))
            }))
            .then(literal("restart").then(existing().executes(move |c, s: &mut S| {
                let id = c.identifier("id").to_string();
                if !s.stopwatch_restart(&id) {
                    return Err(missing(&id));
                }
                s.send_success(tr!("commands.stopwatch.restart.success", id.as_str()), true);
                Ok(1)
            })))
            .then(literal("remove").then(existing().executes(move |c, s: &mut S| {
                let id = c.identifier("id").to_string();
                if !s.stopwatch_remove(&id) {
                    return Err(missing(&id));
                }
                s.send_success(tr!("commands.stopwatch.remove.success", id.as_str()), true);
                Ok(1)
            }))),
    );
}

/// `CommandResponseTracker.sendFeedback` over players, with the post effect id as argument.
fn feedback<S: Host, E: SelectorTarget>(s: &mut S, changed: &[E], id: Option<&str>, key: &str) -> Result<i32> {
    let n = changed.len() as i32;
    let text = match (changed, id) {
        ([], _) => return Err(CommandError::new(tr!(format!("commands.posteffect.{key}.failed")))),
        ([one], Some(id)) => tr!(format!("commands.posteffect.{key}.success.single"), id, one.display_name()),
        ([one], None) => tr!(format!("commands.posteffect.{key}.success.single"), one.display_name()),
        (_, Some(id)) => tr!(format!("commands.posteffect.{key}.success.multiple"), id, n),
        (_, None) => tr!(format!("commands.posteffect.{key}.success.multiple"), n),
    };
    s.send_success(text, true);
    Ok(n)
}

pub fn posteffect<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let effect = || argument("posteffect", ArgumentType::ResourceLocation).suggests(SuggestionProvider::Named("minecraft:post_effects"));
    d.register(
        literal("posteffect")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("add").then(argument("targets", ArgumentType::players()).then(effect().executes(|c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                let id = c.identifier("posteffect").to_string();
                let changed: Vec<_> = targets.into_iter().filter(|p| s.add_post_effect(p, &id)).collect();
                feedback(s, &changed, Some(&id), "add")
            }))))
            .then(literal("clear").then(argument("targets", ArgumentType::players()).executes(|c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                let changed: Vec<_> = targets.into_iter().filter(|p| s.clear_post_effects(p)).collect();
                feedback(s, &changed, None, "clear")
            })))
            .then(literal("list").then(argument("target", ArgumentType::player()).executes(|c, s: &mut S| {
                let target = c.selector("target").players(s)?.into_iter().next().expect("a single player");
                let effects = s.post_effects(&target);
                if effects.is_empty() {
                    s.send_success(tr!("commands.posteffect.list.empty", target.display_name()), false);
                } else {
                    let list = Text::literal(effects.join(", "));
                    s.send_success(tr!("commands.posteffect.list.success", target.display_name(), effects.len() as i32, list), false);
                }
                Ok(effects.len() as i32)
            })))
            .then(literal("remove").then(argument("targets", ArgumentType::players()).then(effect().executes(|c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                let id = c.identifier("posteffect").to_string();
                let changed: Vec<_> = targets.into_iter().filter(|p| s.remove_post_effect(p, &id)).collect();
                feedback(s, &changed, Some(&id), "remove")
            })))),
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn doubles_as_java_prints_them() {
        use super::java_double as j;
        assert_eq!(j(12.345), "12.345");
        assert_eq!(j(3.0), "3.0");
        assert_eq!(j(0.0005), "5.0E-4");
        assert_eq!(j(12_345_678.9), "1.23456789E7");
        assert_eq!(j(0.1 + 0.2), "0.30000000000000004");
    }
}
