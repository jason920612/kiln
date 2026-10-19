//! Chunk tickets other than the players' (`TicketType`, `TicketStorage`, `ServerChunkCache.addTicketWithRadius`): what keeps a chunk and the ring
//! around it loaded and ticking without a player nearby.
//!
//! A ticket of radius `R` puts the chunk at level `33 - R` and each chunk further at one more (`Ticket.getTicketLevel`, the trackers' propagation
//! over the eight neighbours: square rings). Level 31 is entity ticking, 32 block ticking, 33 "full" (loaded, nothing ticks):
//!
//! | ticket | radius | timeout | entity ticking | block ticking | loaded |
//! |---|---|---|---|---|---|
//! | `forced` (`/forceload`) | 2 (level 31) | none | the chunk | + 1 | + 2 |
//! | `ender_pearl` (`ServerPlayer.placeEnderPearlTicket`) | 2 | 40 ticks | the chunk | + 1 | + 2 |
//! | `portal` (`Entity.placePortalTicket`) | 3 | 300 ticks | + 1 | + 2 | + 3 |
//! | `dragon` (`EnderDragonFight`) | 9 | none | + 7 | + 8 | + 9 |
//!
//! A timed ticket counts down once per tick (`TicketStorage.purgeStaleTickets`, before the level ticks) while its chunk is loaded, and goes when
//! the count passes below zero: a ticket renewed at tick `t` works through tick `t + 40`. Adding the same ticket (type and level) again at the same
//! chunk renews it.

use kiln_world::ChunkPos;
use kiln_proto::nbt::Tag;

/// The radius of `/forceload`'s ticket (`ChunkMap.FORCED_TICKET_LEVEL` is level 31 = radius 2).
pub(crate) const FORCED_RADIUS: i32 = 2;
/// `ServerPlayer.ENDER_PEARL_TICKET_RADIUS`.
pub(crate) const PEARL_RADIUS: i32 = 2;
/// `TicketType.ENDER_PEARL` timeout.
pub(crate) const PEARL_TIMEOUT: i64 = 40;
/// `Entity.placePortalTicket` (`addTicketWithRadius(PORTAL, …, 3)`).
pub(crate) const PORTAL_RADIUS: i32 = 3;
/// `TicketType.PORTAL` timeout.
pub(crate) const PORTAL_TIMEOUT: i64 = 300;
/// `EnderDragonFight.ARENA_TICKET_LEVEL`.
pub(crate) const DRAGON_RADIUS: i32 = 9;

/// A ticket with a timeout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Portal,
    EnderPearl,
}

impl Kind {
    fn radius(self) -> i32 {
        match self {
            Kind::Portal => PORTAL_RADIUS,
            Kind::EnderPearl => PEARL_RADIUS,
        }
    }

    fn timeout(self) -> i64 {
        match self {
            Kind::Portal => PORTAL_TIMEOUT,
            Kind::EnderPearl => PEARL_TIMEOUT,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Portal => "minecraft:portal",
            Kind::EnderPearl => "minecraft:ender_pearl",
        }
    }

    /// `TicketType.persist()`: only the portal's ticket is saved.
    fn persists(self) -> bool {
        matches!(self, Kind::Portal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub kind: Kind,
    pub chunk: ChunkPos,
    pub ticks_left: i64,
}

impl Ticket {
    pub(crate) fn radius(&self) -> i32 {
        self.kind.radius()
    }
}

/// One level's timed tickets.
#[derive(Clone, Debug, Default)]
pub(crate) struct Timed {
    list: Vec<Ticket>,
}

impl Timed {
    /// `TicketStorage.addTicket`: a ticket of the same type and level at the chunk is renewed, else a new one starts.
    pub(crate) fn add(&mut self, kind: Kind, chunk: ChunkPos) {
        match self.list.iter_mut().find(|t| t.kind == kind && t.chunk == chunk) {
            Some(t) => t.ticks_left = kind.timeout(),
            None => self.list.push(Ticket { kind, chunk, ticks_left: kind.timeout() }),
        }
    }

    /// `purgeStaleTickets`: tickets whose chunk is loaded count down (`canTicketExpire`), and go below zero.
    pub(crate) fn purge(&mut self, loaded: impl Fn(ChunkPos) -> bool) {
        self.list.retain_mut(|t| {
            if loaded(t.chunk) {
                t.ticks_left -= 1;
            }
            t.ticks_left >= 0
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }

    /// (centre, radius) of every ticket.
    pub(crate) fn rings(&self) -> impl Iterator<Item = (ChunkPos, i32)> + '_ {
        self.list.iter().map(|t| (t.chunk, t.radius()))
    }

    #[cfg(test)]
    pub(crate) fn tickets(&self) -> &[Ticket] {
        &self.list
    }

    /// The saved form (`chunk_tickets.dat`'s entries) of the tickets that persist.
    pub(crate) fn to_nbt(&self) -> Vec<Tag> {
        self.list
            .iter()
            .filter(|t| t.kind.persists())
            .map(|t| {
                Tag::Compound(vec![
                    ("type".into(), Tag::String(t.kind.name().into())),
                    ("level".into(), Tag::Int(33 - t.radius())),
                    ("ticks_left".into(), Tag::Long(t.ticks_left)),
                    ("chunk_pos".into(), Tag::IntArray(vec![t.chunk.x, t.chunk.z])),
                ])
            })
            .collect()
    }

    /// One saved entry (`Ticket.CODEC` with its chunk position) back; other types are not ours.
    pub(crate) fn load(&mut self, tag: &Tag) -> bool {
        let kind = match tag.get("type").and_then(Tag::as_str) {
            Some("minecraft:portal") => Kind::Portal,
            _ => return false,
        };
        let Some(Tag::IntArray(p)) = tag.get("chunk_pos") else { return false };
        let [x, z] = p[..] else { return false };
        let ticks_left = tag.get("ticks_left").and_then(Tag::as_i64).unwrap_or(0);
        self.list.push(Ticket { kind, chunk: ChunkPos::new(x, z), ticks_left });
        true
    }
}

/// Every chunk within `r` of `centre` (the loaded ring of a ticket).
pub(crate) fn ring_chunks(centre: ChunkPos, r: i32) -> impl Iterator<Item = ChunkPos> {
    (centre.x - r..=centre.x + r).flat_map(move |x| (centre.z - r..=centre.z + r).map(move |z| ChunkPos::new(x, z)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pearl_ticket_works_through_forty_ticks_after_its_renewal() {
        let mut t = Timed::default();
        let c = ChunkPos::new(3, 4);
        t.add(Kind::EnderPearl, c);
        // (the tick after the renewal purges first: 39 … 0 are the ticks 1 … 40, -1 the 41st.)
        for tick in 1..=40 {
            t.purge(|_| true);
            assert!(!t.is_empty(), "still there on tick {tick}");
        }
        t.purge(|_| true);
        assert!(t.is_empty(), "gone on the 41st");
    }

    #[test]
    fn renewing_resets_the_count_and_does_not_add_a_second_ticket() {
        let mut t = Timed::default();
        let c = ChunkPos::new(0, 0);
        t.add(Kind::EnderPearl, c);
        for _ in 0..39 {
            t.purge(|_| true);
        }
        t.add(Kind::EnderPearl, c);
        assert_eq!(t.len(), 1);
        for _ in 0..40 {
            t.purge(|_| true);
        }
        assert!(!t.is_empty());
        t.add(Kind::Portal, c);
        assert_eq!(t.len(), 2, "another type is another ticket");
    }

    #[test]
    fn a_ticket_does_not_count_down_while_its_chunk_is_not_loaded() {
        let mut t = Timed::default();
        t.add(Kind::EnderPearl, ChunkPos::new(0, 0));
        for _ in 0..100 {
            t.purge(|_| false);
        }
        assert_eq!(t.tickets()[0].ticks_left, PEARL_TIMEOUT);
    }

    #[test]
    fn only_portal_tickets_are_saved() {
        let mut t = Timed::default();
        t.add(Kind::EnderPearl, ChunkPos::new(0, 0));
        t.add(Kind::Portal, ChunkPos::new(5, -6));
        let saved = t.to_nbt();
        assert_eq!(saved.len(), 1);
        let mut back = Timed::default();
        assert!(back.load(&saved[0]));
        assert_eq!(back.tickets(), &[Ticket { kind: Kind::Portal, chunk: ChunkPos::new(5, -6), ticks_left: PORTAL_TIMEOUT }]);
    }
}
