//! A shop with a custom menu. `/shop` opens a locked chest menu; clicking an item buys it
//! with the plugin's own money (`/balance`; everyone starts with 100).
//!
//! The pattern for spending money that other regions may spend at the same moment: the click
//! (in the player's region) submits a typed atomic `try-add` of `-price` with a floor of 0,
//! remembers what the ticket was for in the player's own namespace, and denies nothing yet.
//! The host applies all `try-add`s of a tick in one deterministic order and answers a tick
//! later in `on-results` (in the region holding the player), where the item is handed out if
//! the money was there. Two sales that race for the last coins cannot both succeed.
//!
//! The wand is a custom item: its `wand` tag marks it as this plugin's, and `item-use` (right
//! click, in the air or on a block) reaches only the plugin whose tag the item carries.

use kiln_plugin_sdk::state::{self, Scope};
use kiln_plugin_sdk::{
    CommandSpec, ContainerClickEvent, GlobalValue, InitInfo, Item, ItemUseEvent, Menu, OpResult, Player, Plugin, Span, Text, Verdict, chat, event,
    export_plugin, inventory, uuid_string,
};

/// (menu slot, item key, label, price, tag)
const GOODS: [(u8, &str, &str, i64, Option<&str>); 5] = [
    (10, "minecraft:bread", "Bread x8", 8, None),
    (12, "minecraft:iron_sword", "Iron sword", 40, None),
    (14, "minecraft:diamond", "Diamond", 100, None),
    (16, "minecraft:stick", "Magic wand", 25, Some("wand")),
    (22, "minecraft:golden_apple", "Golden apple", 60, None),
];

fn balance_key(uuid: &kiln_plugin_sdk::Uuid) -> String {
    format!("bal:{}", uuid_string(uuid))
}

fn item_for(i: usize) -> Item {
    let (_, key, label, _, tag) = GOODS[i];
    let count = if key == "minecraft:bread" { 8 } else { 1 };
    let mut item = Item::new(key, count).name(Text::new().color("gold", label));
    if let Some(t) = tag {
        item = item.tag(t).glint().lore_line(Text::new().color("gray", "Right-click to zap."));
    }
    item
}

fn menu() -> Menu {
    let mut m = Menu::new("main", 3).title(Text::new().color("dark_green", "Shop"));
    for (i, (slot, key, label, price, _)) in GOODS.iter().enumerate() {
        let count = if *key == "minecraft:bread" { 8 } else { 1 };
        let shown = Item::new(key, count)
            .name(Text::new().color("gold", *label))
            .lore_line(Text::new().color("yellow", format!("Price: {price}")))
            .lore_line(Text::new().color("gray", "Click to buy"));
        let _ = i;
        m = m.item(*slot, shown);
    }
    m
}

struct Shop;

impl Plugin for Shop {
    fn init_global(_info: InitInfo) -> Vec<CommandSpec> {
        ["shop", "balance"].iter().map(|n| CommandSpec { name: (*n).to_owned(), permission: 0 }).collect()
    }

    /// First visit: starting money.
    fn on_join(p: Player) {
        let me = Scope::Player(p.handle);
        if state::get_int(me, "welcomed").is_none() {
            state::put_int(me, "welcomed", 1);
            state::add(&balance_key(&p.uuid), 100);
        }
    }

    fn on_command(p: Option<Player>, name: String, _args: String) -> Vec<Span> {
        let Some(p) = p else { return Text::new().color("red", "Players only.").0 };
        if name == "balance" {
            return Text::new().color("gold", format!("Balance: {}", state::global_i64(&balance_key(&p.uuid)))).0;
        }
        inventory::open_menu(p.uuid, menu());
        Vec::new()
    }

    fn on_container_click(ev: ContainerClickEvent) -> Verdict {
        // Only the plugin's own menu (the manifest has no `vanilla`).
        let Some(slot) = u8::try_from(ev.slot).ok() else { return Verdict::Allow };
        let Some(i) = GOODS.iter().position(|g| g.0 == slot) else { return Verdict::Allow };
        let (_, _, _, price, _) = GOODS[i];
        let ticket = state::try_add(&balance_key(&ev.player.uuid), -price, 0);
        state::put_int(Scope::Player(ev.player.handle), &format!("buying:{ticket}"), i as i64);
        Verdict::Allow
    }

    fn on_results(player: Option<Player>, results: Vec<OpResult>) {
        let Some(p) = player else { return };
        let me = Scope::Player(p.handle);
        for r in results {
            let key = format!("buying:{}", r.ticket);
            let Some(i) = state::get_int(me, &key) else { continue };
            state::delete(me, &key);
            if r.applied {
                inventory::give(p.uuid, item_for(i as usize));
                let left = match r.value {
                    Some(GlobalValue::Int(v)) => v,
                    _ => 0,
                };
                chat::send(&p, Text::new().color("green", format!("Bought {}. Balance: {left}", GOODS[i as usize].2)));
            } else {
                chat::send(&p, Text::new().color("red", "Not enough money."));
            }
        }
    }

    /// The wand: a zap instead of whatever the stick would do.
    fn on_item_use(ev: ItemUseEvent) -> Verdict {
        if ev.item.tag.as_deref() == Some("wand") {
            let name = event::player_name(ev.player.handle);
            chat::send(&ev.player, Text::new().color("light_purple", format!("Zap! ({name})")));
            return Verdict::deny_silently();
        }
        Verdict::Allow
    }
}

export_plugin!(Shop);
