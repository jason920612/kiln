//! Locked containers: whoever places a chest, trapped chest or barrel owns it, and nobody else
//! can open, use or break it unless they have the `lockbox.bypass` permission node (operators
//! always do). It shows the 1.1 additions:
//! - `world.read`: a `block-place` event names the clicked position but not the block; the
//!   handler asks `world::block` for it (the host copied the blocks around the event for the call);
//! - permission nodes (`perm`): `/lockbox grant <uuid>` and `/lockbox revoke <uuid>` (operators)
//!   change who has `lockbox.bypass`; the check works in every context from the global snapshot;
//! - `player-moved`: a per-player step counter (`steps:<uuid>` in the global namespace).
//!
//! A lock lives in the **cell** namespace of the chest (`lock:x:y:z` holds the owner), so the
//! decision is made from data the event's own region owns.

use kiln_plugin_sdk::registry::{self, Kind};
use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    BlockEvent, CommandSpec, InitInfo, Observed, PlaceEvent, Player, Plugin, Span, Text, Verdict, export_plugin, perm, uuid_from_u128, uuid_string,
    uuid_u128, world,
};
use std::sync::Mutex;

const BYPASS: &str = "lockbox.bypass";

struct Lockable {
    blocks: Vec<u32>,
    items: Vec<u32>,
}

static LOCKABLE: Mutex<Option<Lockable>> = Mutex::new(None);

fn lock_key(x: i32, y: i32, z: i32) -> String {
    format!("lock:{x}:{y}:{z}")
}

fn owner_of(cell: u64, x: i32, y: i32, z: i32) -> Option<u128> {
    state::get(Scope::Cell(cell), &lock_key(x, y, z)).and_then(|b| b.try_into().ok()).map(u128::from_le_bytes)
}

fn refusal(owner: u128) -> Verdict {
    let online = kiln_plugin_sdk::event::online();
    let name = online.into_iter().find(|p| uuid_u128(&p.uuid) == owner).map(|p| p.name).unwrap_or_else(|| "another player".to_owned());
    Verdict::deny(Text::new().color("red", "Locked by ").color("gold", name))
}

fn check(player: &Player, cell: u64, x: i32, y: i32, z: i32) -> Verdict {
    match owner_of(cell, x, y, z) {
        Some(owner) if owner != uuid_u128(&player.uuid) && !perm::has(player, BYPASS) => refusal(owner),
        _ => Verdict::Allow,
    }
}

fn say(color: &str, text: &str) -> Vec<Span> {
    Text::new().color(color, text).0
}

struct Lockbox;

impl Plugin for Lockbox {
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        vec![CommandSpec { name: "lockbox".to_owned(), permission: 4 }]
    }

    fn on_command(_p: Option<Player>, _name: String, args: String) -> Vec<Span> {
        let mut words = args.split_whitespace();
        let (verb, who) = (words.next().unwrap_or(""), words.next().and_then(|w| u128::from_str_radix(&w.replace('-', ""), 16).ok()));
        match (verb, who) {
            ("grant", Some(who)) => {
                perm::grant(&uuid_from_u128(who), BYPASS);
                say("green", "Granted.")
            }
            ("revoke", Some(who)) => {
                perm::revoke(&uuid_from_u128(who), BYPASS);
                say("green", "Revoked.")
            }
            _ => say("red", "Usage: /lockbox grant|revoke <uuid>"),
        }
    }

    fn init_region(_info: InitInfo) {
        let ids = |kind: Kind, keys: &[&str]| keys.iter().filter_map(|k| registry::id(kind, k)).collect::<Vec<u32>>();
        let keys = ["minecraft:chest", "minecraft:trapped_chest", "minecraft:barrel"];
        *LOCKABLE.lock().unwrap() = Some(Lockable { blocks: ids(Kind::Block, &keys), items: ids(Kind::Item, &keys) });
    }

    fn on_block_break(ev: BlockEvent) -> Verdict {
        let lockable = LOCKABLE.lock().unwrap();
        let Some(l) = lockable.as_ref() else { return Verdict::deny_silently() };
        if !l.blocks.contains(&ev.block) {
            return Verdict::Allow;
        }
        check(&ev.player, ev.cell, ev.pos.x, ev.pos.y, ev.pos.z)
    }

    fn on_block_place(ev: PlaceEvent) -> Verdict {
        let lockable = LOCKABLE.lock().unwrap();
        let Some(l) = lockable.as_ref() else { return Verdict::deny_silently() };
        // Using a locked container (what block is at `against`? only `world::block` knows).
        let clicked = world::block(ev.against.x, ev.against.y, ev.against.z);
        if clicked.is_some_and(|b| l.blocks.contains(&b)) {
            let v = check(&ev.player, ev.cell, ev.against.x, ev.against.y, ev.against.z);
            if v.is_deny() {
                return v;
            }
        }
        // Placing one: the placer owns it.
        if ev.item.is_some_and(|i| l.items.contains(&i)) {
            state::put(Scope::Cell(ev.cell), &lock_key(ev.pos.x, ev.pos.y, ev.pos.z), Some(&uuid_u128(&ev.player.uuid).to_le_bytes()));
        }
        Verdict::Allow
    }

    fn on_observe(events: Vec<Observed>) {
        for ev in events {
            if let Observed::PlayerMoved(m) = ev {
                state::add(&format!("steps:{}", uuid_string(&m.player.uuid)), 1);
            }
        }
    }
}

export_plugin!(Lockbox);
