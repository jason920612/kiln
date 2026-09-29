use super::*;
use crate::memory::MemoryLevel;
use kiln_item::ItemStack;

fn round_trip(e: &Entity) -> Entity {
    let saved = crate::persist::save(e, &|_| None);
    crate::persist::load(&saved, e.id, 5).expect("loads")
}

fn cart_of(e: &Entity) -> &Minecart {
    crate::ext_entity::get::<Minecart>(e).expect("a minecart")
}

#[test]
fn cargo_saves_and_loads() {
    let stone = |n| ItemStack::of("minecraft:stone", n).unwrap();
    let mut chest = new("minecraft:chest_minecart", Vec3::new(0.5, 1.0625, 0.5), 1);
    crate::ext_entity::get_mut::<Minecart>(&mut chest).unwrap().contents.as_mut().unwrap().items[4] = stone(9);
    let back = round_trip(&chest);
    assert_eq!(cart_of(&back).contents.as_ref().unwrap().items[4].count(), 9);
    assert_eq!(cart_of(&back).contents.as_ref().unwrap().items.len(), 27);

    // An unopened loot table is saved instead of the items (with its seed).
    let m = crate::ext_entity::get_mut::<Minecart>(&mut chest).unwrap();
    m.contents.as_mut().unwrap().loot_table = Some("minecraft:chests/simple_dungeon".into());
    m.contents.as_mut().unwrap().loot_seed = 77;
    let saved = crate::persist::save(&chest, &|_| None);
    assert!(saved.get("Items").is_none());
    assert_eq!(saved.get("LootTableSeed").and_then(Tag::as_i64), Some(77));
    let back = round_trip(&chest);
    let c = cart_of(&back).contents.as_ref().unwrap();
    assert_eq!((c.loot_table.as_deref(), c.loot_seed), (Some("minecraft:chests/simple_dungeon"), 77));

    let mut hopper = new("minecraft:hopper_minecart", Vec3::ZERO, 2);
    crate::ext_entity::get_mut::<Minecart>(&mut hopper).unwrap().enabled = false;
    let back = round_trip(&hopper);
    assert!(!cart_of(&back).enabled);
    assert_eq!(cart_of(&back).contents.as_ref().unwrap().items.len(), 5);

    let mut furnace = new("minecraft:furnace_minecart", Vec3::ZERO, 3);
    let m = crate::ext_entity::get_mut::<Minecart>(&mut furnace).unwrap();
    m.fuel = 1234;
    m.push = Vec3::new(0.25, 0.0, -0.5);
    let back = round_trip(&furnace);
    assert_eq!((cart_of(&back).fuel, cart_of(&back).push), (1234, Vec3::new(0.25, 0.0, -0.5)));

    let mut tnt = new("minecraft:tnt_minecart", Vec3::ZERO, 4);
    let m = crate::ext_entity::get_mut::<Minecart>(&mut tnt).unwrap();
    m.fuse = 33;
    m.explosion_power = 9.0;
    let back = round_trip(&tnt);
    assert_eq!((cart_of(&back).fuse, cart_of(&back).explosion_power, cart_of(&back).explosion_speed_factor), (33, 9.0, 1.0));
}

#[test]
fn fuller_container_carts_slow_down_less() {
    let stone = |n| ItemStack::of("minecraft:stone", n).unwrap();
    let empty = new("minecraft:chest_minecart", Vec3::new(0.5, 5.0, 0.5), 1);
    let mut full = new("minecraft:chest_minecart", Vec3::new(0.5, 5.0, 0.5), 1);
    crate::ext_entity::get_mut::<Minecart>(&mut full).unwrap().contents.as_mut().unwrap().items.iter_mut().for_each(|s| *s = stone(64));
    for (e, want) in [(&empty, 0.98f32 + 15.0 * 0.001), (&full, 0.98)] {
        let mut m = cart_of(e).clone();
        let v = m.slowdown(e, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(v.x, want as f64, "signal {}", m.contents.as_ref().map_or(0, |c| c.signal()));
    }
}

#[test]
fn a_hopper_minecart_takes_in_item_entities_unless_switched_off() {
    for enabled in [true, false] {
        let mut level = MemoryLevel::new(-64, 0);
        level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
        let mut hopper = new("minecraft:hopper_minecart", Vec3::new(0.5, 0.0625, 0.5), 1);
        hopper.id = 1;
        crate::ext_entity::get_mut::<Minecart>(&mut hopper).unwrap().enabled = enabled;
        level.insert(hopper);
        let mut coal = crate::item::new_at(2, 0, ItemStack::of("minecraft:coal", 3).unwrap(), Vec3::new(0.5, 0.3, 0.5), 9);
        coal.no_gravity = true;
        coal.delta = Vec3::ZERO;
        level.insert(coal);
        for _ in 0..3 {
            level.tick();
        }
        let taken = cart_of(level.entity_at(0).unwrap()).contents.as_ref().unwrap().items[0].count();
        assert_eq!(taken, if enabled { 3 } else { 0 });
        assert_eq!(level.entity_at(1).unwrap().is_removed(), enabled);
    }
}

/// What a tick costs per minecart of each kind: 1000 of them on rails, half of them rolling.
/// `cargo test -p kiln-entity minecart_tick_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn minecart_tick_cost() {
    let rail = kiln_data::blocks_types::block_by_name("minecraft:rail").unwrap();
    let rail = rail.with_property(rail.default, "shape", "east_west").unwrap();
    for kind in ["minecraft:minecart", "minecraft:chest_minecart", "minecraft:hopper_minecart", "minecraft:furnace_minecart", "minecraft:tnt_minecart"] {
        let mut level = MemoryLevel::new(-64, 0);
        level.bottom_layer = Some(kiln_data::blocks::default_state::BEDROCK);
        let mut id = 1;
        for row in 0..40 {
            for x in -60..60 {
                level.blocks.insert(crate::math::BlockPos::new(x, 0, row * 3), rail);
            }
            for k in 0..25 {
                let mut e = new(kind, Vec3::new(-50.0 + k as f64 * 4.0, 0.0625, (row * 3) as f64 + 0.5), id as i64);
                e.id = id;
                id += 1;
                if k % 2 == 0 {
                    e.delta = Vec3::new(0.2, 0.0, 0.0);
                }
                if let Some(m) = crate::ext_entity::get_mut::<Minecart>(&mut e) {
                    m.fuel = 5000;
                    m.push = Vec3::new(0.5, 0.0, 0.0);
                    if let Some(c) = &mut m.contents {
                        for (i, s) in c.items.iter_mut().enumerate().step_by(2) {
                            *s = ItemStack::of("minecraft:stone", 12 + i as i32).unwrap();
                        }
                    }
                }
                level.insert(e);
            }
        }
        for _ in 0..10 {
            level.tick();
        }
        let start = std::time::Instant::now();
        for _ in 0..50 {
            level.tick();
        }
        let per = start.elapsed().as_nanos() as f64 / (50.0 * 1000.0);
        eprintln!("{kind:32} {per:8.0} ns per minecart tick");
    }
}
