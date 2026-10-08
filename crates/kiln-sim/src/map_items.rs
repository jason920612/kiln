//! Maps in play: the empty map (`EmptyMapItem`), a filled map used on a banner (`MapItem.useOn`),
//! crafted copies (`MapItem.onCraftedPostProcess`) and the per-tick work of a held map
//! (`MapItem.inventoryTick`, `ServerPlayer.synchronizeSpecialItemUpdates`).

use crate::Player;
use crate::entities::Spawn;
use crate::maps::{self, MapData, MapWorld, SharedMaps, Viewer};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_world::{Blocks, ChunkPos};

/// The chunks a map is drawn from.
pub(crate) struct ChunkWorld<'a, B: Blocks> {
    pub blocks: &'a B,
    pub min_y: i32,
    pub ceiling: bool,
}

impl<B: Blocks> MapWorld for ChunkWorld<'_, B> {
    fn min_y(&self) -> i32 {
        self.min_y
    }

    fn has_ceiling(&self) -> bool {
        self.ceiling
    }

    fn loaded(&self, cx: i32, cz: i32) -> bool {
        self.blocks.chunk(ChunkPos::new(cx, cz)).is_some()
    }

    fn surface(&self, x: i32, z: i32) -> i32 {
        self.blocks.chunk(ChunkPos::of_block(x, z)).map_or(self.min_y, |c| c.surface_y((x & 15) as usize, (z & 15) as usize))
    }

    fn block(&self, x: i32, y: i32, z: i32) -> u16 {
        self.blocks.get_block(x, y, z).unwrap_or(kiln_data::blocks::default_state::VOID_AIR)
    }

    fn banner(&self, x: i32, y: i32, z: i32) -> Option<(u8, Option<Tag>)> {
        let state = self.blocks.get_block(x, y, z)?;
        let name = kiln_data::blocks_types::block_of(state).name;
        let colour = name.strip_prefix("minecraft:")?.strip_suffix("_wall_banner").or_else(|| name.strip_prefix("minecraft:")?.strip_suffix("_banner"))?;
        let color = maps::DYES.iter().position(|d| *d == colour)? as u8;
        let custom = self.blocks.chunk(ChunkPos::of_block(x, z))?.block_entity((x & 15) as usize, y, (z & 15) as usize).and_then(|be| be.nbt.get("CustomName").cloned());
        Some((color, custom))
    }
}

/// `MapItem.onCraftedPostProcess`: a map crafted with a glass pane or paper becomes a new map.
pub(crate) fn post_process(maps: &SharedMaps, stack: &mut ItemStack) {
    let Some(kind) = stack.get(kiln_item::keys::MAP_POST_PROCESSING).copied() else { return };
    stack.remove(kiln_item::component::ids::MAP_POST_PROCESSING);
    let Some(id) = maps::map_id_of(stack) else { return };
    let Ok(mut store) = maps.lock() else { return };
    let Some(data) = store.get(id) else { return };
    let copy = match kind {
        kiln_item::component::MapPostProcessing::Lock => data.locked_copy(),
        kiln_item::component::MapPostProcessing::Scale => data.scaled(),
    };
    let new = store.free_id();
    store.set(new, copy);
    stack.insert(kiln_item::keys::MAP_ID, kiln_item::component::MapId(new));
}

/// `MapItem.create`: a filled map of the area around (`x`, `z`) in the level `dimension`.
pub(crate) fn create(maps: &SharedMaps, x: i32, z: i32, scale: i8, tracking: bool, unlimited: bool, dimension: &str) -> ItemStack {
    let mut stack = ItemStack::of("minecraft:filled_map", 1).expect("filled map");
    let data = MapData::create_fresh(x as f64, z as f64, scale, tracking, unlimited, dimension);
    let mut store = maps.lock().unwrap_or_else(|e| e.into_inner());
    let id = store.free_id();
    store.set(id, data);
    stack.insert(kiln_item::keys::MAP_ID, kiln_item::component::MapId(id));
    stack
}

/// `EmptyMapItem.use`: the empty map becomes a filled map of the area around the player.
pub(crate) fn use_empty_map(p: &mut Player, off_hand: bool, dim: crate::DimId, spawns: &mut Vec<Spawn>) {
    use kiln_inventory::Container;
    let used = p.in_hand(off_hand).clone();
    if !p.infinite_materials() {
        let i = p.hand_index(off_hand);
        p.inv.item_mut(i).shrink(1);
        p.inv.times_changed += 1;
    }
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, used.item()), 1);
    p.sound_for_all("minecraft:ui.cartography_table.take_result", kiln_proto::packets::world_fx::SoundSource::Player, 1.0, 1.0);
    let (x, z) = (p.pos[0].floor() as i32, p.pos[2].floor() as i32);
    let map = create(&p.maps, x, z, 0, true, false, crate::DIMENSIONS[dim].0);
    let i = p.hand_index(off_hand);
    if p.inv.item(i).is_empty() {
        // `heldItemTransformedTo`.
        *p.inv.item_mut(i) = map;
        p.inv.times_changed += 1;
    } else {
        let mut rest = map.clone();
        let infinite = p.infinite_materials();
        if !p.inv.add(None, &mut rest, infinite) {
            spawns.push(p.throw(map));
        }
    }
}

/// `MapItem.useOn` on a banner: the banner is put on the map or taken off it. `None`: not a banner.
pub(crate) fn use_on_banner<B: Blocks>(p: &Player, off_hand: bool, blocks: &B, min_y: i32, ceiling: bool, pos: [i32; 3], game_time: i64) -> Option<bool> {
    let state = blocks.get_block(pos[0], pos[1], pos[2])?;
    if !kiln_blocks::tags::is(state, "minecraft:banners") {
        return None;
    }
    let world = ChunkWorld { blocks, min_y, ceiling };
    let ok = match maps::map_id_of(p.in_hand(off_hand)) {
        Some(id) => {
            let mut store = p.maps.lock().unwrap_or_else(|e| e.into_inner());
            store.get(id).is_none_or(|m| m.toggle_banner(&world, pos, game_time))
        }
        None => true,
    };
    Some(ok)
}

/// Whether the level has a ceiling (`DimensionType.hasCeiling`): the Nether's maps are drawn differently.
pub(crate) fn has_ceiling(dim: crate::DimId) -> bool {
    kiln_data::dimension_type(crate::DIMENSIONS[dim].0).is_some_and(|d| d.has_ceiling)
}

/// Filled-map ids in the stack list (all slots of the inventory).
fn map_ids(p: &Player) -> Vec<i32> {
    p.inv.items.iter().chain(p.inv.equipment.iter()).filter_map(|s| if s.is_empty() { None } else { maps::map_id_of(s) }).collect()
}

/// `EquipmentSlot` order of an `EntityEquipment` (an `EnumMap`: off hand, feet, legs, chest, head, body,
/// saddle) as indices into `PlayerInventory::equipment` (feet, legs, chest, head, off hand, body, saddle).
const EQUIPMENT_TICK_ORDER: [usize; 7] = [4, 0, 1, 2, 3, 5, 6];

impl crate::Sim {
    /// `Inventory.tick`, `EntityEquipment.tick` and `ServerPlayer.doTick` for filled maps: every map a player
    /// carries notes the player, a map in a hand redraws the part of the world around the player, and
    /// what changed is sent.
    pub(crate) fn tick_maps(&mut self) {
        if !self.players.values().any(|p| !map_ids(p).is_empty()) {
            return;
        }
        let viewers: std::collections::HashMap<uuid::Uuid, Viewer> = self
            .players
            .values()
            .map(|p| {
                let hidden = p.inv.equipment.iter().enumerate().any(|(i, s)| i != 4 && !s.is_empty() && kiln_entity::mob::item_tag(s.item(), "minecraft:map_invisibility_equipment"));
                (p.uuid, Viewer { uuid: p.uuid, name: p.name.clone(), dim: p.dim, pos: p.pos, yaw: p.rot[0], maps: map_ids(p), hidden })
            })
            .collect();
        let lookup = |u: uuid::Uuid| viewers.get(&u).cloned();
        let mut conns: Vec<_> = self.players.iter().filter(|(_, p)| !viewers[&p.uuid].maps.is_empty()).map(|(c, _)| *c).collect();
        conns.sort();
        let shared = self.maps.clone();
        let mut store = shared.lock().unwrap_or_else(|e| e.into_inner());
        let time = self.game_time;
        for conn in conns {
            let Some(p) = self.players.get_mut(&conn) else { continue };
            let viewer = viewers[&p.uuid].clone();
            let world = ChunkWorld { blocks: &self.dims[p.dim].regions, min_y: self.dims[p.dim].provider.dimension.min_y, ceiling: has_ceiling(p.dim) };
            let selected = p.inv.selected;
            // `Inventory.tick`: the main slots; the selected one is the main hand.
            for (i, stack) in p.inv.items.iter().enumerate() {
                let Some(id) = (!stack.is_empty()).then(|| maps::map_id_of(stack)).flatten() else { continue };
                let Some(data) = store.get(id) else { continue };
                data.tick_carried_by(&viewer, id, stack, None, time, &lookup);
                if !data.locked && i == selected {
                    data.update(&world, &viewer);
                }
            }
            // `EntityEquipment.tick`: the off hand also redraws, armour only notes the player.
            for &i in &EQUIPMENT_TICK_ORDER {
                let stack = &p.inv.equipment[i];
                let Some(id) = (!stack.is_empty()).then(|| maps::map_id_of(stack)).flatten() else { continue };
                let Some(data) = store.get(id) else { continue };
                data.tick_carried_by(&viewer, id, stack, None, time, &lookup);
                if !data.locked && i == 4 {
                    data.update(&world, &viewer);
                }
            }
            // `ServerPlayer.doTick`: what changed goes to the client.
            let mut packets = Vec::new();
            for stack in p.inv.items.iter().chain(p.inv.equipment.iter()) {
                let Some(id) = (!stack.is_empty()).then(|| maps::map_id_of(stack)).flatten() else { continue };
                if let Some(pkt) = store.get(id).and_then(|d| d.update_packet(id, p.uuid)) {
                    packets.push(pkt);
                }
            }
            for pkt in packets {
                p.send(pkt);
            }
        }
    }
}
