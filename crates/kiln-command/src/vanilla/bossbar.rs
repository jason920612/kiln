//! `/bossbar` (`BossBarCommands`) on the host's [`BossBars`].

use super::LEVEL_GAMEMASTERS;
use super::scoreboard::resolved;
use crate::arguments::ArgumentType;
use crate::bossbar::{BossBar, BossBars, COLOR_NAMES, OVERLAY_NAMES};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use crate::types::Identifier;

type Result<T> = std::result::Result<T, CommandError>;

pub fn bossbar<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let id = || argument("id", ArgumentType::ResourceLocation).suggests_server(suggest_bars::<S>);
    // Vanilla registers the colors and styles in this order.
    let colors = [0, 1, 2, 3, 4, 5, 6].into_iter().fold(literal("color"), |b, i| {
        b.then(literal(COLOR_NAMES[i]).executes(move |c, s: &mut S| set_color(c, s, i)))
    });
    let styles = [0, 1, 2, 3, 4].into_iter().fold(literal("style"), |b, i| {
        b.then(literal(OVERLAY_NAMES[i]).executes(move |c, s: &mut S| set_style(c, s, i)))
    });
    let set = id()
        .then(literal("name").then(argument("name", ArgumentType::Component).executes(|c, s: &mut S| {
            let name = resolved(c, s, "name")?;
            set_name(c, s, name)
        })))
        .then(colors)
        .then(styles)
        .then(literal("value").then(
            argument("value", ArgumentType::integer_min(0)).executes(|c, s: &mut S| set_value(c, s, c.integer("value"))),
        ))
        .then(
            literal("max").then(
                argument("max", ArgumentType::integer_min(1)).executes(|c, s: &mut S| set_max(c, s, c.integer("max"))),
            ),
        )
        .then(literal("visible").then(
            argument("visible", ArgumentType::Bool).executes(|c, s: &mut S| set_visible(c, s, c.bool("visible"))),
        ))
        .then(
            literal("players")
                .executes(|c, s: &mut S| set_players(c, s, Vec::new()))
                .then(argument("targets", ArgumentType::players()).executes(|c, s: &mut S| {
                    // `EntityArgument.getOptionalPlayers`: no match is no players.
                    let targets = c.selector("targets").find_players(s)?;
                    set_players(c, s, targets)
                })),
        );
    let get = id()
        .then(literal("value").executes(|c, s: &mut S| {
            let bar = bar(c, s)?;
            s.send_success(tr!("commands.bossbar.get.value", bar.display_name(), bar.value), true);
            Ok(bar.value)
        }))
        .then(literal("max").executes(|c, s: &mut S| {
            let bar = bar(c, s)?;
            s.send_success(tr!("commands.bossbar.get.max", bar.display_name(), bar.max), true);
            Ok(bar.max)
        }))
        .then(literal("visible").executes(|c, s: &mut S| {
            let bar = bar(c, s)?;
            let key = if bar.visible { "commands.bossbar.get.visible.visible" } else { "commands.bossbar.get.visible.hidden" };
            s.send_success(Text::translate(key, vec![bar.display_name().into()]), true);
            Ok(bar.visible as i32)
        }))
        .then(literal("players").executes(|c, s: &mut S| {
            let bar = bar(c, s)?;
            let names = player_names(s, &bar);
            let text = if names.is_empty() {
                tr!("commands.bossbar.get.players.none", bar.display_name())
            } else {
                tr!("commands.bossbar.get.players.some", bar.display_name(), names.len() as i32, Text::join(names.clone()))
            };
            s.send_success(text, true);
            Ok(names.len() as i32)
        }));
    d.register(
        literal("bossbar")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("add").then(
                    argument("id", ArgumentType::ResourceLocation)
                        .then(argument("name", ArgumentType::Component).executes(|c, s: &mut S| create_bar(c, s))),
                ),
            )
            .then(literal("remove").then(id().executes(|c, s: &mut S| remove_bar(c, s))))
            .then(literal("list").executes(|_, s: &mut S| list_bars(s)))
            .then(literal("set").then(set))
            .then(literal("get").then(get)),
    );
}

fn suggest_bars<S: Host>(_: &CommandContext<S>, s: &S, b: &mut crate::suggestion::SuggestionsBuilder) {
    let ids: Vec<String> = s.bossbars().map(|bars| bars.bars().iter().map(|b| b.id.to_string()).collect()).unwrap_or_default();
    b.suggest_resources(ids.iter().map(String::as_str), "");
}

fn bars<S: Host>(s: &mut S) -> Result<&mut BossBars> {
    s.bossbars_mut().ok_or_else(|| CommandError::unsupported("Boss bars"))
}

/// `getBossBar`: the bar named by the `id` argument.
fn bar<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<BossBar> {
    let id = c.identifier("id");
    bars(s)?.get(id).cloned().ok_or_else(|| CommandError::new(tr!("commands.bossbar.unknown", id.to_string())))
}

/// Display names of the online players who see `bar`.
fn player_names<S: Host>(s: &S, bar: &BossBar) -> Vec<Text> {
    let players = s.players();
    bar.online_players()
        .iter()
        .filter_map(|u| players.iter().find(|p| p.uuid() == *u).map(SelectorTarget::display_name))
        .collect()
}

fn create_bar<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let id = c.identifier("id").clone();
    if bars(s)?.get(&id).is_some() {
        return Err(CommandError::new(tr!("commands.bossbar.create.failed", id.to_string())));
    }
    let name = resolved(c, s, "name")?;
    let bars = bars(s)?;
    bars.create(&id, name);
    let display = bars.get(&id).expect("just created").display_name();
    let n = bars.bars().len() as i32;
    s.send_success(tr!("commands.bossbar.create.success", display), true);
    Ok(n)
}

fn remove_bar<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let bar = bar(c, s)?;
    let bars = bars(s)?;
    bars.remove(&bar.id);
    let n = bars.bars().len() as i32;
    s.send_success(tr!("commands.bossbar.remove.success", bar.display_name()), true);
    Ok(n)
}

fn list_bars<S: Host>(s: &mut S) -> Result<i32> {
    let names: Vec<Text> = bars(s)?.bars().into_iter().map(BossBar::display_name).collect();
    let n = names.len() as i32;
    let text = if names.is_empty() {
        tr!("commands.bossbar.list.bars.none")
    } else {
        tr!("commands.bossbar.list.bars.some", n, Text::join(names))
    };
    s.send_success(text, false);
    Ok(n)
}

/// Applies a change to the bar and returns it as changed.
fn change<S: Host>(s: &mut S, id: &Identifier, f: impl FnOnce(&mut BossBars)) -> Result<BossBar> {
    let bars = bars(s)?;
    f(bars);
    Ok(bars.get(id).cloned().expect("bar exists"))
}

fn unchanged(key: &str) -> CommandError {
    CommandError::new(Text::translate(key, vec![]))
}

fn set_name<S: Host>(c: &CommandContext<S>, s: &mut S, name: Text) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.name.to_nbt() == name.to_nbt() {
        return Err(unchanged("commands.bossbar.set.name.unchanged"));
    }
    let bar = change(s, &bar.id, |b| b.set_name(&bar.id, name))?;
    s.send_success(tr!("commands.bossbar.set.name.success", bar.display_name()), true);
    Ok(0)
}

fn set_color<S: Host>(c: &CommandContext<S>, s: &mut S, color: usize) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.color == color {
        return Err(unchanged("commands.bossbar.set.color.unchanged"));
    }
    let bar = change(s, &bar.id, |b| b.set_color(&bar.id, color))?;
    s.send_success(tr!("commands.bossbar.set.color.success", bar.display_name()), true);
    Ok(0)
}

fn set_style<S: Host>(c: &CommandContext<S>, s: &mut S, overlay: usize) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.overlay == overlay {
        return Err(unchanged("commands.bossbar.set.style.unchanged"));
    }
    let bar = change(s, &bar.id, |b| b.set_overlay(&bar.id, overlay))?;
    s.send_success(tr!("commands.bossbar.set.style.success", bar.display_name()), true);
    Ok(0)
}

fn set_value<S: Host>(c: &CommandContext<S>, s: &mut S, value: i32) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.value == value {
        return Err(unchanged("commands.bossbar.set.value.unchanged"));
    }
    let bar = change(s, &bar.id, |b| b.set_value(&bar.id, value))?;
    s.send_success(tr!("commands.bossbar.set.value.success", bar.display_name(), value), true);
    Ok(value)
}

fn set_max<S: Host>(c: &CommandContext<S>, s: &mut S, max: i32) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.max == max {
        return Err(unchanged("commands.bossbar.set.max.unchanged"));
    }
    let bar = change(s, &bar.id, |b| b.set_max(&bar.id, max))?;
    s.send_success(tr!("commands.bossbar.set.max.success", bar.display_name(), max), true);
    Ok(max)
}

fn set_visible<S: Host>(c: &CommandContext<S>, s: &mut S, visible: bool) -> Result<i32> {
    let bar = bar(c, s)?;
    if bar.visible == visible {
        let key = if visible {
            "commands.bossbar.set.visibility.unchanged.visible"
        } else {
            "commands.bossbar.set.visibility.unchanged.hidden"
        };
        return Err(unchanged(key));
    }
    let bar = change(s, &bar.id, |b| b.set_visible(&bar.id, visible))?;
    let key = if visible { "commands.bossbar.set.visible.success.visible" } else { "commands.bossbar.set.visible.success.hidden" };
    s.send_success(Text::translate(key, vec![bar.display_name().into()]), true);
    Ok(0)
}

fn set_players<S: Host>(c: &CommandContext<S>, s: &mut S, targets: Vec<S::Entity>) -> Result<i32> {
    let bar = bar(c, s)?;
    let uuids: Vec<uuid::Uuid> = targets.iter().map(SelectorTarget::uuid).collect();
    if !bars(s)?.set_players(&bar.id, &uuids) {
        return Err(unchanged("commands.bossbar.set.players.unchanged"));
    }
    let bar = bars(s)?.get(&bar.id).cloned().expect("bar exists");
    let names = player_names(s, &bar);
    let text = if names.is_empty() {
        tr!("commands.bossbar.set.players.success.none", bar.display_name())
    } else {
        tr!("commands.bossbar.set.players.success.some", bar.display_name(), names.len() as i32, Text::join(names.clone()))
    };
    s.send_success(text, true);
    Ok(names.len() as i32)
}
