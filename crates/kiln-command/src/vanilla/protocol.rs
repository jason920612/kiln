//! Protocol features as commands: `transfer` (`TransferCommand`) and `dialog`
//! (`DialogCommand`). Both send one play packet per target through [`Host::send_packet`].

use super::{LEVEL_ADMINS, LEVEL_GAMEMASTERS, source_player};
use crate::arguments::{ArgumentType, ArgumentValue};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use bytes::Bytes;
use kiln_proto::packets::common::{self, Dialog, Phase};

type Result<T> = std::result::Result<T, CommandError>;

/// The port a `transfer <hostname>` without one uses.
const DEFAULT_PORT: i32 = 25565;

pub fn transfer<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("transfer").requires(LEVEL_ADMINS).then(
            argument("hostname", ArgumentType::string())
                .executes(|c, s: &mut S| {
                    let me = source_player(s)?;
                    run_transfer(s, c.string("hostname"), DEFAULT_PORT, vec![me])
                })
                .then(
                    argument("port", ArgumentType::integer_range(1, 65535))
                        .executes(|c, s: &mut S| {
                            let me = source_player(s)?;
                            run_transfer(s, c.string("hostname"), c.integer("port"), vec![me])
                        })
                        .then(argument("players", ArgumentType::players()).executes(|c, s: &mut S| {
                            let players = c.selector("players").players(s)?;
                            run_transfer(s, c.string("hostname"), c.integer("port"), players)
                        })),
                ),
        ),
    );
}

/// `TransferCommand.transfer`.
fn run_transfer<S: Host>(s: &mut S, host: &str, port: i32, players: Vec<S::Entity>) -> Result<i32> {
    if players.is_empty() {
        return Err(CommandError::new(tr!("commands.transfer.error.no_players")));
    }
    let pkt = common::transfer(Phase::Play, host, port);
    for p in &players {
        s.send_packet(p, pkt.clone());
    }
    let n = players.len() as i32;
    let text = match players.as_slice() {
        [one] => tr!("commands.transfer.success.single", one.display_name(), host, port),
        _ => tr!("commands.transfer.success.multiple", n, host, port),
    };
    s.send_success(text, true);
    Ok(n)
}

pub fn dialog<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("dialog")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("show").then(argument("targets", ArgumentType::players()).then(
                argument("dialog", ArgumentType::Dialog).executes(|c, s: &mut S| {
                    let pkt = dialog_packet(c);
                    send_all(c, s, pkt, "show")
                }),
            )))
            .then(literal("clear").then(argument("targets", ArgumentType::players()).executes(|c, s: &mut S| {
                send_all(c, s, common::clear_dialog(Phase::Play), "clear")
            }))),
    );
}

/// `ServerPlayer.openDialog`: a registry entry by id, or the inline definition.
fn dialog_packet<S: Host>(c: &CommandContext<S>) -> Bytes {
    match c.get("dialog") {
        Some(ArgumentValue::Identifier(id)) => {
            let index = kiln_data::synced_id("minecraft:dialog", id.as_str()).expect("checked when parsed");
            common::show_dialog_play(&Dialog::Registered(index))
        }
        Some(ArgumentValue::Nbt(tag)) => common::show_dialog_play(&Dialog::Inline(tag)),
        other => unreachable!("dialog argument {other:?}"),
    }
}

/// `showDialog` / `clearDialog`: the packet to every target, `commands.dialog.<kind>.*` feedback.
fn send_all<S: Host>(c: &CommandContext<S>, s: &mut S, pkt: Bytes, kind: &str) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    for t in &targets {
        s.send_packet(t, pkt.clone());
    }
    let n = targets.len() as i32;
    let text = match targets.as_slice() {
        [one] => Text::translate(format!("commands.dialog.{kind}.single"), vec![one.display_name().into()]),
        _ => Text::translate(format!("commands.dialog.{kind}.multiple"), vec![n.into()]),
    };
    s.send_success(text, true);
    Ok(n)
}
