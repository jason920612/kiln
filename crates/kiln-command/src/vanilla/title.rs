//! `/title` (`TitleCommand`): titles, subtitles, the action bar and title timing.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use bytes::Bytes;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::hud;

type Result<T> = std::result::Result<T, CommandError>;

pub fn title<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let show = |kind: &'static str, packet: fn(&Tag) -> Bytes| {
        literal(kind).then(argument("title", ArgumentType::Component).executes(move |c, s: &mut S| {
            show_title(c, s, kind, packet)
        }))
    };
    d.register(
        literal("title").requires(LEVEL_GAMEMASTERS).then(
            argument("targets", ArgumentType::players())
                .then(literal("clear").executes(|c, s: &mut S| send(c, s, &hud::clear_titles(false), "cleared")))
                .then(literal("reset").executes(|c, s: &mut S| send(c, s, &hud::clear_titles(true), "reset")))
                .then(show("title", hud::set_title_text))
                .then(show("subtitle", hud::set_subtitle_text))
                .then(show("actionbar", hud::set_action_bar_text))
                .then(literal("times").then(argument("fadeIn", ArgumentType::time()).then(
                    argument("stay", ArgumentType::time()).then(argument("fadeOut", ArgumentType::time()).executes(
                        |c, s: &mut S| {
                            let pkt = hud::set_titles_animation(c.time("fadeIn"), c.time("stay"), c.time("fadeOut"));
                            send(c, s, &pkt, "times")
                        },
                    )),
                ))),
        ),
    );
}

/// `sendPacketToPlayers` with `commands.title.<kind>.single|multiple` feedback.
fn send<S: Host>(c: &CommandContext<S>, s: &mut S, pkt: &Bytes, kind: &str) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    for t in &targets {
        s.send_packet(t, pkt.clone());
    }
    feedback(s, &targets, kind)
}

/// `CommandResponseTracker` over players, each counting 1.
fn feedback<S: Host>(s: &mut S, targets: &[S::Entity], kind: &str) -> Result<i32> {
    let n = targets.len() as i32;
    let text = match targets {
        [one] => Text::translate(format!("commands.title.{kind}.single"), vec![one.display_name().into()]),
        _ => Text::translate(format!("commands.title.{kind}.multiple"), vec![n.into()]),
    };
    s.send_success(text, true);
    Ok(n)
}

/// `showTitle`: the component resolved with each target as `@s`.
fn show_title<S: Host>(c: &CommandContext<S>, s: &mut S, kind: &str, packet: fn(&Tag) -> Bytes) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    let title = c.component("title");
    for t in &targets {
        let resolved = title.resolve(s, Some(t))?;
        s.send_packet(t, packet(&resolved.to_nbt()));
    }
    feedback(s, &targets, &format!("show.{kind}"))
}
