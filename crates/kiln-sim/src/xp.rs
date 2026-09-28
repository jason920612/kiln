//! Player experience (`Player.giveExperiencePoints` / `giveExperienceLevels`,
//! `ExperienceOrb.playerTouch`): orbs picked up add points, levels pay for anvils and
//! enchanting, a dead player drops some as orbs; mending (`repair_with_xp`) spends orb points
//! on damaged equipment first.

use crate::Player;
use crate::entities::Entities;
use kiln_entity::EntityKind;
use kiln_proto::packets;
use kiln_proto::packets::entity;

impl Player {
    /// `Player.getXpNeededForNextLevel`.
    pub(crate) fn xp_needed_for_next_level(&self) -> i32 {
        if self.xp_level >= 30 {
            112 + (self.xp_level - 30) * 9
        } else if self.xp_level >= 15 {
            37 + (self.xp_level - 15) * 5
        } else {
            7 + self.xp_level * 2
        }
    }

    /// `Player.giveExperiencePoints`.
    pub(crate) fn give_experience_points(&mut self, points: i32) {
        self.xp_progress += points as f32 / self.xp_needed_for_next_level() as f32;
        self.xp_total = self.xp_total.saturating_add(points).max(0);
        while self.xp_progress < 0.0 {
            let f = self.xp_progress * self.xp_needed_for_next_level() as f32;
            if self.xp_level > 0 {
                self.give_experience_levels(-1);
                self.xp_progress = 1.0 + f / self.xp_needed_for_next_level() as f32;
            } else {
                self.give_experience_levels(-1);
                self.xp_progress = 0.0;
            }
        }
        while self.xp_progress >= 1.0 {
            self.xp_progress = (self.xp_progress - 1.0) * self.xp_needed_for_next_level() as f32;
            self.give_experience_levels(1);
            self.xp_progress /= self.xp_needed_for_next_level() as f32;
        }
    }

    /// `Player.giveExperienceLevels` (negative to pay levels).
    pub(crate) fn give_experience_levels(&mut self, levels: i32) {
        self.xp_level = self.xp_level.saturating_add(levels);
        if self.xp_level < 0 {
            self.xp_level = 0;
            self.xp_progress = 0.0;
            self.xp_total = 0;
        }
    }

    /// `Player.onEnchantmentPerformed` / the anvil's cost: levels paid, unless creative.
    pub(crate) fn pay_levels(&mut self, levels: i32) {
        if self.game_mode != 1 {
            self.give_experience_levels(-levels);
        }
    }

    /// Set Experience when the player's experience changed (`ServerPlayer.doTick`).
    pub(crate) fn sync_experience(&mut self) {
        let now = (self.xp_progress.to_bits(), self.xp_level, self.xp_total);
        if self.sent_xp != Some(now) {
            self.sent_xp = Some(now);
            self.send(packets::player::set_experience(self.xp_progress, self.xp_level, self.xp_total));
        }
    }

    /// `ExperienceOrb.repairPlayerItems`: a random damaged equipped item with a `repair_with_xp`
    /// enchantment (the pick from the player's entity random) is repaired by what the
    /// enchantment makes of `amount` (from the level random); the points the repair did not
    /// use go to the next item. Returns the points left for the player.
    pub(crate) fn repair_with_xp(&mut self, amount: i32) -> i32 {
        use kiln_javamath::random::RandomSource;
        let Some(loot) = self.loot.clone() else { return amount };
        let mut amount = amount;
        loop {
            let slots: Vec<kiln_item::component::EquipmentSlot> = {
                let equipment = self.equipment();
                loot.items_with(kiln_loot::effects::ValueComponent::RepairWithXp, &equipment, kiln_item::ItemStack::is_damaged)
                    .into_iter()
                    .map(|(s, _)| s)
                    .collect()
            };
            if slots.is_empty() {
                return amount;
            }
            let slot = slots[self.entity_rng.next_int_bounded(slots.len() as i32) as usize];
            let index = kiln_inventory::inventory::equipment_index(slot, self.inv.selected);
            let stack = kiln_inventory::Container::item(&self.inv, index).clone();
            let to_repair = loot.durability_from_xp(&stack, &mut self.level_rng, amount);
            let repaired = to_repair.min(stack.damage());
            kiln_inventory::Container::item_mut(&mut self.inv, index).insert(kiln_item::keys::DAMAGE, stack.damage() - repaired);
            if repaired > 0 {
                let used = amount.wrapping_sub(repaired.wrapping_mul(amount) / to_repair);
                if used > 0 {
                    amount = used;
                    continue;
                }
            }
            return 0;
        }
    }

    /// `Player.getBaseExperienceReward`: what a dead player drops.
    pub(crate) fn death_experience(&self, keep_inventory: bool) -> i32 {
        if keep_inventory || self.game_mode == 3 { 0 } else { (self.xp_level * 7).min(100) }
    }
}

/// Players touching experience orbs take them, one orb value at a time with a delay of two
/// ticks between orbs (`ExperienceOrb.playerTouch`).
pub(crate) fn pick_up_orbs(entities: &mut Entities, players: &mut [&mut Player]) {
    for p in players.iter_mut() {
        if p.take_xp_delay > 0 {
            p.take_xp_delay -= 1;
        }
    }
    for e in &mut entities.list {
        if e.removed {
            continue;
        }
        let (lo, hi, _) = e.body();
        let Some(EntityKind::ExperienceOrb(orb)) = e.phys.as_mut().map(|p| &mut p.kind) else { continue };
        // The player's box inflated by (1, 0.5, 1), as for items.
        let touching = |p: &Player| {
            let pmin = [p.pos[0] - 1.3, p.pos[1] - 0.5, p.pos[2] - 1.3];
            let pmax = [p.pos[0] + 1.3, p.pos[1] + 2.3, p.pos[2] + 1.3];
            (0..3).all(|i| pmin[i] < hi[i] && pmax[i] > lo[i])
        };
        let Some(i) = players.iter().position(|p| !p.disconnected && !p.dead && p.game_mode != 3 && p.take_xp_delay == 0 && touching(p))
        else {
            continue;
        };
        let p = &mut players[i];
        p.take_xp_delay = 2;
        let pkt = entity::take_item_entity(e.id, p.entity_id, 1);
        p.send(pkt.clone());
        let remaining = p.repair_with_xp(orb.value);
        if remaining > 0 {
            p.give_experience_points(remaining);
        }
        orb.count -= 1;
        for v in &e.seen_by {
            if let Ok(j) = players.binary_search_by_key(v, |q| q.conn)
                && j != i
            {
                players[j].send(pkt.clone());
            }
        }
        if orb.count <= 0 {
            e.removed = true;
            if let Some(phys) = e.phys.as_mut() {
                phys.discard();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn levels_follow_the_vanilla_curve() {
        // Points to reach levels 1, 16, 31 from zero (vanilla's cumulative table).
        let total = |levels: i32| -> i32 {
            (0..levels)
                .map(|l| if l >= 30 { 112 + (l - 30) * 9 } else if l >= 15 { 37 + (l - 15) * 5 } else { 7 + l * 2 })
                .sum()
        };
        assert_eq!((total(1), total(16), total(31)), (7, 352, 1507));
    }
}
