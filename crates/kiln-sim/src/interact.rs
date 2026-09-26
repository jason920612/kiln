//! Block placement rules (for now: block items place their block, oriented by axis/facing).

use kiln_data::blocks_types::{BlockInfo, block_by_name, has_fluid, is_air};

/// Block face / direction ids as used by the protocol (Direction.get3DDataValue).
pub const DOWN: i32 = 0;
pub const UP: i32 = 1;
pub const NORTH: i32 = 2;
pub const SOUTH: i32 = 3;
pub const WEST: i32 = 4;
pub const EAST: i32 = 5;

pub fn offset(face: i32) -> Option<[i32; 3]> {
    Some(match face {
        DOWN => [0, -1, 0],
        UP => [0, 1, 0],
        NORTH => [0, 0, -1],
        SOUTH => [0, 0, 1],
        WEST => [-1, 0, 0],
        EAST => [1, 0, 0],
        _ => return None,
    })
}

/// The block an item places, if it is a block item with a block of the same name.
pub fn block_for_item(item: i32) -> Option<&'static BlockInfo> {
    let name = kiln_data::builtin_entries("minecraft:item")?.get(usize::try_from(item).ok()?)?;
    let block = block_by_name(name)?;
    (!is_air(block.default)).then_some(block)
}

/// Whether placing into a position holding `state` replaces it.
pub fn replaceable(state: u16) -> bool {
    is_air(state) || (has_fluid(state) && kiln_data::blocks_types::block_of(state).properties.iter().any(|p| p.name == "level"))
}

/// Horizontal direction the player faces, from yaw in degrees.
fn horizontal_facing(yaw: f32) -> &'static str {
    match (((yaw as f64 / 90.0) + 0.5).floor() as i64).rem_euclid(4) {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    }
}

/// The state to place for `block` against `face`, given the player's yaw.
pub fn placement_state(block: &BlockInfo, face: i32, yaw: f32) -> u16 {
    let mut state = block.default;
    if block.properties.iter().any(|p| p.name == "axis") {
        let axis = match face {
            DOWN | UP => "y",
            NORTH | SOUTH => "z",
            _ => "x",
        };
        state = block.with_property(state, "axis", axis).unwrap_or(state);
    }
    if let Some(p) = block.properties.iter().find(|p| p.name == "facing") {
        // Stairs, furnaces, etc. face the player; blocks with vertical facing use the clicked face.
        let dir = if p.values.len() == 6 && matches!(face, DOWN | UP) {
            if face == UP { "up" } else { "down" }
        } else {
            match horizontal_facing(yaw) {
                "south" => "north",
                "north" => "south",
                "west" => "east",
                _ => "west",
            }
        };
        let dir = if block.name.ends_with("_stairs") { opposite(dir) } else { dir };
        state = block.with_property(state, "facing", dir).unwrap_or(state);
    }
    state
}

fn opposite(dir: &str) -> &'static str {
    match dir {
        "north" => "south",
        "south" => "north",
        "east" => "west",
        "west" => "east",
        "up" => "down",
        _ => "up",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_follow_the_clicked_axis_and_stairs_face_away() {
        let log = block_by_name("minecraft:oak_log").unwrap();
        assert_eq!(log.property(placement_state(log, EAST, 0.0), "axis"), Some("x"));
        assert_eq!(log.property(placement_state(log, UP, 0.0), "axis"), Some("y"));
        let stairs = block_by_name("minecraft:oak_stairs").unwrap();
        // Looking south (yaw 0): vanilla stairs placed on the ground face south.
        assert_eq!(stairs.property(placement_state(stairs, UP, 0.0), "facing"), Some("south"));
    }

    #[test]
    fn block_items_map_to_blocks() {
        let stone = kiln_data::builtin_id("minecraft:item", "minecraft:stone").unwrap();
        assert_eq!(block_for_item(stone).unwrap().name, "minecraft:stone");
        let stick = kiln_data::builtin_id("minecraft:item", "minecraft:stick").unwrap();
        assert!(block_for_item(stick).is_none());
    }
}
