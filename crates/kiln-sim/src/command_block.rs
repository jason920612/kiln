//! Command blocks (`CommandBlock`, `CommandBlockEntity`, `BaseCommandBlock`): a block that runs a command when it gets
//! power (a command block), every tick (a repeating one) or after the one behind it (a chain one), and counts how
//! often the command succeeded. The blocks' states and the block entity live in the regions; the commands run in the
//! serial phase that follows the regions' tick (a command can change any part of the world), where the chain is walked
//! block by block, each seeing the success count of the one before.

use crate::blocks::RegionLevel;
use crate::commands::CommandSource;
use crate::container::BeKind;
use crate::{DimId, Player, Sim};
use kiln_blocks::{BlockId, BlockPos, Direction, Level, TickPriority, schedule_block_tick, state};
use kiln_command::{Source as _, Text};
use kiln_data::block_logic::{self as logic, BlockClass as C};
use kiln_proto::nbt::Tag;
use std::sync::Arc;

/// `CommandBlockEntity.Mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Sequence,
    Auto,
    Redstone,
}

/// `CommandBlockEntity.getMode`.
pub(crate) fn mode_of(s: u16) -> Mode {
    match BlockId::of(s).name() {
        "minecraft:repeating_command_block" => Mode::Auto,
        "minecraft:chain_command_block" => Mode::Sequence,
        _ => Mode::Redstone,
    }
}

/// `CommandBlock.automatic`: the repeating and the chain blocks start "always active".
pub(crate) fn automatic_default(s: u16) -> bool {
    BlockId::of(s).name() != "minecraft:command_block"
}

/// What a command block entity keeps (`BaseCommandBlock` and the entity's own flags).
#[derive(Debug, Clone)]
pub(crate) struct Data {
    pub command: String,
    pub success_count: i32,
    /// `lastOutput`: the text of the last message, with the time it came.
    pub last_output: Option<Tag>,
    /// `lastOutput` as plain text without its time (for tests; not saved).
    pub last_plain: Option<String>,
    pub track_output: bool,
    pub update_last_execution: bool,
    /// `lastExecution`: the game time of the last run (-1: none).
    pub last_execution: i64,
    pub custom_name: Option<Tag>,
    pub powered: bool,
    pub condition_met: bool,
    /// "Always active".
    pub auto: bool,
}

impl Default for Data {
    fn default() -> Self {
        Data {
            command: String::new(),
            success_count: 0,
            last_output: None,
            last_plain: None,
            track_output: true,
            update_last_execution: true,
            last_execution: -1,
            custom_name: None,
            powered: false,
            condition_met: false,
            auto: false,
        }
    }
}

impl Data {
    /// `CommandBlockEntity.loadAdditional`.
    pub fn load(nbt: &Tag) -> Data {
        let flag = |key: &str, default: bool| nbt.get(key).and_then(Tag::as_i64).map_or(default, |v| v != 0);
        let track_output = flag("TrackOutput", true);
        let update_last_execution = flag("UpdateLastExecution", true);
        Data {
            command: nbt.get("Command").and_then(Tag::as_str).unwrap_or("").to_owned(),
            success_count: nbt.get("SuccessCount").and_then(Tag::as_i64).map_or(0, |v| v as i32),
            last_output: if track_output { nbt.get("LastOutput").cloned() } else { None },
            last_plain: None,
            track_output,
            update_last_execution,
            last_execution: if update_last_execution { nbt.get("LastExecution").and_then(Tag::as_i64).unwrap_or(-1) } else { -1 },
            custom_name: nbt.get("CustomName").cloned(),
            powered: flag("powered", false),
            condition_met: flag("conditionMet", false),
            auto: flag("auto", false),
        }
    }

    /// `CommandBlockEntity.saveAdditional`.
    pub fn save(&self, out: &mut Vec<(String, Tag)>) {
        out.push(("Command".into(), Tag::String(self.command.clone())));
        out.push(("SuccessCount".into(), Tag::Int(self.success_count)));
        if let Some(n) = &self.custom_name {
            out.push(("CustomName".into(), n.clone()));
        }
        out.push(("TrackOutput".into(), Tag::Byte(self.track_output as i8)));
        if self.track_output
            && let Some(o) = &self.last_output
        {
            out.push(("LastOutput".into(), o.clone()));
        }
        out.push(("UpdateLastExecution".into(), Tag::Byte(self.update_last_execution as i8)));
        if self.update_last_execution && self.last_execution != -1 {
            out.push(("LastExecution".into(), Tag::Long(self.last_execution)));
        }
        out.push(("powered".into(), Tag::Byte(self.powered as i8)));
        out.push(("conditionMet".into(), Tag::Byte(self.condition_met as i8)));
        out.push(("auto".into(), Tag::Byte(self.auto as i8)));
    }
}

/// A run in progress: how often the command succeeded, the output it made, whether the block takes it down.
#[derive(Debug, Default)]
pub(crate) struct Run {
    pub success: i32,
    pub output: Option<Tag>,
    pub plain: Option<String>,
    pub track: bool,
}

impl Player {
    /// `Player.canUseGameMasterBlocks`: an operator in creative mode.
    pub(crate) fn can_use_gamemaster_blocks(&self) -> bool {
        self.game_mode == 1 && self.permission >= 2
    }
}

fn data<'a>(level: &'a mut RegionLevel, pos: BlockPos) -> Option<&'a mut Data> {
    level.blocks.containers.get_mut(pos).and_then(|c| c.command.as_deref_mut())
}

/// `CommandBlockEntity.markConditionMet`: a conditional block needs the success of the block behind it.
fn mark_condition_met(level: &mut RegionLevel, pos: BlockPos) -> bool {
    let s = level.block(pos);
    let met = if state::get_bool(s, "conditional") {
        let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
        let behind = pos.relative(facing.opposite());
        logic::is_instance(level.block(behind), C::CommandBlock) && data(level, behind).is_some_and(|d| d.success_count > 0)
    } else {
        true
    };
    if let Some(d) = data(level, pos) {
        d.condition_met = met;
    }
    met
}

/// `CommandBlock.neighborChanged` → `setPoweredAndUpdate`.
pub(crate) fn powered_changed(level: &mut RegionLevel, pos: BlockPos, s: u16, powered: bool) {
    let Some(d) = data(level, pos) else { return };
    if d.powered == powered {
        return;
    }
    d.powered = powered;
    let auto = d.auto;
    if let Some(c) = level.blocks.containers.get_mut(pos) {
        c.mark_changed();
    }
    if !powered || auto || mode_of(s) == Mode::Sequence {
        return;
    }
    mark_condition_met(level, pos);
    schedule_block_tick(level, pos, BlockId::of(s), 1, TickPriority::Normal);
}

/// `CommandBlock.setPlacedBy`: a block put up by a player (the item's own block entity data aside) tracks its output
/// as the rule says and is "always active" as its kind is; then the power it stands in counts.
pub(crate) fn placed_by(level: &mut RegionLevel, pos: BlockPos, powered: bool, has_block_entity_data: bool, loaded: Option<&Tag>) {
    let s = level.block(pos);
    if !logic::is_instance(s, C::CommandBlock) {
        return;
    }
    let feedback = level.env.send_command_feedback;
    let Some(d) = data(level, pos) else { return };
    // `BlockItem.updateCustomBlockEntityTag`: the item's block entity data, for a game master.
    if let Some(tag) = loaded {
        *d = Data::load(tag);
    }
    if !has_block_entity_data {
        d.track_output = feedback;
        d.auto = automatic_default(s);
    }
    powered_changed(level, pos, s, powered);
}

/// `CommandBlock.useWithoutItem`: a game master gets the block's screen (its data), others click through.
pub(crate) fn use_without_item(p: &mut Player, level: &mut RegionLevel, pos: BlockPos) -> Option<bool> {
    if !p.can_use_gamemaster_blocks() {
        return None;
    }
    let c = level.blocks.containers.get(pos)?;
    let mut fields = Vec::new();
    c.command.as_ref()?.save(&mut fields);
    p.send(kiln_proto::packets::block_entity_data([pos.x, pos.y, pos.z], c.type_id as i32, &Tag::Compound(fields)));
    Some(true)
}

/// A new command block entity takes its block's "always active" (`CommandBlock.newBlockEntity`).
pub(crate) fn created(level: &mut RegionLevel, pos: BlockPos) {
    let s = level.block(pos);
    if let Some(d) = data(level, pos) {
        d.auto = automatic_default(s);
    }
}

/// `BaseCommandBlock.getName`'s text as a command source's name: the custom name, else `@`.
pub(crate) fn name_of(sim: &Sim, dim: DimId, pos: [i32; 3]) -> Text {
    let custom = sim.dims[dim]
        .regions
        .at(kiln_world::ChunkPos::of_block(pos[0], pos[2]).cell())
        .and_then(|r| r.part().1.containers.get(BlockPos::new(pos[0], pos[1], pos[2])).and_then(|c| c.command.as_ref().and_then(|d| d.custom_name.clone())));
    match custom {
        Some(Tag::String(s)) => Text::literal(s),
        Some(tag) => Text::raw(tag),
        None => Text::literal("@"),
    }
}

/// The block's `facing` as a yaw (`Direction.toYRot`).
pub(crate) fn facing_yaw(sim: &Sim, dim: DimId, pos: [i32; 3]) -> f32 {
    let s = sim.block_at_in(dim, pos).unwrap_or(0);
    match state::get_dir(s, "facing").unwrap_or(Direction::North) {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        _ => 0.0,
    }
}

/// `CloseableCommandBlockSource.sendSystemMessage`: the message is the block's last output, stamped with the time.
pub(crate) fn output(sim: &mut Sim, text: Text) {
    let Some(run) = sim.commands.block_run.as_mut() else { return };
    if !run.track {
        return;
    }
    // `HH:mm:ss` (UTC: the server's time zone is not known here).
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) % 86400;
    let stamp = format!("[{:02}:{:02}:{:02}] ", secs / 3600, secs / 60 % 60, secs % 60);
    run.plain = Some(crate::commands::console_text(&text));
    run.output = Some(Text::literal(stamp).append(text).to_nbt());
}

/// `CommandSourceStack.sendSuccess` of a command block's source: told when it tracks its output and
/// `send_command_feedback` is on; the operators hear of it with `command_block_output`.
pub(crate) fn success(sim: &mut Sim, text: Text, broadcast: bool) {
    let track = sim.commands.block_run.as_ref().is_some_and(|r| r.track);
    if !track {
        return;
    }
    let inform = sim.rule_bool("minecraft:command_block_output");
    if broadcast && inform {
        // The operators see "[@: message]" (`chat.type.admin`).
        let name = sim.source_name();
        let admin = kiln_command::tr!("chat.type.admin", name, text.clone()).color("gray").italic();
        let pkt = kiln_proto::packets::system_chat(admin.to_nbt(), false);
        let ops: Vec<_> = sim.players.iter().filter(|(_, p)| sim.commands.is_op(&p.name)).map(|(c, _)| *c).collect();
        for c in ops {
            if let Some(p) = sim.players.get_mut(&c) {
                p.send(pkt.clone());
            }
        }
        if sim.rule_bool("minecraft:log_admin_commands") {
            sim.reply_console(&admin);
        }
    }
    if sim.rule_bool("minecraft:send_command_feedback") {
        output(sim, text);
    }
}

impl Sim {
    /// The command blocks whose scheduled tick came, after the regions ticked: in order of place.
    pub(crate) fn run_command_blocks(&mut self) {
        let mut due: Vec<(DimId, [i32; 3])> = Vec::new();
        for dim in 0..self.dims.len() {
            for region in self.dims[dim].regions.iter_mut() {
                for p in std::mem::take(&mut region.part_mut().1.command_ticks) {
                    due.push((dim, [p.x, p.y, p.z]));
                }
            }
        }
        due.sort_unstable();
        due.dedup();
        for (dim, pos) in due {
            self.command_block_tick(dim, pos);
        }
    }

    /// Block work at a position of a level; `None` when its chunk is not loaded.
    fn cb<R>(&mut self, dim: DimId, pos: [i32; 3], f: impl FnOnce(&mut RegionLevel, BlockPos) -> R) -> Option<R> {
        let bp = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level_in(dim, pos, |l| f(l, bp))
    }

    /// `CommandBlock.tick`.
    fn command_block_tick(&mut self, dim: DimId, pos: [i32; 3]) {
        let info = self.cb(dim, pos, |l, bp| {
            let s = l.block(bp);
            if !logic::is_instance(s, C::CommandBlock) {
                return None;
            }
            let d = data(l, bp)?;
            Some((s, !d.command.is_empty(), d.condition_met, d.powered, d.auto))
        });
        let Some(Some((s, has_command, condition_met, _, _))) = info else { return };
        let conditional = state::get_bool(s, "conditional");
        let mode = mode_of(s);
        let bp = BlockPos::new(pos[0], pos[1], pos[2]);
        match mode {
            Mode::Auto => {
                self.cb(dim, pos, |l, bp| mark_condition_met(l, bp));
                if condition_met {
                    self.command_block_execute(dim, pos, s, has_command);
                } else if conditional {
                    self.cb(dim, pos, |l, bp| data(l, bp).map(|d| d.success_count = 0));
                }
                let again = self.cb(dim, pos, |l, bp| data(l, bp).is_some_and(|d| d.powered || d.auto)).unwrap_or(false);
                if again {
                    self.cb(dim, pos, |l, bp| schedule_block_tick(l, bp, BlockId::of(s), 1, TickPriority::Normal));
                }
            }
            Mode::Redstone => {
                if condition_met {
                    self.command_block_execute(dim, pos, s, has_command);
                } else if conditional {
                    self.cb(dim, pos, |l, bp| data(l, bp).map(|d| d.success_count = 0));
                }
            }
            Mode::Sequence => {}
        }
        self.cb(dim, pos, |l, _| kiln_blocks::update::update_neighbour_for_output_signal(l, bp, BlockId::of(s)));
    }

    /// `CommandBlock.execute`: the command runs (or its count is reset), then the chain behind follows.
    fn command_block_execute(&mut self, dim: DimId, pos: [i32; 3], s: u16, can_trigger: bool) {
        if can_trigger {
            self.perform_command(dim, pos);
        } else {
            self.cb(dim, pos, |l, bp| data(l, bp).map(|d| d.success_count = 0));
        }
        let facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
        self.execute_chain(dim, pos, facing);
    }

    /// `CommandBlock.executeChain`.
    fn execute_chain(&mut self, dim: DimId, pos: [i32; 3], mut facing: Direction) {
        let max = self.rule_int("minecraft:max_command_sequence_length");
        let mut remaining = max;
        let mut at = BlockPos::new(pos[0], pos[1], pos[2]);
        loop {
            remaining -= 1;
            if remaining <= 0 {
                tracing::warn!("Command Block chain tried to execute more than {} steps!", max.max(0));
                return;
            }
            at = at.relative(facing);
            let here = [at.x, at.y, at.z];
            let Some(Some(s)) = self.cb(dim, here, |l, bp| {
                let s = l.block(bp);
                (BlockId::of(s).name() == "minecraft:chain_command_block" && data(l, bp).is_some()).then_some(s)
            }) else {
                return;
            };
            let (active, conditional) = self.cb(dim, here, |l, bp| data(l, bp).map(|d| (d.powered || d.auto, state::get_bool(s, "conditional")))).flatten().unwrap_or((false, false));
            if active {
                let met = self.cb(dim, here, |l, bp| mark_condition_met(l, bp)).unwrap_or(false);
                if met {
                    if !self.perform_command(dim, here) {
                        return;
                    }
                    self.cb(dim, here, |l, bp| kiln_blocks::update::update_neighbour_for_output_signal(l, bp, BlockId::of(s)));
                } else if conditional {
                    self.cb(dim, here, |l, bp| data(l, bp).map(|d| d.success_count = 0));
                }
            }
            facing = state::get_dir(s, "facing").unwrap_or(Direction::North);
        }
    }

    /// `BaseCommandBlock.performCommand`: false when it already ran this tick.
    fn perform_command(&mut self, dim: DimId, pos: [i32; 3]) -> bool {
        let now = self.game_time;
        let Some(Some((command, track))) = self.cb(dim, pos, |l, bp| data(l, bp).map(|d| (d.command.clone(), d.track_output))) else { return false };
        let last = self.cb(dim, pos, |l, bp| data(l, bp).map_or(-1, |d| d.last_execution)).unwrap_or(-1);
        if now == last {
            return false;
        }
        if command.eq_ignore_ascii_case("Searge") {
            self.cb(dim, pos, |l, bp| {
                if let Some(d) = data(l, bp) {
                    d.last_output = Some(Tag::String("#itzlipofutzli".into()));
                    d.last_plain = Some("#itzlipofutzli".into());
                    d.success_count = 1;
                }
            });
            return true;
        }
        self.cb(dim, pos, |l, bp| data(l, bp).map(|d| d.success_count = 0));
        if self.config.enable_command_block && !command.is_empty() {
            self.cb(dim, pos, |l, bp| {
                data(l, bp).map(|d| {
                    d.last_output = None;
                    d.last_plain = None;
                })
            });
            let run = self.run_block_command(dim, pos, &command, track);
            self.cb(dim, pos, |l, bp| {
                if let Some(d) = data(l, bp) {
                    d.success_count = run.success;
                    if track {
                        d.last_output = run.output.clone();
                        d.last_plain = run.plain.clone();
                    }
                }
            });
        }
        self.cb(dim, pos, |l, bp| {
            if let Some(d) = data(l, bp) {
                d.last_execution = if d.update_last_execution { now } else { -1 };
            }
            if let Some(c) = l.blocks.containers.get_mut(bp) {
                c.mark_changed();
            }
        });
        true
    }

    /// `Commands.performPrefixedCommand` as the block's source (no entity, permission level 2).
    fn run_block_command(&mut self, dim: DimId, pos: [i32; 3], command: &str, track: bool) -> Run {
        self.commands.block_run = Some(Run { success: 0, output: None, track });
        let source = CommandSource::Block { dim, pos };
        let previous = std::mem::replace(&mut self.commands.source, source);
        let mut start = self.source_stack(source);
        start.callbacks.push(Arc::new(|sim: &mut Sim, success: bool, _| {
            if success && let Some(r) = sim.commands.block_run.as_mut() {
                r.success += 1;
            }
        }));
        let stack = std::mem::replace(&mut self.commands.stack, start);
        let dispatcher = self.commands.dispatcher.clone();
        let command = command.strip_prefix('/').unwrap_or(command);
        if let Err(e) = dispatcher.execute(command, self) {
            for line in e.chat_lines(command) {
                // (`CommandSourceStack.sendFailure`: a block that tracks its output keeps the failure.)
                crate::command_block::output(self, line.color("red"));
            }
        }
        self.commands.source = previous;
        self.commands.stack = stack;
        self.flush_scoreboard();
        self.commands.block_run.take().unwrap_or_default()
    }

    /// `ServerGamePacketListenerImpl.handleSetCommandBlock`: the screen's settings.
    pub(crate) fn set_command_block(&mut self, conn: crate::ConnId, update: &kiln_proto::packets::serverbound::CommandBlockUpdate) {
        use kiln_proto::packets::serverbound::CommandBlockMode;
        let Some(p) = self.players.get_mut(&conn) else { return };
        if !p.can_use_gamemaster_blocks() {
            p.send(kiln_proto::packets::system_chat(crate::container::translatable("advMode.notAllowed"), false));
            return;
        }
        let dim = p.dim;
        let pos = update.pos;
        let bp = BlockPos::new(pos[0], pos[1], pos[2]);
        let enabled = self.config.enable_command_block;
        // The block, the block entity, then the settings (the block entity keeps its data across the block's change).
        let found = self.cb(dim, pos, |l, bp| {
            let s = l.block(bp);
            if !logic::is_instance(s, C::CommandBlock) {
                return None;
            }
            data(l, bp).map(|d| (s, d.clone()))
        });
        let Some(Some((old_state, kept))) = found else { return };
        let old_mode = mode_of(old_state);
        let facing = state::get_dir(old_state, "facing").unwrap_or(Direction::North);
        let name = match update.mode {
            CommandBlockMode::Sequence => "minecraft:chain_command_block",
            CommandBlockMode::Auto => "minecraft:repeating_command_block",
            CommandBlockMode::Redstone => "minecraft:command_block",
        };
        let new_state = state::set_bool(state::set_dir(BlockId::by_name(name).map_or(old_state, |b| b.default_state()), "facing", facing), "conditional", update.conditional);
        let new_mode = mode_of(new_state);
        let (command, track, automatic) = (update.command.clone(), update.track_output, update.automatic);
        self.cb(dim, pos, |l, bp| {
            if new_state != old_state {
                kiln_blocks::set_block(l, bp, new_state, kiln_blocks::flags::CLIENTS);
                // The block entity stays what it was.
                if let Some(c) = l.blocks.containers.get_mut(bp) {
                    c.command = Some(Box::new(kept.clone()));
                }
            }
            let state_now = l.block(bp);
            let Some(d) = data(l, bp) else { return };
            // `BaseCommandBlock.setCommand`.
            d.command = command.clone();
            d.success_count = 0;
            d.track_output = track;
            if !track {
                d.last_output = None;
            }
            // `CommandBlockEntity.setAutomatic`.
            let was_auto = d.auto;
            d.auto = automatic;
            let schedule = (!was_auto && automatic && !d.powered && mode_of(state_now) != Mode::Sequence)
                // `onModeSwitch`.
                || (old_mode != new_mode && new_mode == Mode::Auto && (d.powered || d.auto));
            if schedule {
                mark_condition_met(l, bp);
                schedule_block_tick(l, bp, BlockId::of(state_now), 1, TickPriority::Normal);
            }
            if let Some(c) = l.blocks.containers.get_mut(bp) {
                c.mark_changed();
            }
        });
        let _ = bp;
        if !update.command.is_empty() {
            let key = if enabled { "advMode.setCommand.success" } else { "advMode.setCommand.disabled" };
            let msg = if enabled {
                kiln_command::tr!("advMode.setCommand.success", Text::literal(update.command.clone()))
            } else {
                kiln_command::tr!("advMode.setCommand.disabled", Text::literal(update.command.clone()))
            };
            let _ = key;
            if let Some(p) = self.players.get_mut(&conn) {
                p.send(kiln_proto::packets::system_chat(msg.to_nbt(), false));
            }
        }
    }
}
