//! `compute` (vanilla `ComputeCommand`): the value of a context int or float provider, in the
//! loot context of the source (`LootContextSources`).

use super::blocks::loaded_block_pos;
use super::world::java_float;
use super::LEVEL_GAMEMASTERS;
use crate::arguments::{ArgumentType, ArgumentValue};
use crate::dispatcher::{argument, literal, Builder, CommandContext, Dispatcher};
use crate::error::CommandError;
use crate::host::{ComputeError, ComputeTarget, Host, LootTableArg};
use crate::text::Text;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// The provider argument: a registry id or an inline definition.
pub(super) fn provider_arg<S: Host>(c: &CommandContext<S>) -> LootTableArg {
    match c.get("provider") {
        Some(ArgumentValue::Nbt(t)) => LootTableArg::Inline(t.clone()),
        Some(ArgumentValue::Identifier(id)) => LootTableArg::Id(id.to_string()),
        other => unreachable!("provider argument {other:?}"),
    }
}

/// `LootContextSources` for a branch: `default`, `block <computePos>` or `entity <computeTarget>`.
#[derive(Clone, Copy)]
pub(super) enum Branch {
    Default,
    Block,
    Entity,
}

impl Branch {
    pub(super) fn target<S: Host>(self, c: &CommandContext<S>, s: &mut S) -> Result<ComputeTarget<S::Entity>> {
        Ok(match self {
            Branch::Default => ComputeTarget::Default,
            Branch::Block => {
                let dimension = s.dimension().to_owned();
                ComputeTarget::Block(loaded_block_pos(c, s, "computePos", &dimension)?)
            }
            Branch::Entity => ComputeTarget::Entity(c.selector("computeTarget").entity(s)?),
        })
    }
}

/// The provider's value, with vanilla's failures (`throwInvalidValue`).
pub(super) fn evaluate<S: Host>(s: &mut S, arg: &LootTableArg, float: bool, target: &ComputeTarget<S::Entity>) -> Result<f64> {
    match s.compute_provider(arg, float, target) {
        Ok(v) => Ok(v),
        Err(ComputeError::Command(e)) => Err(e),
        Err(ComputeError::Invalid(message)) => Err(invalid(arg, &message)),
    }
}

/// `Component.translationArg(id)`: a translation of the id's language key that falls back to
/// the id.
fn key_text(id: &str) -> Text {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    Text::raw(kiln_proto::nbt::Tag::Compound(vec![
        ("translate".into(), kiln_proto::nbt::Tag::String(format!("{ns}.{path}"))),
        ("fallback".into(), kiln_proto::nbt::Tag::String(id.to_owned())),
    ]))
}

fn invalid(arg: &LootTableArg, message: &str) -> CommandError {
    match arg {
        LootTableArg::Id(id) => CommandError::new(tr!("command.compute.result.named.invalid", key_text(id), message)),
        LootTableArg::Inline(_) => CommandError::new(tr!("command.compute.result.unnamed.invalid", message)),
    }
}

fn exact<S: Host>(s: &mut S, arg: &LootTableArg, value: i32) {
    let text = match arg {
        LootTableArg::Id(id) => tr!("command.compute.result.named.exact", key_text(id), value),
        LootTableArg::Inline(_) => tr!("command.compute.result.unnamed.exact", value),
    };
    s.send_success(text, false);
}

fn rounded<S: Host>(s: &mut S, arg: &LootTableArg, value: i32, exact_value: f32) {
    let shown = Text::literal(java_float(exact_value));
    let text = match arg {
        LootTableArg::Id(id) => tr!("command.compute.result.named.rounded", key_text(id), shown, value),
        LootTableArg::Inline(_) => tr!("command.compute.result.unnamed.rounded", shown, value),
    };
    s.send_success(text, false);
}

fn compute_int<S: Host>(c: &CommandContext<S>, s: &mut S, branch: Branch) -> Result<i32> {
    let target = branch.target(c, s)?;
    let arg = provider_arg(c);
    let v = evaluate(s, &arg, false, &target)? as i32;
    exact(s, &arg, v);
    Ok(v)
}

fn compute_float<S: Host>(c: &CommandContext<S>, s: &mut S, branch: Branch, scale: f32) -> Result<i32> {
    let target = branch.target(c, s)?;
    let arg = provider_arg(c);
    let v = evaluate(s, &arg, true, &target)? as f32;
    if !v.is_finite() {
        return Err(invalid(&arg, &java_float(v)));
    }
    let scaled = (v * scale).floor() as i32;
    if scaled as f32 == v {
        exact(s, &arg, scaled);
    } else {
        rounded(s, &arg, scaled, v);
    }
    Ok(scaled)
}

fn numbers<S: Host + 'static>(branch: Branch) -> [Builder<S>; 2] {
    [
        literal("float").then(
            argument("provider", ArgumentType::ContextProvider { float: true })
                .executes(move |c, s: &mut S| compute_float(c, s, branch, 1.0))
                .then(argument("scale", ArgumentType::float()).executes(move |c, s: &mut S| compute_float(c, s, branch, c.float("scale")))),
        ),
        literal("integer").then(argument("provider", ArgumentType::ContextProvider { float: false }).executes(move |c, s: &mut S| compute_int(c, s, branch))),
    ]
}

pub fn compute<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let with = |b: Builder<S>, branch: Branch| {
        let [f, i] = numbers(branch);
        b.then(f).then(i)
    };
    d.register(
        literal("compute")
            .requires(LEVEL_GAMEMASTERS)
            .then(with(literal("default"), Branch::Default))
            .then(literal("block").then(with(argument("computePos", ArgumentType::BlockPos), Branch::Block)))
            .then(literal("entity").then(with(argument("computeTarget", ArgumentType::entity()), Branch::Entity))),
    );
}
