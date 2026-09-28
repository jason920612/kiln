//! `attribute` (vanilla `AttributeCommand`): read and change entity attributes.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{AttributeState, Host};
use crate::selector::SelectorTarget;
use crate::text::{Arg, Text};
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `getAttributeDescription`: the attribute's translated name.
fn describe(attribute: &str) -> Text {
    let path = attribute.split_once(':').map_or(attribute, |(_, p)| p);
    Text::translate(format!("attribute.name.{path}"), Vec::new())
}

/// `getAttributeInstance` / `getEntityWithAttribute`.
fn instance<S: Host>(s: &mut S, e: &S::Entity, attribute: &str) -> Result<AttributeState> {
    match s.attribute(e, attribute) {
        Err(()) => Err(CommandError::new(tr!("commands.attribute.failed.entity", e.display_name()))),
        Ok(None) => Err(CommandError::new(tr!("commands.attribute.failed.no_attribute", e.display_name(), describe(attribute)))),
        Ok(Some(state)) => Ok(state),
    }
}

fn target<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<(S::Entity, String)> {
    Ok((c.selector("target").entity(s)?, c.identifier("attribute").to_string()))
}

fn scale<S: Host>(c: &CommandContext<S>) -> f64 {
    if c.has("scale") { c.double("scale") } else { 1.0 }
}

fn get_value<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let (e, a) = target(c, s)?;
    let v = instance(s, &e, &a)?.value;
    s.send_success(tr!("commands.attribute.value.get.success", describe(&a), e.display_name(), Arg::Double(v)), false);
    Ok((v * scale(c)) as i32)
}

fn get_base<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let (e, a) = target(c, s)?;
    let v = instance(s, &e, &a)?.base;
    s.send_success(tr!("commands.attribute.base_value.get.success", describe(&a), e.display_name(), Arg::Double(v)), false);
    Ok((v * scale(c)) as i32)
}

fn modifier_value<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let (e, a) = target(c, s)?;
    let id = c.identifier("id").to_string();
    let state = instance(s, &e, &a)?;
    let Some((_, v)) = state.modifiers.iter().find(|(m, _)| *m == id) else {
        return Err(CommandError::new(tr!("commands.attribute.failed.no_modifier", describe(&a), e.display_name(), id.as_str())));
    };
    let v = *v;
    s.send_success(tr!("commands.attribute.modifier.value.get.success", id.as_str(), describe(&a), e.display_name(), Arg::Double(v)), false);
    Ok((v * scale(c)) as i32)
}

pub fn attribute<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let add = |name: &'static str, op: u8| {
        literal(name).executes(move |c, s: &mut S| {
            let (e, a) = target(c, s)?;
            let id = c.identifier("id").to_string();
            let state = instance(s, &e, &a)?;
            if state.modifiers.iter().any(|(m, _)| *m == id) {
                return Err(CommandError::new(tr!("commands.attribute.failed.modifier_already_present", id.as_str(), describe(&a), e.display_name())));
            }
            s.add_attribute_modifier(&e, &a, &id, c.double("value"), op);
            s.send_success(tr!("commands.attribute.modifier.add.success", id.as_str(), describe(&a), e.display_name()), false);
            Ok(1)
        })
    };
    d.register(
        literal("attribute").requires(LEVEL_GAMEMASTERS).then(
            argument("target", ArgumentType::entity()).then(
                argument("attribute", ArgumentType::resource("minecraft:attribute"))
                    .then(
                        literal("get")
                            .executes(get_value)
                            .then(argument("scale", ArgumentType::double()).executes(get_value)),
                    )
                    .then(
                        literal("base")
                            .then(literal("set").then(argument("value", ArgumentType::double()).executes(|c, s: &mut S| {
                                let (e, a) = target(c, s)?;
                                instance(s, &e, &a)?;
                                let v = c.double("value");
                                s.set_attribute_base(&e, &a, v);
                                s.send_success(
                                    tr!("commands.attribute.base_value.set.success", describe(&a), e.display_name(), Arg::Double(v)),
                                    false,
                                );
                                Ok(1)
                            })))
                            .then(
                                literal("get")
                                    .executes(get_base)
                                    .then(argument("scale", ArgumentType::double()).executes(get_base)),
                            )
                            .then(literal("reset").executes(|c, s: &mut S| {
                                let (e, a) = target(c, s)?;
                                instance(s, &e, &a)?;
                                s.reset_attribute_base(&e, &a);
                                let v = instance(s, &e, &a)?.base;
                                s.send_success(
                                    tr!("commands.attribute.base_value.reset.success", describe(&a), e.display_name(), Arg::Double(v)),
                                    false,
                                );
                                Ok(1)
                            })),
                    )
                    .then(
                        literal("modifier")
                            .then(literal("add").then(argument("id", ArgumentType::ResourceLocation).then(
                                argument("value", ArgumentType::double())
                                    .then(add("add_value", 0))
                                    .then(add("add_multiplied_base", 1))
                                    .then(add("add_multiplied_total", 2)),
                            )))
                            .then(literal("remove").then(argument("id", ArgumentType::ResourceLocation).executes(|c, s: &mut S| {
                                let (e, a) = target(c, s)?;
                                let id = c.identifier("id").to_string();
                                instance(s, &e, &a)?;
                                if !s.remove_attribute_modifier(&e, &a, &id) {
                                    return Err(CommandError::new(tr!(
                                        "commands.attribute.failed.no_modifier",
                                        describe(&a),
                                        e.display_name(),
                                        id.as_str()
                                    )));
                                }
                                s.send_success(tr!("commands.attribute.modifier.remove.success", id.as_str(), describe(&a), e.display_name()), false);
                                Ok(1)
                            })))
                            .then(literal("value").then(literal("get").then(
                                argument("id", ArgumentType::ResourceLocation)
                                    .executes(modifier_value)
                                    .then(argument("scale", ArgumentType::double()).executes(modifier_value)),
                            ))),
                    ),
            ),
        ),
    );
}
