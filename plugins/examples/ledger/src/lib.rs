//! Test plugin for the owned-namespace property test (design §11.3): a currency whose balances
//! are player-scoped and whose transfers between players in different contexts go through
//! escrow keys in the global namespace (atomic `add`). Everything minted is counted in the
//! global `minted`, so `sum(balances) + sum(escrow:*) == minted` must always hold.
//!
//! Chat (region instance): `pay <n> <uuid>`, `paytrap <n> <uuid>` (pays, then traps: nothing
//! may stick), `claim` (moves the escrow snapshot into the balance). Command `/sweep` (global
//! instance): moves the live escrow into the balance.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{ChatEvent, ChatVerdict, CommandSpec, InitInfo, Player, Plugin, Span, export_plugin, text, uuid_string};

fn escrow(uuid: &str) -> String {
    format!("escrow:{uuid}")
}

struct Ledger;

impl Plugin for Ledger {
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        vec![CommandSpec { name: "sweep".into(), permission: 0 }]
    }

    fn on_join(p: Player) {
        if state::get(Scope::Player(p.handle), "balance").is_none() {
            state::put_i64(Scope::Player(p.handle), "balance", 100);
            state::add("minted", 100);
        }
    }

    fn on_chat(ev: ChatEvent) -> ChatVerdict {
        let me = Scope::Player(ev.player.handle);
        let words: Vec<&str> = ev.message.split_whitespace().collect();
        match words.as_slice() {
            ["pay" | "paytrap", n, to] => {
                let n: i64 = n.parse().unwrap_or(0);
                let bal = state::get_i64(me, "balance");
                if n > 0 && bal >= n {
                    state::put_i64(Scope::Player(ev.player.handle), "balance", bal - n);
                    state::add(&escrow(to), n);
                }
                if words[0] == "paytrap" {
                    panic!("paytrap");
                }
            }
            ["claim"] => {
                let e = state::global_i64(&escrow(&uuid_string(&ev.player.uuid)));
                if e > 0 {
                    let bal = state::get_i64(me, "balance");
                    state::put_i64(Scope::Player(ev.player.handle), "balance", bal + e);
                    state::add(&escrow(&uuid_string(&ev.player.uuid)), -e);
                }
            }
            _ => return ChatVerdict::Pass,
        }
        ChatVerdict::Cancel
    }

    fn on_command(p: Option<Player>, _name: String, _args: String) -> Vec<Span> {
        let Some(p) = p else { return Vec::new() };
        let e = state::global_i64(&escrow(&uuid_string(&p.uuid)));
        if e > 0 {
            let bal = state::get_i64(Scope::Player(p.handle), "balance");
            state::put_i64(Scope::Player(p.handle), "balance", bal + e);
            state::add(&escrow(&uuid_string(&p.uuid)), -e);
        }
        vec![text(&format!("balance {}", state::get_i64(Scope::Player(p.handle), "balance")))]
    }
}

export_plugin!(Ledger);
