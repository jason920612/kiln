//! The regionizer (design §4.5): groups occupied cells into regions that tick in parallel.
//!
//! A generic library with no dependency on the simulation, world or network crates.
//!
//! # Model
//!
//! - A **cell** ([`CellPos`]) is 8×8 chunks ([`CELL_SHIFT`] = 3). Cells are ordered by
//!   Morton key; a region keeps its cells in that order and its **anchor** is the smallest.
//! - Two occupied cells are **linked** when their Chebyshev distance is at most
//!   [`RegionPolicy::link_cheb`] (default 2). A region is a connected component of links
//!   and active fusion pins, except that splitting is lazy (below), so a region may
//!   temporarily hold several components. Different regions are never linked, so their
//!   loaded chunks are at least `link_cheb` whole cells (256 blocks) apart.
//! - A [`Region<C, P>`] owns its cells' payloads (`Box<C>`, never moved on merge or
//!   split, only the pointer is) and one per-region part `P: RegionPart` (tick order,
//!   inbox, counters, ... as a tuple). [`Regions`] holds the [`CellTable`] (cell → owner)
//!   and the live regions in id order. [`RegionId`]s are monotonic and never reused.
//!
//! # Rules ([`Regionizer::apply`])
//!
//! - **Occupy**: the owners of occupied cells in the (2·link+1)² neighbourhood all merge,
//!   immediately. The survivor is the region with the **smallest anchor**; the others are
//!   absorbed in anchor order. No owner: a new region is created.
//! - **Vacate**: the cell leaves its region (the region dies with its last cell) and the
//!   region is marked for a split check. Pins touching the cell are dropped.
//! - **Split** (optional, lazy): a region is checked at ticks where
//!   `tick % split_period == id % split_period` if a cell was vacated or a pin removed
//!   since its last check, or the last check found it disconnected. Components other
//!   than the anchor's split off, each into a new region, once they have been apart for
//!   `split_hysteresis` ticks as sampled by the checks: a component's clock starts at the
//!   first check that sees it apart and restarts whenever a check finds it connected to
//!   another part again. A pin counts as a link while it is active, so its expiry starts
//!   the clock. Components are numbered by anchor and new ids are allocated in that
//!   order; the original region keeps its id and anchor.
//! - **Fuse**: a [`FusePin`] merges the owners of its two cells (smallest anchor survives)
//!   and keeps them in one region until `until_tick`, even though they are not linked.
//!
//! Decisions depend only on the tick counter, the event queue and the cell sets: never on
//! wall-clock time or hash-map iteration order, so topology is replayable.
//!
//! # Integration contract
//!
//! - The sim calls [`Regionizer::apply`] **once per tick in the serial B0 phase**, holding
//!   `&mut Regions`, so no region is ticking. Topology never changes anywhere else.
//! - Between applies, the sim queues events with [`Regionizer::push`]:
//!   - `Occupied(cell)` when a chunk becomes region-accessible (FULL) in a cell without an
//!     owner. The chunk stays in the dimension inbox; after `apply` the cell has an owner
//!     (payload from [`CellHooks::create`]) and the next L0 installs the chunk there.
//!   - `Vacated(cell)` when the last region-accessible chunk of a cell unloads, after the
//!     owning region removed every part element bound to the cell (entities, messages).
//!     The payload comes back through [`CellHooks::retire`].
//!   - `Fuse(pin)` from the reach reports and conflict detectors; `Expire(reason)` to drop
//!     pins early. Pin events are applied in queue order, so queue them in a deterministic
//!     order (e.g. by source `MsgKey`). Cell events are reduced to the last event per cell
//!     and applied in cell order, so their queue order across cells does not matter.
//! - `apply` returns [`TopologyDelta`]s in the order they happened. The sim updates
//!   whatever it keeps per region outside [`Region`] (plugin instances, cost EMA,
//!   metrics): `Merged` → drop `from`, `Split` → create `into`, `Dead` → drop.
//! - During the tick, [`Regions::split_mut`] yields the shared `&CellTable` and disjoint
//!   `&mut Region`s for the workers. A region reaches its payloads through
//!   [`Region::cells_mut`] and its state through [`Region::part_mut`].
//! - Per-region state implements [`RegionPart`] (`Default` is the state of a new region):
//!   `merge` must be a linear merge in a key order independent of the partition (global
//!   `TickSeq`, [`MsgKey`], ...), `split` a stable partition by the new owner of each
//!   element's cell. [`TickList`], [`Inbox`] and [`MaxCounters`] are reference
//!   implementations; tuples of parts are parts.
//! - Debug builds check the invariants ([`Regionizer::check_invariants`]) and the
//!   conservation of part elements after every `apply`.
//! - [`RegionPolicy::unified`] keeps exactly one region per dimension (vanilla profile).

mod cell;
mod part;
mod region;
mod regionizer;

pub use cell::{CELL_BLOCKS, CELL_SHIFT, CellHashBuilder, CellMap, CellPos, CellTable};
pub use part::{Inbox, Letter, MaxCounters, MsgKey, RegionPart, TickEntry, TickList};
pub use region::{CellSet, Region, RegionId, Regions};
pub use regionizer::{
    CellHooks, DefaultCells, Deltas, FusePin, FuseReason, LINK_CHEB, MIN_GAP_BLOCKS, RegionPolicy, Regionizer,
    SPLIT_HYSTERESIS, SPLIT_PERIOD, TopologyDelta, TopologyEvent,
};
pub use smallvec;
