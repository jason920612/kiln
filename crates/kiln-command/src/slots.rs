//! `SlotRanges`: the slot names of `/item`, `/loot ... replace` and `execute if items`, with
//! the slot ids `SlotAccess` uses (`container.N` is N, `weapon.mainhand` 98, ...).

use std::sync::OnceLock;

/// Every name with its slots, in vanilla's registration order.
pub fn ranges() -> &'static [(String, Vec<i32>)] {
    static RANGES: OnceLock<Vec<(String, Vec<i32>)>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let mut out: Vec<(String, Vec<i32>)> = Vec::new();
        let single = |out: &mut Vec<(String, Vec<i32>)>, name: &str, slot: i32| out.push((name.to_owned(), vec![slot]));
        let range = |out: &mut Vec<(String, Vec<i32>)>, prefix: &str, offset: i32, size: i32| {
            for i in 0..size {
                out.push((format!("{prefix}{i}"), vec![offset + i]));
            }
            out.push((format!("{prefix}*"), (offset..offset + size).collect()));
        };
        single(&mut out, "contents", 0);
        range(&mut out, "container.", 0, 54);
        range(&mut out, "hotbar.", 0, 9);
        range(&mut out, "inventory.", 9, 27);
        range(&mut out, "enderchest.", 200, 27);
        range(&mut out, "mob.inventory.", 300, 8);
        range(&mut out, "horse.", 500, 15);
        // `EquipmentSlot.getIndex`: hands from 98, armor from 100 (feet first), the body 105.
        single(&mut out, "weapon", 98);
        single(&mut out, "weapon.mainhand", 98);
        single(&mut out, "weapon.offhand", 99);
        out.push(("weapon.*".into(), vec![98, 99]));
        single(&mut out, "armor.head", 103);
        single(&mut out, "armor.chest", 102);
        single(&mut out, "armor.legs", 101);
        single(&mut out, "armor.feet", 100);
        single(&mut out, "armor.body", 105);
        out.push(("armor.*".into(), vec![103, 102, 101, 100, 105]));
        single(&mut out, "saddle", 106);
        single(&mut out, "horse.chest", 499);
        single(&mut out, "player.cursor", 499);
        range(&mut out, "player.crafting.", 500, 4);
        out
    })
}

/// `SlotRanges.nameToIds`.
pub fn by_name(name: &str) -> Option<&'static [i32]> {
    ranges().iter().find(|(n, _)| n == name).map(|(_, s)| s.as_slice())
}

/// Slot 0-35 of a player's inventory, the equipment slots or the cursor; see
/// [`by_name`]'s ids.
pub const WEAPON_MAINHAND: i32 = 98;
pub const WEAPON_OFFHAND: i32 = 99;
