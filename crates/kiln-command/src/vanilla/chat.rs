//! `say`, `msg` (`tell`, `w`) and `me`.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{Dispatcher, argument, literal};
use crate::host::{ChatKind, ChatMessage, Host};
use crate::error::CommandError;
use crate::selector::SelectorTarget;
use crate::text::ClickEvent;
use crate::tr;

pub fn say<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("say").requires(LEVEL_GAMEMASTERS).then(argument("message", ArgumentType::Message).executes(
        |c, s: &mut S| {
            let content = c.message("message").resolve(s)?;
            s.broadcast_chat(ChatMessage { kind: ChatKind::Say, sender: s.source_name(), target: None, content });
            Ok(1)
        },
    )));
}

pub fn msg<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let msg = d.register(literal("msg").then(argument("targets", ArgumentType::players()).then(
        argument("message", ArgumentType::Message).executes(|c, s: &mut S| {
            let targets = c.selector("targets").players(s)?;
            let content = c.message("message").resolve(s)?;
            let sender = s.source_name();
            for target in &targets {
                s.send_chat_to_source(ChatMessage {
                    kind: ChatKind::MsgOutgoing,
                    sender: sender.clone(),
                    target: Some(target.display_name()),
                    content: content.clone(),
                });
                s.send_chat(
                    target,
                    ChatMessage {
                        kind: ChatKind::MsgIncoming,
                        sender: sender.clone(),
                        target: None,
                        content: content.clone(),
                    },
                );
            }
            Ok(targets.len() as i32)
        }),
    )));
    d.register(literal("tell").redirect(msg));
    d.register(literal("w").redirect(msg));
}

/// `TellRawCommand`: the component, resolved with each recipient as `@s`.
pub fn tellraw<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("tellraw").requires(LEVEL_GAMEMASTERS).then(
        argument("targets", ArgumentType::players()).then(argument("message", ArgumentType::Component).executes(
            |c, s: &mut S| {
                let targets = c.selector("targets").players(s)?;
                let message = c.component("message");
                for target in &targets {
                    let resolved = message.resolve(s, Some(target))?;
                    s.send_system(target, resolved.to_text());
                }
                Ok(targets.len() as i32)
            },
        )),
    ));
}

/// `TeamMsgCommand`: the message to the source's team, through the team message chat types.
pub fn teammsg<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let node = d.register(literal("teammsg").then(argument("message", ArgumentType::Message).executes(
        |c, s: &mut S| {
            let me = super::source_entity(s)?;
            let holder = me.scoreboard_name();
            let team = s.scoreboard().and_then(|b| b.team_of(&holder)).cloned();
            let Some(team) = team else {
                return Err(CommandError::new(tr!("commands.teammsg.failed.noteam")));
            };
            let target = team
                .formatted_display_name()
                .hover(tr!("chat.type.team.hover"))
                .click(ClickEvent::SuggestCommand("/teammsg".into()));
            let board = s.scoreboard();
            let recipients: Vec<S::Entity> = s
                .players()
                .into_iter()
                .filter(|p| {
                    p.uuid() == me.uuid()
                        || board.and_then(|b| b.team_of(&p.scoreboard_name())).is_some_and(|t| t.name == team.name)
                })
                .collect();
            if !recipients.is_empty() {
                let content = c.message("message").resolve(s)?;
                let sender = s.source_name();
                for p in &recipients {
                    let kind =
                        if p.uuid() == me.uuid() { ChatKind::TeamMsgOutgoing } else { ChatKind::TeamMsgIncoming };
                    let message =
                        ChatMessage { kind, sender: sender.clone(), target: Some(target.clone()), content: content.clone() };
                    s.send_chat(p, message);
                }
            }
            Ok(recipients.len() as i32)
        },
    )));
    d.register(literal("tm").redirect(node));
}

pub fn me<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("me").then(argument("action", ArgumentType::Message).executes(|c, s: &mut S| {
        let content = c.message("action").resolve(s)?;
        s.broadcast_chat(ChatMessage { kind: ChatKind::Emote, sender: s.source_name(), target: None, content });
        Ok(1)
    })));
}
