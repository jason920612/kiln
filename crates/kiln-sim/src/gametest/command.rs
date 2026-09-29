//! `/test`'s subcommands (vanilla `TestCommand`): finding test instance blocks, clear, reset,
//! create, locate, pos, and building the runs that `run` and `verify` start.

use super::*;
use kiln_command::text::ClickEvent;
use kiln_command::{CommandError, SelectorTarget};

/// `RetryOptions`: tries and whether the first failure halts the retries.
pub(super) type Retry = (i32, bool);

/// `CommandSourceStack.getPlayer`.
fn command_failed() -> CommandError {
    CommandError::new(tr!("command.failed"))
}

fn tp_command(dim: DimId, pos: Pos, rot: Option<(i32, i32)>) -> String {
    let (yaw, pitch) = rot.unwrap_or((0, 0));
    format!("/execute in {} run tp @s {} {} {} {yaw} {pitch}", DIMENSIONS[dim].0, pos[0], pos[1], pos[2])
}

/// A dimension's path (`overworld`), as `test.run.coordinates` names it.
fn dimension_path(dim: DimId) -> &'static str {
    let key = DIMENSIONS[dim].0;
    key.split_once(':').map_or(key, |(_, p)| p)
}

impl Sim {
    /// The source's block position (`BlockPos.containing(getPosition())`).
    fn source_block(&self) -> Pos {
        self.commands.stack.position.map(|v| v.floor() as i32)
    }

    /// `CommandSourceStack.getPlayer`.
    fn source_player_ref(&self) -> Option<crate::commands::PlayerRef> {
        self.commands.stack.entity.clone().filter(|e| e.is_player())
    }

    /// The test instance blocks a selection names (`TestFinder.findTestPos`), as (level,
    /// position).
    fn selection_positions(&mut self, sel: &TestSelection) -> Result<Vec<(DimId, Pos)>, CommandError> {
        let dim = self.source_dim();
        let at = self.source_block();
        Ok(match sel {
            TestSelection::Ids(_) | TestSelection::Failed { .. } => Vec::new(),
            TestSelection::Nearby => self.find_test_blocks(dim, at, FULL_RADIUS).into_iter().map(|p| (dim, p)).collect(),
            TestSelection::Radius(r) => self.find_test_blocks(dim, at, *r).into_iter().map(|p| (dim, p)).collect(),
            TestSelection::Nearest => {
                let blocks = self.find_test_blocks(dim, at, NEARBY_RADIUS);
                let manhattan = |p: &Pos| (0..3).map(|i| (p[i] - at[i]).abs()).sum::<i32>();
                blocks.iter().min_by_key(|p| manhattan(p)).map(|p| vec![(dim, *p)]).unwrap_or_default()
            }
            TestSelection::LookedAt => {
                // `getPlayer().getCamera()` fails for a source that is not a player.
                let player = self.source_player_ref().ok_or_else(command_failed)?;
                let [x, y, z] = player.position();
                let eye = [x, y + player.eye_height(), z];
                let [yaw, pitch] = player.rotation();
                let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
                let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
                let end = [eye[0] + look[0] * 250.0, eye[1] + look[1] * 250.0, eye[2] + look[2] * 250.0];
                let mut hits: Vec<Pos> = self
                    .find_test_blocks(dim, at, FULL_RADIUS)
                    .into_iter()
                    .filter(|&p| {
                        let Some(data) = self.test_block_data(dim, p) else { return false };
                        let (lo, hi) = self.layout_of(p, &data).structure_bounds().aabb();
                        ray_hits(eye, end, lo, hi)
                    })
                    .collect();
                let dist = |p: &Pos| (0..3).map(|i| i64::from(p[i] - at[i]).pow(2)).sum::<i64>();
                hits.sort_by_key(dist);
                hits.truncate(1);
                hits.into_iter().map(|p| (dim, p)).collect()
            }
        })
    }

    /// The test ids a selection names (`TestFinder.findTests`).
    fn selection_tests(&self, sel: &TestSelection) -> Vec<String> {
        match sel {
            TestSelection::Ids(ids) => ids.clone(),
            TestSelection::Failed { only_required } => {
                let mut ids: Vec<String> = self
                    .commands
                    .gametests
                    .last_failed
                    .iter()
                    .filter(|id| !*only_required || self.commands.gametests.defs.tests.get(*id).is_some_and(|d| d.required))
                    .cloned()
                    .collect();
                ids.sort();
                ids
            }
            _ => Vec::new(),
        }
    }

    /// `verifyStructureExists`: whether the test's structure template is there (otherwise the
    /// source is told).
    fn verify_structure_exists(&mut self, structure: &str) -> bool {
        if self.find_template(structure).is_none() {
            self.send_failure(tr!("commands.test.error.structure_not_found", Text::literal(structure)));
            return false;
        }
        true
    }

    /// `createGameTestInfo`: the run of the test the block at `pos` holds.
    fn info_at(&mut self, dim: DimId, pos: Pos, retry: Retry) -> Option<Info> {
        let Some(data) = self.test_block_data(dim, pos) else {
            self.send_failure(tr!("commands.test.error.test_instance_not_found.position", pos[0], pos[1], pos[2]));
            return None;
        };
        let def = data.test.as_ref().and_then(|t| self.commands.gametests.defs.tests.get(t)).cloned();
        let (Some(id), Some(def)) = (data.test.clone(), def) else {
            let name = data.test.as_deref().map_or_else(|| tr!("test_instance_block.invalid_test"), Text::literal);
            self.send_failure(tr!("commands.test.error.non_existant_test", name));
            return None;
        };
        let layout = self.layout_of(pos, &data);
        let structure = def.structure.clone();
        let mut info = Info::new(id, def, layout.rotation, dim, retry);
        info.block = Some(pos);
        self.verify_structure_exists(&structure).then_some(info)
    }

    /// `toGameTestInfo`: one run per selected test whose structure exists.
    fn infos_of_tests(&mut self, ids: &[String], retry: Retry, rotation_steps: i32) -> Vec<Info> {
        let mut out = Vec::new();
        for id in ids {
            let Some(def) = self.commands.gametests.defs.tests.get(id).cloned() else { continue };
            if !self.verify_structure_exists(&def.structure) {
                continue;
            }
            let Some(dim) = dim_id(&def.dimension) else {
                self.send_failure(Text::literal(format!("Could not resolve level for dimension: {}", def.dimension)));
                continue;
            };
            out.push(Info::new(id.clone(), def, (rotation_steps.rem_euclid(4)) as u8, dim, retry));
        }
        out
    }

    /// A player-facing dimension list for the summary and locate: `outputPlayerCoordinates`.
    pub(super) fn output_player_coordinates(&mut self, dims: &[DimId]) {
        let Some(player) = self.source_player_ref() else { return };
        let pdim = dim_id(player.dimension()).unwrap_or(crate::OVERWORLD_ID);
        if in_test_dimension(true, pdim, dims) {
            return;
        }
        let [x, y, z] = self.source_block();
        let [yaw, pitch] = player.rotation();
        let text = tr!("test.player.coordinates", x, y, z, dimension_path(pdim))
            .bracketed()
            .color("gold")
            .click(ClickEvent::SuggestCommand(tp_command(pdim, [x, y, z], Some((yaw.floor() as i32, pitch.floor() as i32)))))
            .hover(tr!("chat.coordinates.tooltip"));
        self.send_success(text, false);
    }

    /// `outputTestCoordinates`: where the tests of each dimension were put.
    pub(super) fn output_test_coordinates(&mut self, dims: &[DimId]) {
        let has_player = self.source_player_ref().is_some();
        let source_dim = self.source_dim();
        let here = if has_player { self.source_player_ref().and_then(|p| dim_id(p.dimension())) } else { None };
        let _ = source_dim;
        for &dim in dims {
            if here.is_some_and(|d| in_test_dimension(true, d, dims)) {
                return;
            }
            let (p, surface) = self.test_info(dim);
            let text = tr!("test.run.coordinates", p[0], surface, p[2], dimension_path(dim))
                .bracketed()
                .color("yellow")
                .click(ClickEvent::SuggestCommand(tp_command(dim, [p[0], surface, p[2]], None)))
                .hover(tr!("chat.coordinates.tooltip"));
            self.send_success(text, false);
        }
    }

    // ---- the subcommands ----

    pub(crate) fn gametest_command(&mut self, command: &TestCommand) -> Result<i32, CommandError> {
        match command {
            TestCommand::Stop => {
                self.stop_tests();
                Ok(1)
            }
            TestCommand::Clear(sel) => self.test_clear(sel),
            TestCommand::Reset(sel) => self.test_reset(sel),
            TestCommand::Create { id, size } => self.test_create(id, *size),
            TestCommand::Locate { ids } => self.test_locate(ids),
            TestCommand::Pos(var) => self.test_pos(var),
            TestCommand::Run { select, copies, tries, halt_on_failure, rotation_steps, per_row } => {
                self.test_run(select, *copies, (*tries, *halt_on_failure), *rotation_steps, *per_row)
            }
            TestCommand::Verify { ids } => self.test_verify(ids),
        }
    }

    /// `clear`: takes the test structures away.
    fn test_clear(&mut self, sel: &TestSelection) -> Result<i32, CommandError> {
        self.stop_tests();
        let positions = self.selection_positions(sel)?;
        let mut cleared = 0;
        for (dim, pos) in positions {
            let Some(data) = self.test_block_data(dim, pos) else { continue };
            let layout = self.layout_of(pos, &data);
            self.clear_space(dim, layout.test_bounds());
            self.remove_barriers(dim, &layout, data.test.as_ref().and_then(|t| self.commands.gametests.defs.tests.get(t)).is_some_and(|d| d.sky_access));
            Host::destroy_block(self, DIMENSIONS[dim].0, pos, false);
            cleared += 1;
        }
        if cleared == 0 {
            return Err(CommandError::new(tr!("commands.test.clear.error.no_tests")));
        }
        self.send_success(tr!("commands.test.clear.success", cleared), true);
        Ok(cleared)
    }

    /// `reset`: puts the structure of each test back.
    fn test_reset(&mut self, sel: &TestSelection) -> Result<i32, CommandError> {
        self.stop_tests();
        let positions = self.selection_positions(sel)?;
        let mut infos = Vec::new();
        for (dim, pos) in positions {
            if let Some(info) = self.info_at(dim, pos, (1, true)) {
                infos.push(info);
            }
        }
        let mut count = 0;
        for info in &infos {
            let (dim, pos) = (info.dim, info.block.expect("a test block"));
            let Some(data) = self.test_block_data(dim, pos) else { continue };
            let layout = self.layout_of(pos, &data);
            self.remove_barriers(dim, &layout, info.def.sky_access);
            let mut data = data;
            data.errors.clear();
            let placed = self.place_structure(dim, pos);
            if placed {
                let name = Text::literal(info.test.clone());
                self.send_system_to_source(tr!("test_instance_block.reset_success", name).color("green"));
            }
            data.status = Status::Cleared;
            if let Some(mut current) = self.test_block_data(dim, pos) {
                current.status = Status::Cleared;
                current.errors.clear();
                data = current;
            }
            self.set_test_block_data(dim, pos, &data);
            count += 1;
        }
        if count == 0 {
            return Err(CommandError::new(tr!("commands.test.clear.error.no_tests")));
        }
        self.send_success(tr!("commands.test.reset.success", count), true);
        Ok(count)
    }

    /// `Consumer<Component>` = `source::sendSystemMessage`: to the source only.
    pub(super) fn send_system_to_source(&mut self, text: Text) {
        match self.commands.source {
            CommandSource::Console => self.reply_console(&text),
            CommandSource::Player(conn) => {
                if let Some(p) = self.players.get_mut(&conn) {
                    p.send(kiln_proto::packets::system_chat(text.to_nbt(), false));
                }
            }
        }
    }

    /// `create`: an empty test structure with its test instance block.
    fn test_create(&mut self, id: &Identifier, size: [i32; 3]) -> Result<i32, CommandError> {
        if size.iter().any(|&v| v > MAX_SIZE) {
            return Err(CommandError::new(tr!("commands.test.error.too_large", MAX_SIZE)));
        }
        let dim = self.source_dim();
        let block = self.test_position_around(dim);
        let dimension = DIMENSIONS[dim].0;
        let layout = Layout { block, size, rotation: 0, padding: 0 };
        let sp = [block[0] + STRUCTURE_OFFSET[0], block[1] + STRUCTURE_OFFSET[1], block[2] + STRUCTURE_OFFSET[2]];
        self.clear_space(dim, Layout { block, size, rotation: 0, padding: 0 }.structure_bounds());
        let _ = sp;
        self.dims[dim].load_chunk(kiln_world::ChunkPos::of_block(block[0], block[2]));
        self.set_block(dimension, block, kiln_data::blocks::default_state::TEST_INSTANCE_BLOCK, None, UpdateFlags(3));
        let mut data = BlockData::empty();
        data.test = Some(id.as_str().to_owned());
        data.size = size;
        self.set_test_block_data(dim, block, &data);
        // A bedrock floor under the structure.
        let floor = layout.structure_pos();
        for x in 0..size[0].max(1) {
            for z in 0..size[2].max(1) {
                self.set_block(dimension, [floor[0] + x, floor[1], floor[2] + z], kiln_data::blocks::default_state::BEDROCK, None, UpdateFlags(3));
            }
        }
        self.send_success(tr!("commands.test.create.success", test_name(id.as_str())), true);
        Ok(1)
    }

    /// `locate`: where the selected tests' blocks are.
    fn test_locate(&mut self, ids: &[String]) -> Result<i32, CommandError> {
        self.send_system_to_source(tr!("commands.test.locate.started"));
        let at = self.source_block();
        let source_dim = self.source_dim();
        let mut found: Vec<(DimId, Pos)> = Vec::new();
        for dim in 0..self.dims.len() {
            for pos in self.find_test_blocks(dim, at, FULL_RADIUS) {
                let Some(data) = self.test_block_data(dim, pos) else { continue };
                if data.test.as_ref().is_some_and(|t| ids.contains(t)) && !found.contains(&(dim, pos)) {
                    found.push((dim, pos));
                }
            }
        }
        let mut dims: Vec<DimId> = Vec::new();
        for &(dim, pos) in &found {
            let Some(data) = self.test_block_data(dim, pos) else { continue };
            if !dims.contains(&dim) {
                dims.push(dim);
            }
            // The spot three blocks in front of the structure, looking at it.
            let layout = self.layout_of(pos, &data);
            let (dx, dz) = match layout.rotation % 4 {
                0 => (0, -1),
                1 => (1, 0),
                2 => (0, 1),
                _ => (-1, 0),
            };
            let stand = [pos[0] + dx * 3, pos[1], pos[2] + dz * 3];
            // The direction the block faces, turned around: `toYRot` of the opposite.
            let yaw = match layout.rotation % 4 {
                0 => 0,
                1 => 90,
                2 => 180,
                _ => -90,
            };
            let command = format!("/execute in {} run tp @s {} {} {} {yaw} 0", DIMENSIONS[dim].0, stand[0], stand[1], stand[2]);
            let distance = if dim == source_dim {
                let (ex, ez) = ((pos[0] - at[0]) as f32, (pos[2] - at[2]) as f32);
                ((ex * ex + ez * ez).sqrt().floor() as i32).to_string()
            } else {
                "N/A".to_owned()
            };
            let coords = coordinates(pos).append(Text::literal(format!(" ->[{}]", dimension_path(dim)))).bracketed();
            let coords =
                coords.color("green").click(ClickEvent::SuggestCommand(command)).hover(tr!("chat.coordinates.tooltip"));
            self.send_success(tr!("commands.test.locate.found", coords, distance), false);
        }
        if found.is_empty() {
            return Err(CommandError::new(tr!("commands.test.error.no_test_instances")));
        }
        self.output_player_coordinates(&dims);
        self.send_success(tr!("commands.test.locate.done", found.len() as i32), true);
        Ok(found.len() as i32)
    }

    /// `pos`: the looked-at block relative to the test structure it is in.
    fn test_pos(&mut self, var: &str) -> Result<i32, CommandError> {
        let player = self.source_player_ref().ok_or_else(CommandError::requires_player)?;
        let dim = dim_id(player.dimension()).unwrap_or(crate::OVERWORLD_ID);
        let [x, y, z] = player.position();
        let eye = [x, y + player.eye_height(), z];
        let [yaw, pitch] = player.rotation();
        let (yaw, pitch) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
        let look = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
        // `pick(10, 0, false)`: the first solid block along the view, else the point 10 away.
        let mut hit = None;
        for i in 0..=200 {
            let t = f64::from(i) * 0.05;
            let p = [0, 1, 2].map(|k| (eye[k] + look[k] * t * 10.0 / 10.0 * (10.0 / 10.0)).floor() as i32);
            let state = Host::block_state(self, DIMENSIONS[dim].0, p);
            if state != kiln_data::blocks::default_state::AIR && state != kiln_data::blocks::default_state::VOID_AIR {
                hit = Some(p);
                break;
            }
        }
        let hit = hit.unwrap_or_else(|| [0, 1, 2].map(|k| (eye[k] + look[k] * 10.0).floor() as i32));
        let containing = |s: &Sim, r: i32| {
            s.find_test_blocks(dim, hit, r).into_iter().find(|&p| {
                s.test_block_data(dim, p).is_some_and(|d| s.layout_of(p, &d).structure_bounds().contains(hit))
            })
        };
        let Some(block) = containing(self, NEARBY_RADIUS).or_else(|| containing(self, FULL_RADIUS)) else {
            return Err(CommandError::new(tr!("commands.test.error.no_test_containing_pos", hit[0], hit[1], hit[2])));
        };
        let Some(data) = self.test_block_data(dim, block) else {
            return Err(CommandError::new(tr!("commands.test.error.test_instance_not_found")));
        };
        let layout = self.layout_of(block, &data);
        let origin = layout.structure_pos();
        let rel = [hit[0] - origin[0], hit[1] - origin[1], hit[2] - origin[2]];
        let coords = format!("{}, {}, {}", rel[0], rel[1], rel[2]);
        let name = data.test.clone().unwrap_or_default();
        let copy = format!("final BlockPos {var} = new BlockPos({coords});");
        let component = tr!("commands.test.coordinates", rel[0], rel[1], rel[2])
            .color("green")
            .click(ClickEvent::CopyToClipboard(copy))
            .hover(tr!("commands.test.coordinates.copy"));
        let component = {
            let mut c = component;
            c.style.bold = Some(true);
            c
        };
        let _ = name.clone();
        self.send_success(tr!("commands.test.relative_position", Text::literal(name), component), false);
        if let Some(conn) = match self.commands.source {
            CommandSource::Player(c) => Some(c),
            CommandSource::Console => None,
        } {
            let pkt = highlight_packet(hit, rel);
            if let Some(p) = self.players.get_mut(&conn) {
                p.send(pkt);
            }
        }
        Ok(1)
    }

    /// `run` and its variants: makes the runs and starts them.
    fn test_run(&mut self, sel: &TestSelection, copies: i32, retry: Retry, rotation_steps: i32, per_row: i32) -> Result<i32, CommandError> {
        self.stop_tests();
        let positions = self.selection_positions(sel)?;
        let mut infos = Vec::new();
        for (dim, pos) in positions {
            infos.extend(self.info_at(dim, pos, retry));
        }
        let ids = self.selection_tests(sel);
        let repeated: Vec<String> = (0..copies.max(0)).flat_map(|_| ids.iter().cloned()).collect();
        infos.extend(self.infos_of_tests(&repeated, retry, rotation_steps));
        if infos.is_empty() {
            self.send_success(tr!("commands.test.no_tests"), false);
            return Ok(0);
        }
        self.commands.gametests.last_failed.clear();
        self.send_success(tr!("commands.test.run.running", infos.len() as i32), false);
        let batches = self.batch_infos(&infos, BATCH_SIZE);
        let mut runner = Runner::new(self.commands.source, infos, batches);
        runner.per_row = per_row;
        self.fill_origins(&mut runner);
        self.track_and_start(runner);
        Ok(1)
    }

    /// `verify`: every test in each rotation, 100 copies at a time.
    fn test_verify(&mut self, ids: &[String]) -> Result<i32, CommandError> {
        self.stop_tests();
        let base = self.infos_of_tests(ids, (1, true), 0);
        self.commands.gametests.last_failed.clear();
        let mut infos: Vec<Info> = Vec::new();
        let mut batches = Vec::new();
        for info in &base {
            for rotation in 0..4u8 {
                let mut indexes = Vec::new();
                for _ in 0..VERIFY_BATCH {
                    let i = Info::new(info.test.clone(), info.def.clone(), rotation, info.dim, (1, true));
                    indexes.push(infos.len());
                    infos.push(i);
                }
                batches.push(Batch { env: info.def.environment.clone(), dim: info.dim, index: rotation as usize, infos: indexes });
            }
        }
        let mut runner = Runner::new(self.commands.source, infos, batches);
        runner.per_row = VERIFY_PER_ROW;
        runner.grid_clears_on_batch = true;
        runner.halt_on_error = true;
        runner.clear_between_batches = true;
        runner.batch_size = VERIFY_BATCH;
        self.fill_origins(&mut runner);
        self.track_and_start(runner);
        Ok(1)
    }

    /// Where each dimension's grid of test structures starts, taken from the source now.
    fn fill_origins(&mut self, runner: &mut Runner) {
        let dims: Vec<DimId> = runner.infos.iter().map(|i| i.dim).collect();
        for dim in dims {
            if !runner.origins.contains_key(&dim) {
                let origin = self.test_position_around(dim);
                runner.origins.insert(dim, origin);
            }
        }
    }

    /// `GameTestBatchFactory.fromGameTestInfo`: by environment and dimension, `size` at a time.
    pub(super) fn batch_infos(&self, infos: &[Info], size: usize) -> Vec<Batch> {
        let mut groups: Vec<((String, DimId), Vec<usize>)> = Vec::new();
        for (i, info) in infos.iter().enumerate() {
            let key = (info.def.environment.clone(), info.dim);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(i),
                None => groups.push((key, vec![i])),
            }
        }
        let mut out = Vec::new();
        for ((env, dim), all) in groups {
            for (index, chunk) in all.chunks(size).enumerate() {
                out.push(Batch { env: env.clone(), dim, index, infos: chunk.to_vec() });
            }
        }
        out
    }
}

/// `isPlayerInCurrentTestDimension`: the player is in the only test dimension, the overworld.
fn in_test_dimension(has_player: bool, player_dim: DimId, dims: &[DimId]) -> bool {
    has_player && dims.len() <= 1 && dims.contains(&player_dim) && dims.contains(&crate::OVERWORLD_ID)
}

/// Whether the segment from `a` to `b` crosses the box `lo..hi` (`AABB.clip`).
fn ray_hits(a: [f64; 3], b: [f64; 3], lo: [f64; 3], hi: [f64; 3]) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for i in 0..3 {
        let d = b[i] - a[i];
        if d.abs() < 1e-12 {
            if a[i] < lo[i] || a[i] > hi[i] {
                return false;
            }
        } else {
            let (mut n, mut f) = ((lo[i] - a[i]) / d, (hi[i] - a[i]) / d);
            if n > f {
                std::mem::swap(&mut n, &mut f);
            }
            t0 = t0.max(n);
            t1 = t1.min(f);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// `ClientboundGameTestHighlightPosPacket(absolute, relative)`.
fn highlight_packet(absolute: Pos, relative: Pos) -> bytes::Bytes {
    kiln_proto::packets::game_test_highlight_pos(absolute, relative)
}
