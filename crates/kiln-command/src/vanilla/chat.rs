//! `say`, `msg` (`tell`, `w`) and `me`.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{Dispatcher, argument, literal};
use crate::host::{ChatKind, ChatMessage, Host};
use crate::selector::SelectorTarget;

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

pub fn me<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("me").then(argument("action", ArgumentType::Message).executes(|c, s: &mut S| {
        let content = c.message("action").resolve(s)?;
        s.broadcast_chat(ChatMessage { kind: ChatKind::Emote, sender: s.source_name(), target: None, content });
        Ok(1)
    })));
}
