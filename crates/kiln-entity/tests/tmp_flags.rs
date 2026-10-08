#[test]
fn tmp_flags() {
    for n in ["minecraft:leaf_litter", "minecraft:short_grass", "minecraft:grass_block", "minecraft:fern", "minecraft:dirt", "minecraft:oak_leaves", "minecraft:air", "minecraft:snow"] {
        let b = kiln_data::blocks_types::block_by_name(n).unwrap();
        let s = b.default;
        println!("FLAGS {n} valid_spawn_animal {} valid_empty_animal {} valid_empty_monster {} full {}", kiln_entity::mob::path::valid_spawn(s, true), kiln_entity::mob::path::valid_empty_spawn(s, true), kiln_entity::mob::path::valid_empty_spawn(s, false), kiln_entity::mob::path::collision_full_block(s));
    }
}
