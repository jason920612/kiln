//! NPCs: `/npc spawn <name>` puts a named armor stand (no AI, invulnerable, silent) where the
//! player stands; right-clicking it greets the player and counts the clicks in the entity's own
//! data (saved with the entity, so it follows it across regions and restarts);
//! `/npc remove` takes the player's last one away. A plugin can only remove entities it
//! spawned itself.

use kiln_plugin_sdk::entities::{self, Spawn};
use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{CommandSpec, EntityEvent, Player, Plugin, Span, Text, Verdict, chat, event, export_plugin, uuid_from_u128, uuid_u128};

struct Npc;

impl Plugin for Npc {
    fn init_global(_info: kiln_plugin_sdk::InitInfo) -> Vec<CommandSpec> {
        vec![CommandSpec { name: "npc".into(), permission: 2 }]
    }

    fn on_command(p: Option<Player>, _name: String, args: String) -> Vec<Span> {
        let Some(p) = p else { return Text::new().color("red", "Players only.").0 };
        let mine = Scope::Player(p.handle);
        let info = event::info(p.handle);
        let mut words = args.split_whitespace();
        match words.next() {
            Some("spawn") => {
                let name = words.collect::<Vec<_>>().join(" ");
                let name = if name.is_empty() { "Guide".to_owned() } else { name };
                let id = entities::spawn(Spawn::new("minecraft:armor_stand", info.level, info.pos).name(Text::new().color("gold", &name)).yaw(info.rot.0).npc());
                state::put(mine, "last", Some(&uuid_u128(&id).to_le_bytes()));
                state::put_int(mine, "level", info.level as i64);
                Text::new().color("green", format!("{name} is here.")).0
            }
            Some("remove") => {
                let last = state::get(mine, "last").and_then(|b| <[u8; 16]>::try_from(b.as_slice()).ok()).map(u128::from_le_bytes);
                match last {
                    Some(u) => {
                        entities::remove(state::get_i64(mine, "level") as u32, uuid_from_u128(u));
                        state::delete(mine, "last");
                        Text::new().color("yellow", "Removed.").0
                    }
                    None => Text::new().color("red", "You have no NPC.").0,
                }
            }
            _ => Text::new().color("gray", "/npc spawn <name> | remove").0,
        }
    }

    fn on_entity_interact(ev: EntityEvent) -> Verdict {
        let owned = Scope::Entity(ev.entity);
        let clicks = state::bump(owned, "clicks", 1);
        chat::send(&ev.player, Text::new().color("aqua", format!("Hello, {}! ({clicks})", event::player_name(ev.player.handle))));
        // The armor stand does not take the item in the hand.
        Verdict::deny_silently()
    }
}

export_plugin!(Npc);
