#[test]
fn sizes() {
    use std::mem::size_of;
    println!("Entity {} align {}", size_of::<kiln_entity::Entity>(), std::mem::align_of::<kiln_entity::Entity>());
    println!("EntityKind {}", size_of::<kiln_entity::EntityKind>());
    println!("ItemData {}", size_of::<kiln_entity::item::ItemData>());
    println!("ItemStack {}", size_of::<kiln_item::ItemStack>());
    println!("ArrowData {}", size_of::<kiln_entity::arrow::ArrowData>());
    println!("PlayerData {}", size_of::<kiln_entity::player::PlayerData>());
    println!("FallingBlockData {}", size_of::<kiln_entity::falling_block::FallingBlockData>());
    println!("ThrowableData {}", size_of::<kiln_entity::projectile::ThrowableData>());
    println!("TntData {}", size_of::<kiln_entity::tnt::TntData>());
    println!("OrbData {}", size_of::<kiln_entity::xp_orb::OrbData>());
    println!("LegacyRandom {}", size_of::<kiln_javamath::random::LegacyRandom>());
}
