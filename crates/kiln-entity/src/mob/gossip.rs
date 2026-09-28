//! Villager gossip (`GossipContainer`, `GossipType`): what a villager remembers about each
//! entity (by UUID), as five kinds of gossip with a weight, a cap and a daily decay. The
//! weighted sum is the entity's reputation, which prices trades for players
//! (`Villager.updateSpecialPrices`).

use kiln_proto::nbt::Tag;

/// `GossipType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GossipType {
    MajorNegative,
    MinorNegative,
    MinorPositive,
    MajorPositive,
    Trading,
}

impl GossipType {
    pub const ALL: [GossipType; 5] =
        [GossipType::MajorNegative, GossipType::MinorNegative, GossipType::MinorPositive, GossipType::MajorPositive, GossipType::Trading];

    /// (id, weight, max, decay per day, decay per transfer).
    pub fn info(self) -> (&'static str, i32, i32, i32, i32) {
        match self {
            GossipType::MajorNegative => ("major_negative", -5, 100, 10, 10),
            GossipType::MinorNegative => ("minor_negative", -1, 200, 20, 20),
            GossipType::MinorPositive => ("minor_positive", 1, 25, 1, 5),
            GossipType::MajorPositive => ("major_positive", 5, 20, 0, 20),
            GossipType::Trading => ("trading", 1, 25, 2, 20),
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn by_name(name: &str) -> Option<GossipType> {
        GossipType::ALL.into_iter().find(|t| t.info().0 == name)
    }
}

/// `GossipContainer`: per target, the value of each gossip type (0: none).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gossips {
    pub entries: Vec<(u128, [i32; 5])>,
}

impl Gossips {
    fn get_or_create(&mut self, target: u128) -> &mut [i32; 5] {
        let i = match self.entries.iter().position(|(t, _)| *t == target) {
            Some(i) => i,
            None => {
                self.entries.push((target, [0; 5]));
                self.entries.len() - 1
            }
        };
        &mut self.entries[i].1
    }

    fn drop_empty(&mut self) {
        self.entries.retain(|(_, v)| v.iter().any(|&x| x != 0));
    }

    /// `add(target, type, amount)`: summed up to the type's cap (a value already over it
    /// stays), values below 2 forgotten.
    pub fn add(&mut self, target: u128, t: GossipType, amount: i32) {
        let (_, _, max, _, _) = t.info();
        let v = self.get_or_create(target);
        let old = v[t.index()];
        let sum = old + amount;
        let mut new = if old == 0 { amount } else if sum > max { max.max(old) } else { sum };
        // `makeSureValueIsntTooLowOrTooHigh`.
        if new > max {
            new = max;
        }
        if new < 2 {
            new = 0;
        }
        v[t.index()] = new;
        self.drop_empty();
    }

    /// `getReputation(entity, all types)`: the weighted sum.
    pub fn reputation(&self, target: u128) -> i32 {
        self.entries
            .iter()
            .find(|(t, _)| *t == target)
            .map_or(0, |(_, v)| GossipType::ALL.iter().map(|t| v[t.index()] * t.info().1).sum())
    }

    /// `decay`: each value loses its daily decay; below 2 it is forgotten.
    pub fn decay(&mut self) {
        for (_, v) in self.entries.iter_mut() {
            for t in GossipType::ALL {
                let x = v[t.index()];
                if x != 0 {
                    let n = x - t.info().3;
                    v[t.index()] = if n < 2 { 0 } else { n };
                }
            }
        }
        self.drop_empty();
    }

    /// `putAll`: the other container's values replace these, type by type.
    pub fn put_all(&mut self, other: &Gossips) {
        for (target, vals) in &other.entries {
            let v = self.get_or_create(*target);
            for (i, &x) in vals.iter().enumerate() {
                if x != 0 {
                    v[i] = x;
                }
            }
        }
    }

    /// The saved list (`GossipContainer.CODEC`): `{Target, Type, Value}` entries.
    pub fn save(&self) -> Tag {
        let mut list = Vec::new();
        for (target, v) in &self.entries {
            for t in GossipType::ALL {
                let x = v[t.index()];
                if x > 0 {
                    list.push(Tag::Compound(vec![
                        ("Target".into(), crate::persist::uuid_to_tag(*target)),
                        ("Type".into(), Tag::String(t.info().0.into())),
                        ("Value".into(), Tag::Int(x)),
                    ]));
                }
            }
        }
        Tag::List(list)
    }

    pub fn load(tag: &Tag) -> Gossips {
        let mut g = Gossips::default();
        let Tag::List(list) = tag else { return g };
        for e in list {
            let (Some(target), Some(t), Some(value)) = (
                e.get("Target").and_then(crate::persist::uuid_from_tag),
                e.get("Type").and_then(Tag::as_str).and_then(GossipType::by_name),
                e.get("Value").and_then(Tag::as_f64).map(|v| v as i32).filter(|&v| v > 0),
            ) else {
                continue;
            };
            g.get_or_create(target)[t.index()] = value;
        }
        g
    }
}

/// `ReputationEventType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReputationEvent {
    ZombieVillagerCured,
    Trade,
    VillagerHurt,
    VillagerKilled,
}

impl Gossips {
    /// `Villager.onReputationEventFrom`.
    pub fn on_event(&mut self, event: ReputationEvent, source: u128) {
        match event {
            ReputationEvent::ZombieVillagerCured => {
                self.add(source, GossipType::MajorPositive, 20);
                self.add(source, GossipType::MinorPositive, 25);
            }
            ReputationEvent::Trade => self.add(source, GossipType::Trading, 2),
            ReputationEvent::VillagerHurt => self.add(source, GossipType::MinorNegative, 25),
            ReputationEvent::VillagerKilled => self.add(source, GossipType::MajorNegative, 25),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cure_reputation_and_caps() {
        let mut g = Gossips::default();
        g.on_event(ReputationEvent::ZombieVillagerCured, 7);
        // 20 * 5 + 25 * 1.
        assert_eq!(g.reputation(7), 125);
        g.on_event(ReputationEvent::ZombieVillagerCured, 7);
        assert_eq!(g.reputation(7), 125, "both at their caps");
        g.decay();
        // Major positive never decays; minor positive loses 1.
        assert_eq!(g.reputation(7), 124);
        let tag = g.save();
        assert_eq!(Gossips::load(&tag), g);
        g.add(9, GossipType::Trading, 1);
        assert_eq!(g.reputation(9), 0, "below 2 is forgotten");
    }
}
