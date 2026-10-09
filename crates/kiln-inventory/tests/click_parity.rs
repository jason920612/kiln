//! Replays click sequences recorded from vanilla 26.3 (`tools/InventoryVectors.java clicks`) and
//! compares every clientbound packet, dropped item and resulting slot.
//!
//! Vectors: `$KILN_WORK/wp3-inventory/clicks*.jsonl` (skipped when absent). Without
//! `KILN_PARITY=1` only the first 300 sequences run.

mod common;

use common::{same, show, stack, stacks, unhex};
use kiln_inventory::click::{ContainerClick, handle_container_button_click, handle_container_click, handle_set_creative_slot};
use kiln_inventory::{Effect, Env, FurnaceKind, Menu, NoWorld, PlayerFlags, PlayerInventory, SimpleContainer};
use kiln_item::ItemStack;
use serde_json::Value as Json;

fn menu_for(kind: &str, id: i32) -> Menu {
    match kind {
        "inventory" => Menu::inventory(),
        "generic_9x1" => Menu::generic(id, 1),
        "generic_9x3" => Menu::generic(id, 3),
        "generic_9x6" => Menu::generic(id, 6),
        "generic_3x3" => Menu::generic_3x3(id),
        "hopper" => Menu::hopper(id),
        "shulker_box" => Menu::shulker_box(id),
        "crafting" => Menu::crafting(id),
        "furnace" => Menu::furnace(id, FurnaceKind::Furnace),
        "blast_furnace" => Menu::furnace(id, FurnaceKind::BlastFurnace),
        "smoker" => Menu::furnace(id, FurnaceKind::Smoker),
        "stonecutter" => Menu::stonecutter(id),
        "smithing" => Menu::smithing(id),
        "loom" => Menu::loom(id),
        _ => panic!("unknown menu kind {kind}"),
    }
}

/// An expected packet or drop, and ours, in a comparable form.
#[derive(Debug)]
enum Out {
    SetSlot(i32, i32, i16, ItemStack),
    SetContent(i32, i32, Vec<ItemStack>, ItemStack),
    SetCursor(ItemStack),
    SetData(i32, i16, i16),
    SetPlayerInventory(i32, ItemStack),
    Drop(ItemStack, bool),
}

fn expected(v: &Json) -> Out {
    let (k, a) = v.as_object().unwrap().iter().next().unwrap();
    let i = |n: usize| a[n].as_i64().unwrap();
    match k.as_str() {
        "set_slot" => Out::SetSlot(i(0) as i32, i(1) as i32, i(2) as i16, stack(a[3].as_str().unwrap())),
        "set_content" => Out::SetContent(i(0) as i32, i(1) as i32, stacks(&a[2]), stack(a[3].as_str().unwrap())),
        "set_cursor" => Out::SetCursor(stack(a.as_str().unwrap())),
        "set_data" => Out::SetData(i(0) as i32, i(1) as i16, i(2) as i16),
        "set_player_inventory" => Out::SetPlayerInventory(i(0) as i32, stack(a[1].as_str().unwrap())),
        "drop" => Out::Drop(stack(a.as_str().unwrap()), v["retain"].as_bool().unwrap()),
        other => panic!("unexpected record {other}"),
    }
}

fn ours(e: &Effect) -> Option<Out> {
    Some(match e {
        Effect::SetSlot { container_id, state_id, slot, stack } => Out::SetSlot(*container_id, *state_id, *slot, stack.clone()),
        Effect::SetContent { container_id, state_id, items, carried } => Out::SetContent(*container_id, *state_id, items.clone(), carried.clone()),
        Effect::SetCursor { stack } => Out::SetCursor(stack.clone()),
        Effect::SetData { container_id, id, value } => Out::SetData(*container_id, *id, *value),
        Effect::SetPlayerInventory { slot, stack } => Out::SetPlayerInventory(*slot, stack.clone()),
        Effect::Drop { stack, retain_ownership } => Out::Drop(stack.clone(), *retain_ownership),
        _ => return None,
    })
}

fn out_eq(a: &Out, b: &Out) -> bool {
    match (a, b) {
        (Out::SetSlot(c, s, i, x), Out::SetSlot(c2, s2, i2, y)) => c == c2 && s == s2 && i == i2 && same(x, y),
        (Out::SetContent(c, s, xs, x), Out::SetContent(c2, s2, ys, y)) => {
            c == c2 && s == s2 && xs.len() == ys.len() && xs.iter().zip(ys).all(|(p, q)| same(p, q)) && same(x, y)
        }
        (Out::SetCursor(x), Out::SetCursor(y)) => same(x, y),
        (Out::SetData(c, i, v), Out::SetData(c2, i2, v2)) => c == c2 && i == i2 && v == v2,
        (Out::SetPlayerInventory(i, x), Out::SetPlayerInventory(i2, y)) => i == i2 && same(x, y),
        (Out::Drop(x, r), Out::Drop(y, r2)) => r == r2 && same(x, y),
        _ => false,
    }
}

fn show_out(o: &Out) -> String {
    match o {
        Out::SetSlot(c, s, i, x) => format!("set_slot({c},{s},{i},{})", show(x)),
        Out::SetContent(c, s, _, x) => format!("set_content({c},{s},..,{})", show(x)),
        Out::SetCursor(x) => format!("set_cursor({})", show(x)),
        Out::SetData(c, i, v) => format!("set_data({c},{i},{v})"),
        Out::SetPlayerInventory(i, x) => format!("set_player_inventory({i},{})", show(x)),
        Out::Drop(x, r) => format!("drop({},{r})", show(x)),
    }
}

fn compare_out(exp: &Json, effects: &[Effect]) -> Result<(), String> {
    let exp: Vec<Out> = exp.as_array().unwrap().iter().map(expected).collect();
    let got: Vec<Out> = effects.iter().filter_map(ours).collect();
    if exp.len() == got.len() && exp.iter().zip(&got).all(|(a, b)| out_eq(a, b)) {
        return Ok(());
    }
    Err(format!(
        "packets differ:\n  vanilla: {}\n  kiln:    {}",
        exp.iter().map(show_out).collect::<Vec<_>>().join(" "),
        got.iter().map(show_out).collect::<Vec<_>>().join(" ")
    ))
}

fn compare_state(state: &Json, menu: &Menu, inv: &PlayerInventory, block: &SimpleContainer, env_items: Vec<ItemStack>) -> Result<(), String> {
    let mut errs = Vec::new();
    for (i, (e, g)) in stacks(&state["slots"]).iter().zip(&env_items).enumerate() {
        if !same(e, g) {
            errs.push(format!("slot {i}: vanilla {} kiln {}", show(e), show(g)));
        }
    }
    let carried = stack(state["carried"].as_str().unwrap());
    if !same(&carried, menu.carried()) {
        errs.push(format!("carried: vanilla {} kiln {}", show(&carried), show(menu.carried())));
    }
    for (i, e) in stacks(&state["inv"]).iter().enumerate() {
        let g = kiln_inventory::Container::item(inv, i);
        if !same(e, g) {
            errs.push(format!("inventory {i}: vanilla {} kiln {}", show(e), show(g)));
        }
    }
    for (i, e) in stacks(&state["block"]).iter().enumerate() {
        if let Some(g) = block.items.get(i)
            && !same(e, g)
        {
            errs.push(format!("block {i}: vanilla {} kiln {}", show(e), show(g)));
        }
    }
    let sid = state["state_id"].as_i64().unwrap() as i32;
    if sid != menu.state_id() {
        errs.push(format!("state id: vanilla {sid} kiln {}", menu.state_id()));
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("\n  ")) }
}

#[test]
fn click_sequences_match_vanilla() {
    let dir = common::work().join("wp3-inventory");
    // (`KILN_CLICK_VECTORS`: these files, joined by `:`, instead of the directory's.)
    let mut files: Vec<std::path::PathBuf> = match std::env::var("KILN_CLICK_VECTORS") {
        Ok(list) => list.split(':').map(Into::into).collect(),
        Err(_) => {
            let mut f: Vec<_> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            f.retain(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("clicks") && n.ends_with(".jsonl")));
            f
        }
    };
    files.sort();
    if files.is_empty() {
        eprintln!("skipping: no {}/clicks*.jsonl", dir.display());
        return;
    }
    let text: String = files.iter().map(|f| std::fs::read_to_string(f).unwrap()).collect::<Vec<_>>().join("\n");
    let Some(rules) = common::rules() else {
        eprintln!("skipping: vanilla datapack not found");
        return;
    };
    let limit = if common::full_parity() { usize::MAX } else { 300 };
    let (mut sequences, mut steps, mut failures) = (0, 0, Vec::new());
    for (n, line) in text.lines().filter(|l| !l.is_empty()).take(limit).enumerate() {
        let seq: Json = serde_json::from_str(line).unwrap();
        sequences += 1;
        if let Err(e) = replay(&seq, rules, &mut steps) {
            failures.push(format!("sequence {n} ({}): {e}", seq["kind"]));
        }
    }
    for f in failures.iter().take(15) {
        eprintln!("{f}\n");
    }
    eprintln!("{sequences} sequences, {steps} steps, {} failing sequences", failures.len());
    assert!(failures.is_empty(), "{} of {sequences} sequences differ from vanilla", failures.len());
}

fn replay(seq: &Json, rules: &kiln_inventory::Rules, steps: &mut usize) -> Result<(), String> {
    let kind = seq["kind"].as_str().unwrap();
    let id = seq["container_id"].as_i64().unwrap() as i32;
    let creative = seq["creative"].as_bool().unwrap();
    let mut inv = PlayerInventory::new();
    for (i, s) in stacks(&seq["inv"]).into_iter().enumerate() {
        kiln_inventory::Container::set_item(&mut inv, i, s);
    }
    inv.selected = seq["selected"].as_u64().unwrap() as usize;
    let mut block = SimpleContainer::from_items(stacks(&seq["block"]));
    block.data = seq["data"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
    let has_block = !matches!(kind, "inventory" | "crafting" | "stonecutter" | "smithing" | "loom");
    let mut menu = menu_for(kind, id);
    let flags = PlayerFlags { creative, infinite_materials: creative, ..Default::default() };
    let mut world = NoWorld;
    let mut out = Vec::new();
    macro_rules! env {
        () => {
            Env {
                inventory: &mut inv,
                block: if has_block { Some(&mut block) } else { None },
                player: flags,
                rules,
                world: &mut world,
                out: &mut out,
            }
        };
    }
    // A crafting grid starts at slot 1, station inputs at slot 0.
    let first = if matches!(kind, "stonecutter" | "smithing" | "loom") { 0 } else { 1 };
    for (i, s) in seq.get("grid").map(stacks).unwrap_or_default().into_iter().enumerate() {
        if !s.is_empty() {
            menu.set_slot(&mut env!(), first + i, s);
        }
    }
    menu.open(&mut env!());
    compare_out(&seq["open"], &out).map_err(|e| format!("open: {e}"))?;
    for (k, step) in seq["steps"].as_array().unwrap().iter().enumerate() {
        *steps += 1;
        out.clear();
        if std::env::var_os("LOOM_DBG").is_some() { eprintln!("-- step {k}"); }
        let what;
        if let Some(click) = step.get("click") {
            let click = ContainerClick::decode(&unhex(click.as_str().unwrap())).map_err(|e| format!("step {k}: decode: {e}"))?;
            what = format!("{:?} slot {} button {}", click.input, click.slot, click.button);
            let result = handle_container_click(&mut menu, &mut env!(), &click, true);
            if step.get("crash").is_some() {
                return if result.is_err() { Ok(()) } else { Err(format!("step {k} ({what}): vanilla crashed, kiln did not")) };
            }
            result.map_err(|e| format!("step {k} ({what}): kiln crashed: {e}"))?;
        } else if step.get("close").is_some() {
            what = "close".into();
            menu.removed(&mut env!());
        } else if let Some(b) = step.get("button") {
            let (cid, button) = (b[0].as_i64().unwrap() as i32, b[1].as_i64().unwrap() as i32);
            what = format!("button {button} (container {cid})");
            handle_container_button_click(&mut menu, &mut env!(), cid, button, true);
        } else {
            let c = step["creative"].as_array().unwrap();
            let slot = c[0].as_i64().unwrap() as i16;
            let bytes = unhex(c[1].as_str().unwrap());
            let s = ItemStack::read_untrusted_optional(&mut kiln_proto::Reader::new(&bytes)).unwrap();
            what = format!("creative slot {slot} {}", show(&s));
            let mut inventory_menu = std::mem::replace(&mut menu, Menu::inventory());
            handle_set_creative_slot(&mut inventory_menu, &mut env!(), slot, s, true);
            menu = inventory_menu;
        }
        compare_out(&step["out"], &out).map_err(|e| format!("step {k} ({what}): {e}"))?;
        let items = {
            let env = env!();
            menu.items(&env)
        };
        compare_state(&step["state"], &menu, &inv, &block, items).map_err(|e| format!("step {k} ({what}):\n  {e}"))?;
    }
    Ok(())
}
