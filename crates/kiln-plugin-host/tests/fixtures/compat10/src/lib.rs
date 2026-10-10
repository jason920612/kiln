//! The smallest plugin that exercises the 1.0 boundary: a deny rule on `block-break` (above
//! y = 200), a counter in the global namespace bumped from `on-observe`, a chat rewrite, a
//! command, and imports from several interfaces (event, state, chat, log, env).

wit_bindgen::generate!({
    world: "plugin",
    path: "wit",
    generate_all,
});

use exports::kiln::api::global_hooks;
use exports::kiln::api::region_hooks;
use kiln::api::types::{
    AtomicOp, BlockEvent, ChatEvent, ChatVerdict, CommandEvent, CommandSpec, CustomEvent, Decision, DamageEvent, EntityEvent, GlobalValue,
    InitInfo, ItemUseEvent, ContainerClickEvent, Observed, OpResult, PlaceEvent, Player, Span, TaskEvent, CancelledTask,
};

struct Compat;

fn span(text: &str) -> Span {
    Span { text: text.to_string(), color: None, bold: false, italic: false }
}

impl global_hooks::Guest for Compat {
    fn init(info: InitInfo) -> Vec<CommandSpec> {
        kiln::api::log::info(&format!("compat10 init: {} levels, generation {}", info.levels.len(), info.generation));
        vec![CommandSpec { name: "compat10".to_string(), permission: 0 }]
    }
    fn on_enable(_blob: Option<Vec<u8>>) {}
    fn on_disable() -> Option<Vec<u8>> {
        None
    }
    fn on_join(_p: Player) {}
    fn on_leave(_p: Player) {}
    fn on_command(_p: Option<Player>, name: String, args: String) -> Vec<Span> {
        vec![span(&format!("{name}:{args}:{}", kiln::api::env::tick()))]
    }
    fn on_task(_t: TaskEvent) {}
    fn on_results(_p: Option<Player>, _results: Vec<OpResult>) {}
    fn on_cancelled(_tasks: Vec<CancelledTask>) {}
    fn on_custom(_ev: CustomEvent) -> Decision {
        Decision::Allow
    }
}

impl region_hooks::Guest for Compat {
    fn init(_info: InitInfo) {}
    fn on_block_break(ev: BlockEvent) -> Decision {
        if ev.pos.y > 200 {
            kiln::api::event::deny_message(&[span("compat10: too high")]);
            Decision::Deny
        } else {
            Decision::Allow
        }
    }
    fn on_block_place(_ev: PlaceEvent) -> Decision {
        Decision::Allow
    }
    fn on_entity_interact(_ev: EntityEvent) -> Decision {
        Decision::Allow
    }
    fn on_entity_attack(_ev: EntityEvent) -> Decision {
        Decision::Allow
    }
    fn on_player_damage(_ev: DamageEvent) -> Decision {
        Decision::Allow
    }
    fn on_item_use(_ev: ItemUseEvent) -> Decision {
        Decision::Allow
    }
    fn on_container_click(_ev: ContainerClickEvent) -> Decision {
        Decision::Allow
    }
    fn on_chat(ev: ChatEvent) -> ChatVerdict {
        ChatVerdict::Rewrite(vec![span("[1.0] "), span(&ev.message)])
    }
    fn on_command(_ev: CommandEvent) -> Decision {
        Decision::Allow
    }
    fn on_observe(events: Vec<Observed>) {
        // One count per event, in the global namespace.
        let mut broken = 0i64;
        for e in &events {
            if let Observed::BlockBroken(_) = e {
                broken += 1;
            }
        }
        if broken > 0 {
            kiln::api::state::submit(&AtomicOp::Add(("compat10.broken".to_string(), broken)));
        }
        let _ = GlobalValue::Int(0);
    }
    fn on_task(_t: TaskEvent) {}
    fn on_results(_p: Option<Player>, _results: Vec<OpResult>) {}
    fn on_custom(_ev: CustomEvent) -> Decision {
        Decision::Allow
    }
}

export!(Compat);
