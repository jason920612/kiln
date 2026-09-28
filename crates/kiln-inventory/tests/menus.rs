//! Hand-made menu scenarios with hand-written recipes (no vanilla data needed). The expected
//! behaviour is vanilla's, as checked at scale by the parity tests.

use kiln_inventory::click::{ContainerClick, handle_container_button_click, handle_container_click, handle_set_creative_slot};
use kiln_inventory::recipe::RecipeManager;
use kiln_inventory::{Container, ContainerInput, Effect, Env, Menu, NoWorld, PlayerFlags, PlayerInventory, Rules};
use kiln_item::{HashedStack, ItemStack};

fn stack(name: &str, n: i32) -> ItemStack {
    ItemStack::of(name, n).unwrap()
}

fn hash(s: &ItemStack) -> HashedStack {
    HashedStack::of(s).unwrap()
}

fn rules() -> Rules {
    Rules::with_recipes(RecipeManager::from_json_entries([
        (
            "test:stick",
            r##"{"type":"minecraft:crafting_shaped","pattern":["#","#"],"key":{"#":"minecraft:oak_planks"},"result":{"id":"minecraft:stick","count":4}}"##,
        ),
        ("test:slab", r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stone_slab","count":2}}"#),
        ("test:bricks", r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stone_bricks"}}"#),
        (
            "test:sword",
            r#"{"type":"minecraft:smithing_transform","template":"minecraft:netherite_upgrade_smithing_template","base":"minecraft:diamond_sword","addition":"minecraft:netherite_ingot","result":{"id":"minecraft:netherite_sword"}}"#,
        ),
    ]))
}

/// A player and what the menus send them.
struct Player {
    inv: PlayerInventory,
    flags: PlayerFlags,
    world: NoWorld,
    out: Vec<Effect>,
}

impl Player {
    fn new() -> Self {
        Player { inv: PlayerInventory::new(), flags: PlayerFlags::default(), world: NoWorld, out: Vec::new() }
    }

    fn env<'a>(&'a mut self, rules: &'a Rules) -> Env<'a> {
        Env { inventory: &mut self.inv, block: None, player: self.flags, rules, world: &mut self.world, out: &mut self.out }
    }

    /// The packets sent since the last call.
    fn packets(&mut self) -> Vec<Effect> {
        self.out.drain(..).filter(Effect::is_packet).collect()
    }
}

fn click(menu: &Menu, slot: i16, button: i8, input: ContainerInput, changed: Vec<(i16, HashedStack)>, carried: HashedStack) -> ContainerClick {
    ContainerClick { container_id: menu.container_id, state_id: menu.state_id(), slot, button, input, changed, carried }
}

#[test]
fn opening_sends_the_full_content() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("stone", 10));
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    match p.packets().as_slice() {
        [Effect::SetContent { container_id: 0, state_id: 1, items, carried }] => {
            assert_eq!(items.len(), 46);
            assert_eq!(items[36], stack("stone", 10));
            assert!(carried.is_empty());
        }
        other => panic!("unexpected packets {other:?}"),
    }
}

#[test]
fn a_predicted_click_sends_nothing() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("stone", 10));
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    p.packets();
    let c = click(&menu, 36, 0, ContainerInput::Pickup, vec![(36, HashedStack::Empty)], hash(&stack("stone", 10)));
    handle_container_click(&mut menu, &mut p.env(&rules), &c, true).unwrap();
    assert!(p.packets().is_empty());
    let c = click(&menu, 9, 1, ContainerInput::Pickup, vec![(9, hash(&stack("stone", 1)))], hash(&stack("stone", 9)));
    handle_container_click(&mut menu, &mut p.env(&rules), &c, true).unwrap();
    assert!(p.packets().is_empty());
    assert_eq!(p.inv.item(9), &stack("stone", 1));
    assert_eq!(menu.carried(), &stack("stone", 9));
}

#[test]
fn a_wrong_prediction_is_corrected() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("stone", 10));
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    p.packets();
    let c = click(&menu, 36, 0, ContainerInput::Pickup, vec![], HashedStack::Empty);
    handle_container_click(&mut menu, &mut p.env(&rules), &c, true).unwrap();
    match p.packets().as_slice() {
        [Effect::SetSlot { container_id: 0, state_id: 2, slot: 36, stack: s }, Effect::SetCursor { stack: cursor }] => {
            assert!(s.is_empty());
            assert_eq!(cursor, &stack("stone", 10));
        }
        other => panic!("unexpected packets {other:?}"),
    }
}

#[test]
fn a_stale_state_id_resends_everything() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    p.packets();
    let mut c = click(&menu, 20, 0, ContainerInput::Pickup, vec![], HashedStack::Empty);
    c.state_id = 0;
    handle_container_click(&mut menu, &mut p.env(&rules), &c, true).unwrap();
    assert!(matches!(p.packets().as_slice(), [Effect::SetContent { state_id: 2, .. }]));
}

#[test]
fn shift_click_puts_armor_on() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("iron_helmet", 1));
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    menu.clicked(&mut p.env(&rules), 36, 0, ContainerInput::QuickMove).unwrap();
    assert!(p.inv.item(0).is_empty());
    assert_eq!(p.inv.item(39), &stack("iron_helmet", 1));
}

#[test]
fn clicking_outside_drops_the_carried_stack() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("stone", 10));
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    menu.clicked(&mut p.env(&rules), 36, 0, ContainerInput::Pickup).unwrap();
    p.out.clear();
    menu.clicked(&mut p.env(&rules), -999, 1, ContainerInput::Pickup).unwrap();
    menu.clicked(&mut p.env(&rules), -999, 0, ContainerInput::Pickup).unwrap();
    let drops: Vec<_> = p.out.iter().filter_map(|e| if let Effect::Drop { stack, .. } = e { Some(stack.count()) } else { None }).collect();
    assert_eq!(drops, [1, 9]);
    assert!(menu.carried().is_empty());
}

#[test]
fn crafting_table_shift_click_crafts_everything() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::crafting(1);
    menu.open(&mut p.env(&rules));
    menu.set_slot(&mut p.env(&rules), 1, stack("oak_planks", 2));
    menu.set_slot(&mut p.env(&rules), 4, stack("oak_planks", 2));
    assert_eq!(menu.item(&p.env(&rules), 0), &stack("stick", 4));
    menu.clicked(&mut p.env(&rules), 0, 0, ContainerInput::QuickMove).unwrap();
    assert_eq!(p.inv.item(8), &stack("stick", 8));
    assert!(menu.craft_grid().iter().all(ItemStack::is_empty));
    assert!(menu.item(&p.env(&rules), 0).is_empty());
}

#[test]
fn stonecutter_selects_a_recipe_and_takes_its_result() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::stonecutter(2);
    menu.open(&mut p.env(&rules));
    menu.set_slot(&mut p.env(&rules), 0, stack("stone", 3));
    assert_eq!(menu.stonecutter_recipes(), [1, 2]);
    p.out.clear();
    assert!(handle_container_button_click(&mut menu, &mut p.env(&rules), 2, 1, true));
    assert!(p.out.iter().any(|e| matches!(e, Effect::SetData { container_id: 2, id: 0, value: 1 })));
    assert_eq!(menu.item(&p.env(&rules), 1), &stack("stone_bricks", 1));
    menu.clicked(&mut p.env(&rules), 1, 0, ContainerInput::Pickup).unwrap();
    assert_eq!(menu.carried(), &stack("stone_bricks", 1));
    assert_eq!(menu.item(&p.env(&rules), 0), &stack("stone", 2));
    assert_eq!(menu.item(&p.env(&rules), 1), &stack("stone_bricks", 1));
    assert!(!handle_container_button_click(&mut menu, &mut p.env(&rules), 2, 1, true), "already selected");
}

#[test]
fn smithing_keeps_components_and_flags_a_missing_recipe() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::smithing(3);
    menu.open(&mut p.env(&rules));
    let mut sword = stack("diamond_sword", 1);
    sword.insert(kiln_item::keys::DAMAGE, 5);
    menu.set_slot(&mut p.env(&rules), 0, stack("netherite_upgrade_smithing_template", 1));
    menu.set_slot(&mut p.env(&rules), 1, sword);
    menu.set_slot(&mut p.env(&rules), 2, stack("netherite_ingot", 1));
    let result = menu.item(&p.env(&rules), 3).clone();
    assert_eq!(result.item(), stack("netherite_sword", 1).item());
    assert_eq!(result.get(kiln_item::keys::DAMAGE), Some(&5));
    p.out.clear();
    menu.set_slot(&mut p.env(&rules), 2, stack("iron_ingot", 1));
    assert!(menu.item(&p.env(&rules), 3).is_empty());
    menu.broadcast_changes(&mut p.env(&rules));
    assert!(p.out.iter().any(|e| matches!(e, Effect::SetData { container_id: 3, id: 0, value: 1 })));
}

#[test]
fn closing_a_station_returns_its_input() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::stonecutter(2);
    menu.open(&mut p.env(&rules));
    menu.set_slot(&mut p.env(&rules), 0, stack("stone", 3));
    menu.removed(&mut p.env(&rules));
    assert_eq!(p.inv.item(0), &stack("stone", 3));
}

#[test]
fn creative_slots_need_infinite_materials() {
    let rules = rules();
    let mut p = Player::new();
    let mut menu = Menu::inventory();
    menu.open(&mut p.env(&rules));
    handle_set_creative_slot(&mut menu, &mut p.env(&rules), 36, stack("stone", 5), true);
    assert!(p.inv.item(0).is_empty());
    p.flags.infinite_materials = true;
    handle_set_creative_slot(&mut menu, &mut p.env(&rules), 36, stack("stone", 5), true);
    assert_eq!(p.inv.item(0), &stack("stone", 5));
}

fn enchanted(name: &str, enchantments: &[(&str, i32)]) -> ItemStack {
    let mut s = stack(name, 1);
    let list = enchantments.iter().map(|(e, l)| (kiln_item::registry::ENCHANTMENT.id(e).unwrap(), *l)).collect();
    s.insert(kiln_item::keys::ENCHANTMENTS, kiln_item::component::Enchantments(list));
    s
}

#[test]
fn grindstones_disenchant_and_use_up_their_inputs() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, enchanted("minecraft:diamond_sword", &[("minecraft:sharpness", 4), ("minecraft:binding_curse", 1)]));
    p.inv.set_item(1, stack("stone", 3));
    let mut menu = Menu::grindstone(1);
    menu.open(&mut p.env(&rules));
    // Stone cannot go in; the sword can.
    menu.clicked(&mut p.env(&rules), 31, 0, ContainerInput::QuickMove).unwrap();
    assert!(menu.items(&p.env(&rules))[0].is_empty());
    menu.clicked(&mut p.env(&rules), 30, 0, ContainerInput::QuickMove).unwrap();
    let items = menu.items(&p.env(&rules));
    let result = &items[2];
    assert_eq!(result.item_name(), "minecraft:diamond_sword");
    let left: Vec<i32> = result.get(kiln_item::keys::ENCHANTMENTS).unwrap().0.iter().map(|e| e.0).collect();
    assert_eq!(left, vec![kiln_item::registry::ENCHANTMENT.id("minecraft:binding_curse").unwrap()]);
    // Taking the result empties the inputs and reports the grindstone's use.
    menu.clicked(&mut p.env(&rules), 2, 0, ContainerInput::QuickMove).unwrap();
    let items = menu.items(&p.env(&rules));
    assert!(items[0].is_empty() && items[1].is_empty() && items[2].is_empty());
    assert!(p.out.iter().any(|e| matches!(e, Effect::GrindstoneUsed { .. })));
    assert_eq!(p.inv.item(8).item_name(), "minecraft:diamond_sword");
}

#[test]
fn anvils_rename_for_creative_players_only_when_affordable() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("minecraft:iron_pickaxe", 1));
    let mut menu = Menu::anvil(1);
    menu.open(&mut p.env(&rules));
    menu.clicked(&mut p.env(&rules), 30, 0, ContainerInput::QuickMove).unwrap();
    assert!(menu.items(&p.env(&rules))[2].is_empty(), "nothing to do without a name or an addition");
    assert!(kiln_inventory::click::handle_rename_item(&mut menu, &mut p.env(&rules), "Digger", true));
    let result = menu.items(&p.env(&rules))[2].clone();
    assert_eq!(result.get(kiln_item::keys::CUSTOM_NAME).and_then(|t| t.as_plain()), Some("Digger"));
    assert_eq!(result.get(kiln_item::keys::REPAIR_COST), None.or(Some(&0)), "renaming alone keeps the repair cost");
    // One level: a player without levels cannot take it; a creative one can.
    menu.clicked(&mut p.env(&rules), 2, 0, ContainerInput::QuickMove).unwrap();
    assert!(p.inv.item(8).is_empty());
    p.flags.infinite_materials = true;
    menu.clicked(&mut p.env(&rules), 2, 0, ContainerInput::QuickMove).unwrap();
    assert_eq!(p.inv.item(8).get(kiln_item::keys::CUSTOM_NAME).and_then(|t| t.as_plain()), Some("Digger"));
    assert!(p.out.iter().any(|e| matches!(e, Effect::AnvilUsed { levels: 0 })));
    assert!(menu.items(&p.env(&rules))[0].is_empty());
}

#[test]
fn looms_add_the_selected_pattern_in_the_dye_color() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("minecraft:white_banner", 2));
    p.inv.set_item(1, stack("minecraft:red_dye", 5));
    p.inv.set_item(2, stack("stone", 1));
    let mut menu = Menu::loom(1);
    menu.open(&mut p.env(&rules));
    // Shift-clicks sort the banner and the dye into their slots; stone goes nowhere near.
    menu.clicked(&mut p.env(&rules), 31, 0, ContainerInput::QuickMove).unwrap();
    menu.clicked(&mut p.env(&rules), 32, 0, ContainerInput::QuickMove).unwrap();
    menu.clicked(&mut p.env(&rules), 33, 0, ContainerInput::QuickMove).unwrap();
    let items = menu.items(&p.env(&rules));
    assert_eq!(items[0], stack("minecraft:white_banner", 2));
    assert_eq!(items[1], stack("minecraft:red_dye", 5));
    assert!(items[2].is_empty() && items[3].is_empty(), "no pattern chosen yet");
    assert!(handle_container_button_click(&mut menu, &mut p.env(&rules), 1, 0, true));
    let result = menu.items(&p.env(&rules))[3].clone();
    assert_eq!(result.item_name(), "minecraft:white_banner");
    let layers = result.get(kiln_item::keys::BANNER_PATTERNS).unwrap();
    assert_eq!(layers.0.len(), 1);
    assert_eq!(layers.0[0].color, kiln_item::component::DyeColor::Red);
    // Taking it uses one banner and one dye; the selection stays for the next one.
    menu.clicked(&mut p.env(&rules), 3, 0, ContainerInput::Pickup).unwrap();
    assert_eq!(menu.carried().item_name(), "minecraft:white_banner");
    let items = menu.items(&p.env(&rules));
    assert_eq!(items[0].count(), 1);
    assert_eq!(items[1].count(), 4);
    assert!(!items[3].is_empty());
    assert!(p.out.iter().any(|e| matches!(e, Effect::LoomUsed)));
}

#[test]
fn enchanting_tables_keep_one_item_and_lapis_apart() {
    let rules = rules();
    let mut p = Player::new();
    p.inv.set_item(0, stack("minecraft:book", 3));
    p.inv.set_item(1, stack("minecraft:lapis_lazuli", 10));
    let mut menu = Menu::enchantment(1, 12345);
    menu.open(&mut p.env(&rules));
    menu.clicked(&mut p.env(&rules), 29, 0, ContainerInput::QuickMove).unwrap();
    menu.clicked(&mut p.env(&rules), 30, 0, ContainerInput::QuickMove).unwrap();
    let items = menu.items(&p.env(&rules));
    assert_eq!(items[0], stack("minecraft:book", 1), "one item goes in");
    assert_eq!(items[1], stack("minecraft:lapis_lazuli", 10));
    assert_eq!(p.inv.item(0).count(), 2);
    // Without bookshelves the costs are low but positive for the first option.
    assert!(p.out.iter().any(|e| matches!(e, Effect::SetData { id: 3, value, .. } if *value == 12345i32 as i16)));
}
