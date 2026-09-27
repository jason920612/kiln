//! `setblock`, `fill` and `clone`, with the loaded-position checks, volume limit
//! (`max_block_modifications`) and update flags of `SetBlockCommand`, `FillCommand` and
//! `CloneCommands`.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::blocks::{BlockInput, BlockPredicate, UpdateFlags};
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::tr;
use crate::types::Identifier;
use kiln_data::blocks::default_state::{AIR, BARRIER};
use kiln_data::blocks_types::is_air;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// `BoundingBox.fromCorners`: inclusive bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl BlockBox {
    pub fn from_corners(a: [i32; 3], b: [i32; 3]) -> Self {
        BlockBox { min: std::array::from_fn(|i| a[i].min(b[i])), max: std::array::from_fn(|i| a[i].max(b[i])) }
    }

    /// `getLength`: `max - min` per axis.
    pub fn length(&self) -> [i32; 3] {
        std::array::from_fn(|i| self.max[i] - self.min[i])
    }

    /// `getXSpan * getYSpan * getZSpan` as a long.
    pub fn volume(&self) -> i64 {
        (0..3).map(|i| self.max[i] as i64 - self.min[i] as i64 + 1).product()
    }

    pub fn intersects(&self, o: &BlockBox) -> bool {
        (0..3).all(|i| self.max[i] >= o.min[i] && self.min[i] <= o.max[i])
    }

    /// Whether `pos` lies on the box's surface (`fill outline`/`hollow`).
    pub fn on_surface(&self, pos: [i32; 3]) -> bool {
        (0..3).any(|i| pos[i] == self.min[i] || pos[i] == self.max[i])
    }

    /// Positions with x varying fastest, then y, then z (`BlockPos.betweenClosed`).
    pub fn positions(&self) -> impl Iterator<Item = [i32; 3]> + use<> {
        let (min, max) = (self.min, self.max);
        (min[2]..=max[2])
            .flat_map(move |z| (min[1]..=max[1]).flat_map(move |y| (min[0]..=max[0]).map(move |x| [x, y, z])))
    }
}

/// `Level.isInWorldBounds`.
pub(super) fn in_world_bounds<S: Host>(s: &S, dimension: &str, pos: [i32; 3]) -> bool {
    let (min_y, max_y) = s.build_height(dimension);
    (min_y..max_y).contains(&pos[1])
        && (-30_000_000..30_000_000).contains(&pos[0])
        && (-30_000_000..30_000_000).contains(&pos[2])
}

/// `BlockPosArgument.getLoadedBlockPos(ctx, level, name)`.
pub(super) fn loaded_block_pos<S: Host>(c: &CommandContext<S>, s: &S, name: &str, dimension: &str) -> Result<[i32; 3]> {
    let pos = s.stack().resolve_block(c.coordinates(name));
    if !s.is_chunk_loaded(dimension, pos[0] >> 4, pos[2] >> 4) {
        return Err(CommandError::pos_unloaded());
    }
    if !in_world_bounds(s, dimension, pos) {
        return Err(CommandError::pos_out_of_world());
    }
    Ok(pos)
}

/// `LevelReader.hasChunksAt(from, to)`: corners as given (not sorted), like vanilla.
pub(super) fn has_chunks_at<S: Host>(s: &S, dimension: &str, from: [i32; 3], to: [i32; 3]) -> bool {
    let (min_y, max_y) = s.build_height(dimension);
    if to[1] < min_y || from[1] > max_y - 1 {
        return false;
    }
    (from[0] >> 4..=to[0] >> 4).all(|cx| (from[2] >> 4..=to[2] >> 4).all(|cz| s.is_chunk_loaded(dimension, cx, cz)))
}

/// `DimensionArgument.getDimension`.
pub(super) fn dimension_arg<S: Host>(c: &CommandContext<S>, s: &S, name: &str) -> Result<String> {
    let id: &Identifier = c.identifier(name);
    if s.has_dimension(id.as_str()) {
        Ok(id.to_string())
    } else {
        Err(CommandError::unknown_dimension(id.as_str()))
    }
}

fn block_limit<S: Host>(s: &S) -> i64 {
    s.game_rule("minecraft:max_block_modifications").command_result() as i64
}

/// Tests a predicate against the block at `pos`, reading block entity data only when needed.
pub(super) fn test_block<S: Host>(s: &mut S, dimension: &str, pos: [i32; 3], predicate: &BlockPredicate) -> bool {
    let state = s.block_state(dimension, pos);
    let entity = if predicate.requires_nbt() { s.block_entity(dimension, pos) } else { None };
    predicate.test(state, entity.as_ref())
}

/// `BlockInput.place`: the host adapts the state to its neighbours unless the flags say
/// `KNOWN_SHAPE`, then vanilla re-applies the explicitly given properties.
fn place<S: Host>(s: &mut S, dimension: &str, pos: [i32; 3], block: &BlockInput, flags: UpdateFlags) -> bool {
    s.place_block(dimension, pos, block, flags)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SetMode {
    Replace,
    Keep,
    Destroy,
}

pub fn setblock<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let run = |mode: SetMode, strict: bool| move |c: &CommandContext<S>, s: &mut S| set_block(c, s, mode, strict);
    d.register(
        literal("setblock").requires(LEVEL_GAMEMASTERS).then(
            argument("pos", ArgumentType::BlockPos).then(
                argument("block", ArgumentType::BlockState)
                    .executes(run(SetMode::Replace, false))
                    .then(literal("destroy").executes(run(SetMode::Destroy, false)))
                    .then(literal("keep").executes(run(SetMode::Keep, false)))
                    .then(literal("replace").executes(run(SetMode::Replace, false)))
                    .then(literal("strict").executes(run(SetMode::Replace, true))),
            ),
        ),
    );
}

/// `SetBlockCommand.setBlock`.
fn set_block<S: Host>(c: &CommandContext<S>, s: &mut S, mode: SetMode, strict: bool) -> Result<i32> {
    let failed = || CommandError::new(tr!("commands.setblock.failed"));
    let dimension = s.dimension().to_owned();
    let pos = loaded_block_pos(c, s, "pos", &dimension)?;
    let block = c.block_state("block");
    if mode == SetMode::Keep && !is_air(s.block_state(&dimension, pos)) {
        return Err(failed());
    }
    let should_place = if mode == SetMode::Destroy {
        s.destroy_block(&dimension, pos, true);
        !(is_air(block.state) && is_air(s.block_state(&dimension, pos)))
    } else {
        true
    };
    let old = s.block_state(&dimension, pos);
    if should_place && !place(s, &dimension, pos, block, UpdateFlags::placement(strict)) {
        return Err(failed());
    }
    if !strict {
        s.update_neighbours(&dimension, pos, old);
    }
    let [x, y, z] = pos;
    s.send_success(tr!("commands.setblock.success", x, y, z), true);
    Ok(1)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FillMode {
    Replace,
    Outline,
    Hollow,
    Destroy,
}

type FilterFn<S> = for<'a, 'b> fn(&'b CommandContext<'a, S>) -> Option<&'a BlockPredicate>;

pub fn fill<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let with_modes = |b: Builder<S>, filter: FilterFn<S>| {
        let run = move |mode: FillMode, strict: bool| {
            move |c: &CommandContext<S>, s: &mut S| fill_blocks(c, s, mode, Filter::Predicate(filter(c)), strict)
        };
        b.executes(run(FillMode::Replace, false))
            .then(literal("outline").executes(run(FillMode::Outline, false)))
            .then(literal("hollow").executes(run(FillMode::Hollow, false)))
            .then(literal("destroy").executes(run(FillMode::Destroy, false)))
            .then(literal("strict").executes(run(FillMode::Replace, true)))
    };
    let block = with_modes(argument("block", ArgumentType::BlockState), |_| None)
        .then(
            literal("replace")
                .executes(|c, s: &mut S| fill_blocks(c, s, FillMode::Replace, Filter::Predicate(None), false))
                .then(with_modes(argument("filter", ArgumentType::BlockPredicate), |c| Some(c.block_predicate("filter")))),
        )
        .then(literal("keep").executes(|c, s: &mut S| fill_blocks(c, s, FillMode::Replace, Filter::Empty, false)));
    d.register(
        literal("fill")
            .requires(LEVEL_GAMEMASTERS)
            .then(argument("from", ArgumentType::BlockPos).then(argument("to", ArgumentType::BlockPos).then(block))),
    );
}

enum Filter<'a> {
    Predicate(Option<&'a BlockPredicate>),
    /// `keep`: only air (`LevelReader.isEmptyBlock`).
    Empty,
}

/// `FillCommand.fillBlocks`.
fn fill_blocks<S: Host>(c: &CommandContext<S>, s: &mut S, mode: FillMode, filter: Filter, strict: bool) -> Result<i32> {
    let dimension = s.dimension().to_owned();
    let from = loaded_block_pos(c, s, "from", &dimension)?;
    let to = loaded_block_pos(c, s, "to", &dimension)?;
    let area = BlockBox::from_corners(from, to);
    let volume = area.volume();
    let limit = block_limit(s);
    if volume > limit {
        return Err(CommandError::new(tr!("commands.fill.toobig", limit as i32, volume)));
    }
    let block = c.block_state("block");
    let air = BlockInput { state: AIR, properties: Vec::new(), nbt: None };
    let flags = UpdateFlags::placement(strict);
    let mut updated = Vec::new();
    let mut count = 0;
    for pos in area.positions() {
        let pass = match &filter {
            Filter::Predicate(Some(p)) => test_block(s, &dimension, pos, p),
            Filter::Predicate(None) => true,
            Filter::Empty => is_air(s.block_state(&dimension, pos)),
        };
        if !pass {
            continue;
        }
        let old = s.block_state(&dimension, pos);
        let affected = mode == FillMode::Destroy && s.destroy_block(&dimension, pos, true);
        let to_place = match mode {
            FillMode::Replace | FillMode::Destroy => Some(block),
            FillMode::Outline => area.on_surface(pos).then_some(block),
            FillMode::Hollow => Some(if area.on_surface(pos) { block } else { &air }),
        };
        let placed = to_place.is_some_and(|b| place(s, &dimension, pos, b, flags));
        if !placed {
            if affected {
                count += 1;
            }
            continue;
        }
        if !strict {
            updated.push((pos, old));
        }
        count += 1;
    }
    for (pos, old) in updated {
        s.update_neighbours(&dimension, pos, old);
    }
    if count == 0 {
        return Err(CommandError::new(tr!("commands.fill.failed")));
    }
    s.send_success(tr!("commands.fill.success", count), true);
    Ok(count)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CloneMode {
    Normal,
    Force,
    Move,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mask {
    Replace,
    Masked,
    Filtered,
}

type DimFn<S> = fn(&CommandContext<S>, &S) -> Result<String>;

pub fn clone<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let source_here: DimFn<S> = |_, s| Ok(s.dimension().to_owned());
    let source_given: DimFn<S> = |c, s| dimension_arg(c, s, "sourceDimension");
    d.register(
        literal("clone")
            .requires(LEVEL_GAMEMASTERS)
            .then(begin_end_destination(source_here))
            .then(literal("from").then(argument("sourceDimension", ArgumentType::Dimension).then(begin_end_destination(source_given)))),
    );
}

/// `beginEndDestinationAndModeSuffix`.
fn begin_end_destination<S: Host + 'static>(source: DimFn<S>) -> Builder<S> {
    let target_same: DimFn<S> = |_, s| Ok(s.dimension().to_owned());
    let target_given: DimFn<S> = |c, s| dimension_arg(c, s, "targetDimension");
    argument("begin", ArgumentType::BlockPos).then(
        argument("end", ArgumentType::BlockPos)
            .then(destination_and_strict(source, target_same))
            .then(literal("to").then(
                argument("targetDimension", ArgumentType::Dimension).then(destination_and_strict(source, target_given)),
            )),
    )
}

/// `destinationAndStrictSuffix`.
fn destination_and_strict<S: Host + 'static>(source: DimFn<S>, target: DimFn<S>) -> Builder<S> {
    mode_suffix(source, target, false, argument("destination", ArgumentType::BlockPos))
        .then(mode_suffix(source, target, true, literal("strict")))
}

/// `modeSuffix`: the default (replace, normal) and the `replace`/`masked`/`filtered` masks.
fn mode_suffix<S: Host + 'static>(source: DimFn<S>, target: DimFn<S>, strict: bool, b: Builder<S>) -> Builder<S> {
    b.executes(move |c, s: &mut S| clone_blocks(c, s, source, target, Mask::Replace, CloneMode::Normal, strict))
        .then(with_clone_modes(source, target, Mask::Replace, strict, literal("replace")))
        .then(with_clone_modes(source, target, Mask::Masked, strict, literal("masked")))
        .then(literal("filtered").then(with_clone_modes(
            source,
            target,
            Mask::Filtered,
            strict,
            argument("filter", ArgumentType::BlockPredicate),
        )))
}

/// `wrapWithCloneMode`.
fn with_clone_modes<S: Host + 'static>(
    source: DimFn<S>,
    target: DimFn<S>,
    mask: Mask,
    strict: bool,
    b: Builder<S>,
) -> Builder<S> {
    let run = move |mode| move |c: &CommandContext<S>, s: &mut S| clone_blocks(c, s, source, target, mask, mode, strict);
    b.executes(run(CloneMode::Normal))
        .then(literal("force").executes(run(CloneMode::Force)))
        .then(literal("move").executes(run(CloneMode::Move)))
        .then(literal("normal").executes(run(CloneMode::Normal)))
}

/// `CloneCommands.clone`.
fn clone_blocks<S: Host>(
    c: &CommandContext<S>,
    s: &mut S,
    source: DimFn<S>,
    target: DimFn<S>,
    mask: Mask,
    mode: CloneMode,
    strict: bool,
) -> Result<i32> {
    let src_dim = source(c, s)?;
    let begin = loaded_block_pos(c, s, "begin", &src_dim)?;
    let end = loaded_block_pos(c, s, "end", &src_dim)?;
    let dst_dim = target(c, s)?;
    let dest = loaded_block_pos(c, s, "destination", &dst_dim)?;
    let filter = (mask == Mask::Filtered).then(|| c.block_predicate("filter"));
    let src = BlockBox::from_corners(begin, end);
    let len = src.length();
    let dest_end = std::array::from_fn(|i| dest[i] + len[i]);
    let dst = BlockBox::from_corners(dest, dest_end);
    if mode == CloneMode::Normal && src_dim == dst_dim && dst.intersects(&src) {
        return Err(CommandError::new(tr!("commands.clone.overlap")));
    }
    let volume = src.volume();
    let limit = block_limit(s);
    if volume > limit {
        return Err(CommandError::new(tr!("commands.clone.toobig", limit as i32, volume)));
    }
    if !has_chunks_at(s, &src_dim, begin, end) || !has_chunks_at(s, &dst_dim, dest, dest_end) {
        return Err(CommandError::pos_unloaded());
    }
    let offset: [i32; 3] = std::array::from_fn(|i| dst.min[i] - src.min[i]);
    // Everything is read before anything is written, so overlapping copies are exact.
    let mut copies: Vec<CloneBlock> = Vec::new();
    let mut previous = Vec::new();
    for pos in src.positions() {
        let state = s.block_state(&src_dim, pos);
        let keep = match mask {
            Mask::Replace => true,
            Mask::Masked => !is_air(state),
            Mask::Filtered => filter.is_some_and(|f| {
                let entity = if f.requires_nbt() { s.block_entity(&src_dim, pos) } else { None };
                f.test(state, entity.as_ref())
            }),
        };
        if keep {
            let to: [i32; 3] = std::array::from_fn(|i| pos[i] + offset[i]);
            previous.push((to, s.block_state(&dst_dim, to)));
            copies.push(CloneBlock { from: pos, to, state, entity: s.block_entity(&src_dim, pos) });
        }
    }
    let flags = UpdateFlags(UpdateFlags::CLIENTS | if strict { UpdateFlags::STRICT } else { 0 });
    if mode == CloneMode::Move {
        let remove_flags = if strict { flags } else { UpdateFlags::ALL };
        for &CloneBlock { from, .. } in &copies {
            s.set_block(&src_dim, from, AIR, None, remove_flags);
        }
    }
    let mut count = 0;
    for CloneBlock { to, state, entity, .. } in &copies {
        if !in_world_bounds(s, &dst_dim, *to) {
            continue;
        }
        s.set_block(&dst_dim, *to, *state, entity.as_ref(), flags);
        // Vanilla fills the destination with barriers first, so every block but a barrier
        // counts as changed.
        if *state != BARRIER {
            count += 1;
        }
    }
    if !strict {
        previous.retain(|(pos, _)| in_world_bounds(s, &dst_dim, *pos));
        for (pos, old) in previous.into_iter().rev() {
            s.update_neighbours(&dst_dim, pos, old);
        }
    }
    if count == 0 {
        return Err(CommandError::new(tr!("commands.clone.failed")));
    }
    s.send_success(tr!("commands.clone.success", count), true);
    Ok(count)
}

/// `CloneCommands.CloneBlockInfo`: a block to copy, read before anything is written.
struct CloneBlock {
    from: [i32; 3],
    to: [i32; 3],
    state: u16,
    entity: Option<Tag>,
}
