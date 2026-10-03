//! `onEquipItem` for the screen of a horse: the seed of the equip sound comes off the animal's
//! random when the item goes into the slot (the packet), not when the animal next ticks.

use kiln_entity::mob::kinds::horse;
use kiln_entity::mob::{self, MobKind};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;

#[test]
fn the_equip_sound_seed_is_drawn_when_the_screen_changes_the_slot() {
    let seed = 0x5eed;
    let mut e = mob::new(MobKind::Horse, 7, 0x1111, seed);
    e.first_tick = false;
    let mut slots = horse::mount_slots(mob::data(&e).unwrap()).expect("a horse has a screen");
    assert!(slots[0].is_empty());
    slots[0] = ItemStack::of("minecraft:saddle", 1).unwrap();
    let mut reference = e.random.clone();
    horse::set_mount_slots(&mut e, &slots);
    // One `nextLong` went to the sound's seed, at once.
    reference.next_long();
    assert_eq!(e.random.next_long(), reference.next_long());
    // The same item again is no change: nothing is drawn.
    let mut e2 = mob::new(MobKind::Horse, 8, 0x2222, seed);
    e2.first_tick = false;
    let slots2 = horse::mount_slots(mob::data(&e2).unwrap()).unwrap();
    let mut reference2 = e2.random.clone();
    horse::set_mount_slots(&mut e2, &slots2);
    assert_eq!(e2.random.next_long(), reference2.next_long());
    // A silent animal draws nothing either (`!isSilent`).
    let mut slots3 = slots2.clone();
    slots3[0] = ItemStack::of("minecraft:saddle", 1).unwrap();
    let mut e3 = mob::new(MobKind::Horse, 9, 0x3333, seed);
    e3.first_tick = false;
    e3.silent = true;
    let mut reference3 = e3.random.clone();
    horse::set_mount_slots(&mut e3, &slots3);
    assert_eq!(e3.random.next_long(), reference3.next_long());
}
