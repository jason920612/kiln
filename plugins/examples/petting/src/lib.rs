//! Entity-scoped state: right-clicking a farm animal pets it. Each animal remembers how often
//! it was petted (entity data, saved in the entity's NBT under `kiln:plugin`, so it follows
//! the animal across regions and restarts); the player hears the count. The manifest's
//! `entities` filter keeps every other entity from reaching the plugin.

use kiln_plugin_sdk::registry::{self, Kind};
use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{EntityEvent, Plugin, Verdict, chat, colored, export_plugin, text};

struct Petting;

impl Plugin for Petting {
    fn on_entity_interact(ev: EntityEvent) -> Verdict {
        let n = state::get_i64(Scope::Entity(ev.entity), "pets") + 1;
        state::put_i64(Scope::Entity(ev.entity), "pets", n);
        let kind = registry::key(Kind::EntityType, ev.kind).unwrap_or_default();
        let name = kind.strip_prefix("minecraft:").unwrap_or(&kind).replace('_', " ");
        let times = if n == 1 { "once".to_owned() } else { format!("{n} times") };
        chat::send(ev.player.handle, &[text("You petted this "), colored(&name, "gold"), text(&format!(" {times}."))]);
        Verdict::Allow
    }
}

export_plugin!(Petting);
