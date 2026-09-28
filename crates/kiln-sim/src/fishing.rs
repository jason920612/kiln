//! Fishing rods (`FishingRodItem.use`): casting a bobber ([`kiln_entity::ext_entity::fishing_hook`])
//! with the rod's luck of the sea and lure, and reeling it in (`FishingHook.retrieve`): a
//! hooked entity is pulled toward the player, a biting fish becomes the `gameplay/fishing`
//! loot (flung at the player, with experience and the `fish_caught` statistic), and the rod
//! wears by what the retrieve did. `fishing_rod_hooked` fires for both.

use crate::Player;
use crate::blocks::RegionLevel;
use crate::entities::{Body, Entities, Spawn};
use kiln_entity::ext_entity::fishing_hook;
use kiln_entity::math::Vec3;
use kiln_item::ItemStack;
use kiln_item::component::EquipmentSlot;
use kiln_javamath::random::RandomSource;
use kiln_proto::packets::world_fx;

/// Whether the player's `hand` holds a fishing rod (its use goes to [`use_rod`]).
pub(crate) fn holds_rod(p: &Player, off_hand: bool) -> bool {
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let s = p.inv.equipped(slot);
    !s.is_empty() && s.item_name() == "minecraft:fishing_rod"
}

/// The loot context of a catch (`LootContextParamSets.FISHING`): the bobber as `this` at its
/// position, the rod as the tool, luck; location checks see the region's biomes.
struct Catch<'a> {
    origin: [f64; 3],
    tool: &'a ItemStack,
    luck: f32,
    open_water: bool,
    dim: &'static str,
    probe: &'a crate::advancements::triggers::CellProbe<'a>,
}

impl kiln_loot::LootContext for Catch<'_> {
    fn has_entity(&self, target: kiln_loot::EntityTarget) -> bool {
        target == kiln_loot::EntityTarget::This
    }
    fn origin(&self) -> Option<[f64; 3]> {
        Some(self.origin)
    }
    fn tool(&self) -> Option<&ItemStack> {
        Some(self.tool)
    }
    fn luck(&self) -> f32 {
        self.luck
    }
    fn entity_matches(&self, target: kiln_loot::EntityTarget, predicate: &kiln_loot::predicate::EntityPredicate) -> bool {
        use kiln_loot::predicate::world::EntitySubPredicate as P;
        let bobber = kiln_item::registry::ENTITY_TYPE.id("minecraft:fishing_bobber").unwrap_or(-1);
        target == kiln_loot::EntityTarget::This
            && predicate.parts.iter().all(|part| match part {
                P::FishingHook { in_open_water } => in_open_water.is_none_or(|w| w == self.open_water),
                P::EntityType(set) => set.contains(bobber),
                P::Location(l) => crate::advancements::criteria::location_matches(l, self.origin, self.dim, Some(self.probe)),
                _ => false,
            })
    }
    fn location_matches(&self, predicate: &kiln_loot::predicate::world::LocationPredicate, pos: [f64; 3]) -> bool {
        crate::advancements::criteria::location_matches(predicate, pos, self.dim, Some(self.probe))
    }
}

/// `EnchantmentHelper.getFishingLuckBonus` / `getFishingTimeReduction` of the rod.
fn rod_value(p: &Player, rod: &ItemStack, c: kiln_loot::effects::ValueComponent) -> f32 {
    let Some(loot) = p.loot.clone() else { return 0.0 };
    let mut rng = kiln_javamath::random::LegacyRandom::new(0);
    let mut value = 0.0;
    loot.for_each_enchantment(rod, |e, level| {
        let ctx = kiln_loot::effects::ItemContext { tool: rod, level };
        value = loot.apply_value_effects(e, c, level, &ctx, &mut rng, value);
    });
    value
}

/// `FishingRodItem.use` by player `i`: reel in its bobber if it has one out, else cast.
pub(crate) fn use_rod(entities: &mut Entities, level: &mut RegionLevel, players: &mut [&mut Player], i: usize, off_hand: bool, spawns: &mut Vec<Spawn>) {
    let env = level.env;
    let pid = players[i].entity_id;
    let slot = if off_hand { EquipmentSlot::OffHand } else { EquipmentSlot::MainHand };
    let rod = players[i].inv.equipped(slot).clone();
    let mut rng = crate::container::pos_random(level, kiln_blocks::BlockPos::new(pid, 0, 0), 0x6669_7368);
    let hook = entities.list.iter().position(|e| !e.removed && e.phys.as_ref().and_then(fishing_hook::get).is_some_and(|h| h.owner == pid));
    let at = players[i].pos;
    match hook {
        Some(idx) => {
            let result = retrieve(entities, level, players, i, idx, &rod, spawns);
            if result > 0 {
                players[i].hurt_and_break(slot, result, None);
            }
            let pitch = 0.4 / (rng.next_float() * 0.4 + 0.8);
            sound(players, env, at, "minecraft:entity.fishing_bobber.retrieve", 1.0, pitch);
        }
        None => {
            let pitch = 0.4 / (rng.next_float() * 0.4 + 0.8);
            sound(players, env, at, "minecraft:entity.fishing_bobber.throw", 0.5, pitch);
            let p = &*players[i];
            let luck = rod_value(p, &rod, kiln_loot::effects::ValueComponent::FishingLuckBonus) as i32;
            let lure = (rod_value(p, &rod, kiln_loot::effects::ValueComponent::FishingTimeReduction) * 20.0) as i32;
            let seed = rng.next_long();
            let eye = p.eye_position()[1] - p.pos[1];
            let hook = fishing_hook::cast(0, 0, pid, Vec3::new(p.pos[0], p.pos[1], p.pos[2]), eye, p.rot[0], p.rot[1], luck, lure, seed);
            spawns.push(Spawn {
                kind: &kiln_data::entities::types::FISHING_BOBBER,
                pos: [hook.position().x, hook.position().y, hook.position().z],
                vel: [hook.delta.x, hook.delta.y, hook.delta.z],
                body: Body::Ready(Box::new(hook)),
            });
        }
    }
    let p = &mut *players[i];
    p.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, rod.item()), 1);
}

fn sound(players: &mut [&mut Player], env: &crate::blocks::BlockEnv, at: [f64; 3], name: &str, volume: f32, pitch: f32) {
    let Some(id) = kiln_data::builtin_id("minecraft:sound_event", name) else { return };
    let pkt = world_fx::sound(&world_fx::Sound::Registered(id), world_fx::SoundSource::Neutral, at, volume, pitch, env.game_time ^ env.seed);
    for p in players.iter_mut().filter(|p| (0..3).map(|k| (p.pos[k] - at[k]).powi(2)).sum::<f64>() < 256.0) {
        p.send(pkt.clone());
    }
}

/// `FishingHook.retrieve`: what reeling in did to the rod (5 for a hooked entity, 3 for a
/// hooked item, 1 for a catch, 2 from the ground, else 0); the bobber goes.
fn retrieve(entities: &mut Entities, level: &mut RegionLevel, players: &mut [&mut Player], i: usize, idx: usize, rod: &ItemStack, spawns: &mut Vec<Spawn>) -> i32 {
    let env = level.env;
    let dim = crate::DIMENSIONS[env.dim].0;
    let Some(phys) = entities.list[idx].phys.as_ref() else { return 0 };
    let Some(hook) = fishing_hook::get(phys).cloned() else { return 0 };
    let hook_pos = phys.position();
    let on_ground = phys.on_ground;
    let owner = [players[i].pos[0], players[i].pos[1], players[i].pos[2]];
    let rod_id = kiln_data::builtin_id("minecraft:item", "minecraft:fishing_rod").unwrap_or(-1);
    let p = &*players[i];
    let still = p.alive_for_fishing()
        && (p.inv.equipped(EquipmentSlot::MainHand).item() == rod_id && !p.inv.equipped(EquipmentSlot::MainHand).is_empty()
            || p.inv.equipped(EquipmentSlot::OffHand).item() == rod_id && !p.inv.equipped(EquipmentSlot::OffHand).is_empty())
        && hook_pos.distance_to_sqr(Vec3::new(owner[0], owner[1], owner[2])) <= 1024.0;
    let mut result = 0;
    if still {
        if let Some(target) = hook.hooked {
            // `pullEntity`: a tenth of the way to the player.
            let pull = Vec3::new(owner[0] - hook_pos.x, owner[1] - hook_pos.y, owner[2] - hook_pos.z).scale(0.1);
            let mut hooked_seen = None;
            let mut item = None;
            if let Ok(t) = entities.list.binary_search_by_key(&target, |e| e.id)
                && let Some(te) = entities.list[t].phys.as_mut()
            {
                te.delta = te.delta + pull;
                if let kiln_entity::EntityKind::Item(d) = &te.kind {
                    item = Some(d.stack.clone());
                }
                hooked_seen = Some(kiln_entity::level::Seen::of(te));
                entities.list[t].sync();
            }
            let subject = hooked_seen.as_ref().map(|s| crate::advancements::triggers::seen_subject(s, dim));
            players[i].fishing_rod_hooked(rod, subject.as_ref(), item.as_slice());
            // Entity event 31: the viewers pull too.
            let pkt = kiln_proto::packets::entity::entity_event(entities.list[idx].id, 31);
            for q in players.iter_mut().filter(|q| entities.list[idx].seen_by.binary_search(&q.conn).is_ok()) {
                q.send(pkt.clone());
            }
            result = if item.is_some() { 3 } else { 5 };
        } else if hook.nibble > 0 {
            let loot = env.loot.clone();
            let luck = hook.luck as f32 + players[i].attribute(crate::combat::LUCK) as f32;
            let probe = crate::advancements::triggers::CellProbe::new(&*level.cells, env);
            let ctx = Catch { origin: [hook_pos.x, hook_pos.y, hook_pos.z], tool: rod, luck, open_water: hook.open_water, dim, probe: &probe };
            let seed = crate::mobs::loot_seed(env.seed, env.game_time, entities.list[idx].id, 0x6669_7368);
            let items = loot.as_deref().map(|l| crate::mobs::roll(l, "minecraft:gameplay/fishing", &ctx, seed)).unwrap_or_default();
            let seen = entities.list[idx].phys.as_ref().map(kiln_entity::level::Seen::of);
            let subject = seen.as_ref().map(|s| crate::advancements::triggers::seen_subject(s, dim));
            players[i].fishing_rod_hooked(rod, subject.as_ref(), &items);
            let fishes = |s: &ItemStack| kiln_inventory::tags::contains("minecraft:item", "minecraft:fishes", s.item());
            for stack in items {
                let (dx, dy, dz) = (owner[0] - hook_pos.x, owner[1] - hook_pos.y, owner[2] - hook_pos.z);
                let lift = (dx * dx + dy * dy + dz * dz).sqrt().sqrt() * 0.08;
                let caught_fish = fishes(&stack);
                spawns.push(Spawn {
                    kind: &kiln_data::entities::types::ITEM,
                    pos: [hook_pos.x, hook_pos.y, hook_pos.z],
                    vel: [dx * 0.1, dy * 0.1 + lift, dz * 0.1],
                    body: Body::Item { stack, pickup_delay: 0, thrower: None },
                });
                // The experience from the bobber's random.
                let value = entities.list[idx].phys.as_mut().map_or(1, |h| h.random.next_int_bounded(6) + 1);
                let orb_seed = crate::mobs::loot_seed(env.seed, env.game_time, entities.list[idx].id, 0x6f72_6200 | value as u64);
                let orb = kiln_entity::xp_orb::new_at(0, 0, Vec3::new(owner[0], owner[1] + 0.5, owner[2] + 0.5), value, orb_seed);
                spawns.push(Spawn {
                    kind: &kiln_data::entities::types::EXPERIENCE_ORB,
                    pos: [owner[0], owner[1] + 0.5, owner[2] + 0.5],
                    vel: [orb.delta.x, orb.delta.y, orb.delta.z],
                    body: Body::Ready(Box::new(orb)),
                });
                if caught_fish {
                    players[i].award_stat(crate::player_stats::custom("minecraft:fish_caught"), 1);
                }
            }
            result = 1;
        }
        if on_ground {
            result = 2;
        }
    }
    let e = &mut entities.list[idx];
    if let Some(ph) = e.phys.as_mut() {
        ph.discard();
    }
    e.removed = true;
    result
}

impl Player {
    /// `canInteractWithLevel` for fishing: alive and not a spectator.
    fn alive_for_fishing(&self) -> bool {
        !self.dead && !self.disconnected && self.game_mode != 3
    }
}
