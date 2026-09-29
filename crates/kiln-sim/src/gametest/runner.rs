//! The runner (`GameTestRunner`, `GameTestInfo`, `GameTestTicker`, `ReportGameListener`,
//! `MultipleTestTracker` and the summary the command shows): batches of tests spawned on a
//! grid, an environment per batch, ticking each test and reporting what happens.

use super::*;

const BARRIER: u16 = kiln_data::blocks::default_state::BARRIER;
const TEST_INSTANCE_BLOCK: u16 = kiln_data::blocks::default_state::TEST_INSTANCE_BLOCK;
const TEST_BLOCK_FIRST: u16 = 25095;
const REDSTONE_BLOCK: u16 = kiln_data::blocks::default_state::REDSTONE_BLOCK;
/// `TestBlockMode` in state order.
const NEIGHBORS: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];
const MODES: [&str; 4] = ["start", "log", "fail", "accept"];

fn mode_of(state: u16) -> Option<&'static str> {
    (TEST_BLOCK_FIRST..TEST_BLOCK_FIRST + 4).contains(&state).then(|| MODES[(state - TEST_BLOCK_FIRST) as usize])
}

impl Info {
    pub(super) fn new(test: String, def: TestDef, extra_rotation: u8, dim: DimId, (tries, halt_on_failure): (i32, bool)) -> Info {
        Info {
            test,
            def,
            extra_rotation: extra_rotation % 4,
            dim,
            tries,
            halt_on_failure,
            block: None,
            placed: false,
            tick: 0,
            started: false,
            done: false,
            error: None,
            error_kind: "GameTestException",
            report: usize::MAX,
            test_blocks: Vec::new(),
            triggered: Vec::new(),
            powered: Vec::new(),
            starts_powered: Vec::new(),
            started_at: std::time::Instant::now(),
            ran_ms: 0,
        }
    }

    fn unlimited_tries(&self) -> bool {
        self.tries < 1
    }

    fn has_retries(&self) -> bool {
        self.tries != 1
    }

    /// `RetryOptions.hasTriesLeft`.
    fn has_tries_left(&self, attempts: i32, successes: i32) -> bool {
        let failed = attempts != successes;
        let left = self.unlimited_tries() || attempts < self.tries;
        left && (!failed || !self.halt_on_failure)
    }

    fn flaky(&self) -> bool {
        self.def.max_attempts > 1
    }

    fn has_failed(&self) -> bool {
        self.error.is_some()
    }

    /// `startExecution(1)`: setup ticks and one to spawn, then the test.
    fn start_execution(&mut self) {
        self.tick = -(self.def.setup_ticks + 1 + 1);
    }

    /// `GameTestInfo.fail(Component)`: an assertion failing on the current tick.
    fn fail(&mut self, message: Text) {
        let text = tr!("test.error.tick", message, self.tick);
        self.error = Some(Failure { text, at: None, assertion: true });
    }

    /// `GameTestInfo.fail(GameTestException)` for the framework's own exceptions.
    fn fail_plain(&mut self, kind: &'static str, message: Text) {
        self.error = Some(Failure { text: message, at: None, assertion: false });
        self.error_kind = kind;
    }
}

impl Runner {
    pub(super) fn new(source: CommandSource, mut infos: Vec<Info>, batches: Vec<Batch>) -> Runner {
        let mut reports = Vec::new();
        for info in &mut infos {
            info.report = reports.len();
            reports.push(Report::default());
        }
        Runner {
            source,
            infos,
            batches,
            current: 0,
            stopped: true,
            env: None,
            reports,
            per_row: 8,
            grid_clears_on_batch: false,
            halt_on_error: false,
            clear_between_batches: false,
            batch_size: BATCH_SIZE,
            origins: HashMap::new(),
            grids: HashMap::new(),
            tracked: Vec::new(),
            batch_tracked: Vec::new(),
            scheduled_reruns: Vec::new(),
            summary_dims: Vec::new(),
            terminated: false,
            ticking: Vec::new(),
        }
    }

    fn done_count(&self) -> usize {
        self.tracked.iter().filter(|&&i| self.infos[i].done).count()
    }

    fn failed_required(&self) -> usize {
        self.tracked.iter().filter(|&&i| self.infos[i].has_failed() && self.infos[i].def.required).count()
    }

    fn failed_optional(&self) -> usize {
        self.tracked.iter().filter(|&&i| self.infos[i].has_failed() && !self.infos[i].def.required).count()
    }
}

impl Sim {
    // ---- starting and stopping ----

    /// `TestCommand.stopTests` (`GameTestTicker.clear`): forgets the run, ending its environment.
    pub(super) fn stop_tests(&mut self) {
        if let Some(mut r) = self.commands.gametests.runner.take() {
            r.stopped = true;
            self.end_env(&mut r);
        }
    }

    /// `TestCommand.trackAndStartRunner`: the summary tracks every test; the first batch starts.
    pub(super) fn track_and_start(&mut self, mut r: Runner) {
        r.tracked = (0..r.infos.len()).collect();
        r.stopped = false;
        self.run_batch(&mut r, 0);
        if !r.terminated {
            self.commands.gametests.runner = Some(r);
        }
    }

    /// `GameTestRunner.runBatch`.
    fn run_batch(&mut self, r: &mut Runner, index: usize) {
        if index >= r.batches.len() {
            self.end_env(r);
            self.run_scheduled_reruns(r);
            return;
        }
        r.current = index;
        if index > 0 && r.clear_between_batches {
            let previous = r.batches[index - 1].infos.clone();
            for i in previous {
                let info = &r.infos[i];
                let (Some(block), dim) = (info.block, info.dim) else { continue };
                if let Some(data) = self.test_block_data(dim, block) {
                    let layout = self.layout_of(block, &data);
                    self.clear_space(dim, layout.test_bounds());
                    Host::destroy_block(self, DIMENSIONS[dim].0, block, false);
                }
            }
        }
        // `StructureSpawner.onBatchStart` of the grid spawner that clears.
        if r.grid_clears_on_batch {
            let dims: Vec<DimId> = r.grids.keys().copied().collect();
            for dim in dims {
                let last = std::mem::take(&mut r.grids.get_mut(&dim).expect("grid").last_batch);
                for i in last {
                    if let (Some(block), d) = (r.infos[i].block, r.infos[i].dim)
                        && let Some(data) = self.test_block_data(d, block)
                    {
                        let layout = self.layout_of(block, &data);
                        self.clear_space(d, layout.test_bounds());
                    }
                }
                let g = r.grids.get_mut(&dim).expect("grid");
                g.next = g.first;
                g.row_min = g.first.map(f64::from);
                g.row_max = g.first.map(|v| f64::from(v + 1));
                g.in_row = 0;
            }
        }
        let wanted = r.batches[index].infos.clone();
        let mut spawned = Vec::new();
        for i in wanted {
            if self.spawn(r, i) {
                spawned.push(i);
            }
        }
        let (env, dim, batch_index) = {
            let b = &r.batches[index];
            (b.env.clone(), b.dim, b.index)
        };
        info!(
            "Running test environment '{}' batch {} ({} tests)...",
            self.commands.gametests.defs.env_name(&env),
            batch_index,
            spawned.len()
        );
        self.end_env(r);
        let activation = self.activate_env(&env, dim);
        r.env = Some(activation);
        let name = self.commands.gametests.defs.env_name(&env);
        self.as_source(r.source, |s| s.send_success(tr!("commands.test.batch.starting", Text::literal(name), batch_index as i32), true));
        r.batch_tracked = spawned.clone();
        r.ticking.extend(spawned);
    }

    /// `StructureSpawner.spawnStructure`: onto the grid (a test without a block) or in place.
    fn spawn(&mut self, r: &mut Runner, i: usize) -> bool {
        let dim = r.infos[i].dim;
        let on_grid = r.infos[i].block.is_none();
        if on_grid {
            let origin = match r.origins.get(&dim) {
                Some(o) => *o,
                None => {
                    let o = self.test_position_around(dim);
                    r.origins.insert(dim, o);
                    o
                }
            };
            let grid = r.grids.entry(dim).or_insert_with(|| Grid::new(origin));
            r.infos[i].block = Some(grid.next);
        }
        if !self.prepare_test_structure(r, i) {
            return false;
        }
        r.infos[i].start_execution();
        if on_grid {
            let (block, dim) = (r.infos[i].block.expect("block"), r.infos[i].dim);
            let data = self.test_block_data(dim, block);
            let bounds = data.map(|d| self.layout_of(block, &d).test_bounds());
            let per_row = r.per_row;
            let grid = r.grids.get_mut(&dim).expect("grid");
            if let Some(b) = bounds {
                let (lo, hi) = b.aabb();
                for k in 0..3 {
                    grid.row_min[k] = grid.row_min[k].min(lo[k]);
                    grid.row_max[k] = grid.row_max[k].max(hi[k]);
                }
                grid.next[0] += (hi[0] - lo[0]) as i32 + COLUMN_GAP;
            }
            grid.in_row += 1;
            if grid.in_row >= per_row {
                grid.in_row = 0;
                grid.next[2] += (grid.row_max[2] - grid.row_min[2]) as i32 + ROW_GAP;
                grid.next[0] = grid.first[0];
                grid.row_min = grid.next.map(f64::from);
                grid.row_max = grid.next.map(|v| f64::from(v + 1));
            }
            grid.last_batch.push(i);
        }
        true
    }

    /// `GameTestInfo.prepareTestStructure`: the test instance block with its data, and the
    /// structure around it.
    fn prepare_test_structure(&mut self, r: &mut Runner, i: usize) -> bool {
        let (dim, block, test, extra) = {
            let info = &r.infos[i];
            (info.dim, info.block.expect("a position"), info.test.clone(), info.extra_rotation)
        };
        let dimension = DIMENSIONS[dim].0;
        self.dims[dim].load_chunk(kiln_world::ChunkPos::of_block(block[0], block[2]));
        self.set_block(dimension, block, TEST_INSTANCE_BLOCK, None, UpdateFlags(3));
        let Some(def) = self.commands.gametests.defs.tests.get(&test).cloned() else { return false };
        let size = self.find_template(&def.structure).map_or([1, 1, 1], |t| t.size);
        let mut data = BlockData::empty();
        data.test = Some(test.clone());
        data.size = size;
        data.rotation = extra;
        self.set_test_block_data(dim, block, &data);
        // `GameTestInfo.placeStructure`.
        if !self.place_structure(dim, block) {
            let name = test_name(&test);
            r.infos[i].fail_plain("GameTestException", tr!("test.error.structure.failure", name));
        }
        r.infos[i].placed = true;
        if let Some(data) = self.test_block_data(dim, block) {
            let layout = self.layout_of(block, &data);
            self.encase_structure(dim, &layout, def.sky_access);
        }
        // `testStructureLoaded`: the report counts an attempt.
        r.reports[r.infos[i].report].attempts += 1;
        true
    }

    /// `TestInstanceBlockEntity.placeStructure`: the space cleared and the template placed.
    pub(super) fn place_structure(&mut self, dim: DimId, block: Pos) -> bool {
        let Some(data) = self.test_block_data(dim, block) else { return false };
        let Some(def) = data.test.as_ref().and_then(|t| self.commands.gametests.defs.tests.get(t)).cloned() else { return false };
        let Some(template) = self.find_template(&def.structure) else { return false };
        let _ = template;
        let layout = self.layout_of(block, &data);
        self.force_load(dim, layout.structure_bounds());
        self.clear_space(dim, layout.test_bounds());
        let at = layout.start_corner();
        self.place_template(dim, &def.structure, at, layout.rotation, 0, 1.0, 818, true).is_ok()
    }

    /// The blocks around the structure (`processStructureBoundary`): the sides and floor, and
    /// the top of a test that has no sky access.
    fn boundary(layout: &Layout, sky_access: bool) -> Vec<Pos> {
        let b = layout.structure_bounds();
        let (c1, c2) = (b.min.map(|v| v - 1), b.max.map(|v| v + 1));
        Bounds { min: c1, max: c2 }
            .positions()
            .filter(|p| {
                let side = p[0] == c1[0] || p[0] == c2[0] || p[2] == c1[2] || p[2] == c2[2] || p[1] == c1[1];
                let top = p[1] == c2[1];
                side || (top && !sky_access)
            })
            .collect()
    }

    fn encase_structure(&mut self, dim: DimId, layout: &Layout, sky_access: bool) {
        let dimension = DIMENSIONS[dim].0;
        for p in Self::boundary(layout, sky_access) {
            if Host::block_state(self, dimension, p) != TEST_INSTANCE_BLOCK {
                self.set_block(dimension, p, BARRIER, None, UpdateFlags(3));
            }
        }
    }

    pub(super) fn remove_barriers(&mut self, dim: DimId, layout: &Layout, sky_access: bool) {
        let dimension = DIMENSIONS[dim].0;
        for p in Self::boundary(layout, sky_access) {
            if Host::block_state(self, dimension, p) == BARRIER {
                self.set_block(dimension, p, kiln_data::blocks::default_state::AIR, None, UpdateFlags(3));
            }
        }
    }

    // ---- environments ----

    fn end_env(&mut self, r: &mut Runner) {
        if let Some(active) = r.env.take() {
            self.teardown_env(active);
        }
    }

    /// `TestEnvironmentDefinition.activate`: sets the environment up and remembers how to undo it.
    fn activate_env(&mut self, id: &str, dim: DimId) -> Activation {
        let env = self.commands.gametests.defs.envs.get(id).cloned().unwrap_or(Env::AllOf(Vec::new()));
        let mut saved = Vec::new();
        self.setup_env(&env, dim, &mut saved);
        Activation { env, dim, saved }
    }

    fn setup_env(&mut self, env: &Env, dim: DimId, saved: &mut Vec<Saved>) {
        match env {
            Env::AllOf(list) => list.iter().for_each(|e| self.setup_env(e, dim, saved)),
            Env::GameRules(rules) => {
                for (rule, value) in rules {
                    saved.push(Saved::Rules(rule.clone(), Host::game_rule(self, rule)));
                    Host::set_game_rule(self, rule, *value);
                }
            }
            Env::Difficulty(name) => {
                let current = Host::difficulty(self);
                saved.push(Saved::Difficulty(current.name().to_owned()));
                if let Some(d) = kiln_command::Difficulty::by_name(name) {
                    Host::set_difficulty(self, d);
                }
            }
            Env::ClockTime { clock, time } => {
                let id = Identifier::parse(clock);
                let now = self.as_console(|s| Host::time(s, id.as_ref(), &TimeAction::QueryTime)).unwrap_or(0);
                saved.push(Saved::Time(clock.clone(), now));
                let _ = self.as_console(|s| Host::time(s, id.as_ref(), &TimeAction::Set(*time)));
            }
            Env::Weather(w) => {
                saved.push(Saved::Weather);
                Host::set_weather(self, *w, Some(100_000));
            }
            Env::Function { setup, teardown } => {
                if let Some(f) = setup {
                    self.run_env_function(f);
                }
                if let Some(t) = teardown {
                    saved.push(Saved::Teardown(t.clone()));
                }
            }
            Env::Timelines => {}
        }
        let _ = dim;
    }

    fn teardown_env(&mut self, active: Activation) {
        for s in active.saved.into_iter().rev() {
            match s {
                Saved::Rules(rule, value) => Host::set_game_rule(self, &rule, value),
                Saved::Difficulty(name) => {
                    if let Some(d) = kiln_command::Difficulty::by_name(&name) {
                        Host::set_difficulty(self, d);
                    }
                }
                Saved::Time(clock, t) => {
                    let id = Identifier::parse(&clock);
                    let _ = self.as_console(|s| Host::time(s, id.as_ref(), &TimeAction::Set(t)));
                }
                Saved::Weather => {
                    Host::set_weather(self, Weather::Clear, None);
                }
                Saved::Teardown(f) => self.run_env_function(&f),
            }
        }
        let _ = (active.env, active.dim);
    }

    /// `ServerFunctionManager.execute` for an environment's function; a missing one is logged.
    fn run_env_function(&mut self, id: &str) {
        let function = Identifier::parse(id).and_then(|i| self.commands.packs.library.get(&i));
        match function {
            Some(f) => self.run_server_function(&f),
            None => error!("Test Batch failed for non-existent function {id}"),
        }
    }

    fn as_console<R>(&mut self, f: impl FnOnce(&mut Sim) -> R) -> R {
        self.as_source(CommandSource::Console, f)
    }

    // ---- ticking ----

    /// `GameTestTicker.tick`: every running test ticks; finished ones leave.
    pub(crate) fn tick_gametests(&mut self) {
        let Some(mut r) = self.commands.gametests.runner.take() else { return };
        let ticking = std::mem::take(&mut r.ticking);
        let mut still = Vec::new();
        for i in ticking {
            if r.terminated {
                break;
            }
            self.tick_info(&mut r, i);
            if !r.infos[i].done {
                still.push(i);
            }
        }
        // Tests added while ticking (the next batch, reruns) keep their place after these.
        let added = std::mem::take(&mut r.ticking);
        r.ticking = still;
        r.ticking.extend(added);
        if !r.terminated {
            self.commands.gametests.runner = Some(r);
        }
    }

    /// `GameTestInfo.tick`.
    fn tick_info(&mut self, r: &mut Runner, i: usize) {
        if r.infos[i].done {
            return;
        }
        if !r.infos[i].placed {
            r.infos[i].fail_plain("GameTestException", tr!("test.error.ticking_without_structure"));
        }
        let (dim, block) = (r.infos[i].dim, r.infos[i].block);
        if block.is_none_or(|b| self.test_block_data(dim, b).is_none()) {
            r.infos[i].fail_plain("GameTestException", tr!("test.error.missing_block_entity"));
        }
        if r.infos[i].error.is_some() {
            self.finish(r, i);
        } else {
            self.tick_internal(r, i);
        }
        if r.infos[i].done {
            if r.infos[i].error.is_some() {
                self.test_failed(r, i);
            } else {
                self.test_passed(r, i);
            }
        }
    }

    fn finish(&mut self, r: &mut Runner, i: usize) {
        let info = &mut r.infos[i];
        if !info.done {
            info.done = true;
            info.ran_ms = info.started_at.elapsed().as_millis() as u64;
        }
    }

    /// `GameTestInfo.succeed`: unless it failed already, the test is over and its entities go.
    fn succeed(&mut self, r: &mut Runner, i: usize) {
        if r.infos[i].error.is_some() {
            return;
        }
        self.finish(r, i);
        let (dim, block) = (r.infos[i].dim, r.infos[i].block);
        if let Some(b) = block
            && let Some(data) = self.test_block_data(dim, b)
        {
            let bounds = self.layout_of(b, &data).structure_bounds();
            self.discard_entities(dim, bounds, 1.0);
        }
    }

    fn tick_internal(&mut self, r: &mut Runner, i: usize) {
        r.infos[i].tick += 1;
        if r.infos[i].tick < 0 {
            return;
        }
        if !r.infos[i].started {
            self.start_test(r, i);
        }
        // `onEachTick` of block-based tests: at every tick up to the timeout.
        if r.infos[i].started && !r.infos[i].done && r.infos[i].error.is_none() && r.infos[i].def.kind == Kind::BlockBased {
            self.block_based_tick(r, i);
        }
        let max = r.infos[i].def.max_ticks;
        if r.infos[i].tick > max && !r.infos[i].done {
            r.infos[i].fail_plain("GameTestTimeoutException", tr!("test.error.timeout.no_result", max));
        }
    }

    /// `GameTestInfo.startTest`.
    fn start_test(&mut self, r: &mut Runner, i: usize) {
        r.infos[i].started = true;
        r.infos[i].started_at = std::time::Instant::now();
        let (dim, block) = (r.infos[i].dim, r.infos[i].block.expect("a block"));
        if let Some(mut data) = self.test_block_data(dim, block) {
            data.status = Status::Running;
            self.set_test_block_data(dim, block, &data);
        }
        match r.infos[i].def.kind.clone() {
            Kind::Function(f) => {
                if f == TEST_FUNCTION_ALWAYS_PASS {
                    self.succeed(r, i);
                }
            }
            Kind::BlockBased => self.block_based_start(r, i),
        }
    }

    // ---- block-based tests ----

    fn block_based_start(&mut self, r: &mut Runner, i: usize) {
        let (dim, block) = (r.infos[i].dim, r.infos[i].block.expect("a block"));
        let dimension = DIMENSIONS[dim].0;
        let Some(data) = self.test_block_data(dim, block) else { return };
        let bounds = self.layout_of(block, &data).structure_bounds();
        let mut found = Vec::new();
        for p in bounds.positions() {
            if let Some(mode) = mode_of(Host::block_state(self, dimension, p)) {
                found.push((p, mode.to_owned()));
            }
        }
        let starts: Vec<Pos> = found.iter().filter(|(_, m)| m == "start").map(|(p, _)| *p).collect();
        r.infos[i].test_blocks = found;
        if starts.is_empty() {
            r.infos[i].fail(tr!("test_block.error.missing", tr!("test_block.mode.start")));
            return;
        }
        if starts.len() != 1 {
            r.infos[i].fail(tr!("test_block.error.too_many", tr!("test_block.mode.start")));
            return;
        }
        // `TestBlockEntity.trigger` of the start block: it powers up and its neighbours notice.
        r.infos[i].starts_powered.push(starts[0]);
        self.update_test_blocks(r, i, starts[0]);
    }

    /// `hasNeighborSignal` for a test block: a powered start block or a redstone block next
    /// to it (there is no redstone circuit to carry anything further).
    fn has_neighbor_signal(&mut self, dim: DimId, starts: &[Pos], p: Pos) -> bool {
        let dimension = DIMENSIONS[dim].0;
        NEIGHBORS.iter().any(|d| {
            let n = [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
            starts.contains(&n) || Host::block_state(self, dimension, n) == REDSTONE_BLOCK
        })
    }

    /// `Level.updateNeighborsAt(start)`: the accept, fail and log blocks next to the start
    /// block notice its signal (`TestBlock.neighborChanged`): a rising signal triggers them.
    /// Nothing else changes a neighbour here, so nothing else re-checks.
    fn update_test_blocks(&mut self, r: &mut Runner, i: usize, start: Pos) {
        let dim = r.infos[i].dim;
        let starts = r.infos[i].starts_powered.clone();
        for d in NEIGHBORS {
            let p = [start[0] + d[0], start[1] + d[1], start[2] + d[2]];
            let Some(mode) = r.infos[i].test_blocks.iter().find(|(q, m)| *q == p && m != "start").map(|(_, m)| m.clone()) else { continue };
            let signal = self.has_neighbor_signal(dim, &starts, p);
            let was = r.infos[i].powered.contains(&p);
            if signal && !was {
                r.infos[i].powered.push(p);
                r.infos[i].triggered.push(p);
                if mode == "log" {
                    self.log_test_block(dim, &mode, p);
                }
            } else if !signal && was {
                r.infos[i].powered.retain(|q| *q != p);
            }
        }
    }

    /// `TestBlockEntity.log`.
    fn log_test_block(&mut self, dim: DimId, mode: &str, p: Pos) {
        let message = self.test_block_message(dim, p);
        if !message.trim().is_empty() {
            info!("Test {mode} (at {}, {}, {}): {message}", p[0], p[1], p[2]);
        }
    }

    fn test_block_message(&self, dim: DimId, p: Pos) -> String {
        let Some(chunk) = self.dims[dim].regions.chunk(kiln_world::ChunkPos::of_block(p[0], p[2])) else { return String::new() };
        chunk
            .block_entity((p[0] & 15) as usize, p[1], (p[2] & 15) as usize)
            .and_then(|be| be.nbt.get("message").and_then(Tag::as_str).map(str::to_owned))
            .unwrap_or_default()
    }

    /// `BlockBasedTestInstance.run`'s per-tick check.
    fn block_based_tick(&mut self, r: &mut Runner, i: usize) {
        let dim = r.infos[i].dim;
        let of = |mode: &str, blocks: &[(Pos, String)]| -> Vec<Pos> { blocks.iter().filter(|(_, m)| m == mode).map(|(p, _)| *p).collect() };
        let accepts = of("accept", &r.infos[i].test_blocks);
        if accepts.is_empty() {
            r.infos[i].fail(tr!("test_block.error.missing", tr!("test_block.mode.accept")));
            return;
        }
        if accepts.iter().any(|p| r.infos[i].triggered.contains(p)) {
            self.succeed(r, i);
            return;
        }
        for p in of("fail", &r.infos[i].test_blocks) {
            if r.infos[i].triggered.contains(&p) {
                let message = self.test_block_message(dim, p);
                r.infos[i].fail(Text::literal(message));
                return;
            }
        }
        // Log blocks that fired were logged when they did; they are reset.
        for p in of("log", &r.infos[i].test_blocks) {
            r.infos[i].triggered.retain(|q| *q != p);
        }
    }

    // ---- listeners, in the order vanilla adds them ----

    /// A test passed: the report, the summary, then the batch.
    fn test_passed(&mut self, r: &mut Runner, i: usize) {
        self.report_passed_event(r, i);
        self.summary_event(r, i);
        // `GameTestRunner$1.testPassed`.
        let (dim, block) = (r.infos[i].dim, r.infos[i].block);
        if let Some(b) = block
            && let Some(data) = self.test_block_data(dim, b)
        {
            let layout = self.layout_of(b, &data);
            self.remove_barriers(dim, &layout, r.infos[i].def.sky_access);
        }
        self.test_completed(r, i);
    }

    /// A test failed: the report, the summary, the failed list, then the batch.
    fn test_failed(&mut self, r: &mut Runner, i: usize) {
        self.report_failed_event(r, i);
        self.summary_event(r, i);
        let id = r.infos[i].test.clone();
        let failed = &mut self.commands.gametests.last_failed;
        if !failed.contains(&id) {
            failed.push(id);
        }
        // `GameTestRunner$1.testFailed`.
        if r.halt_on_error {
            self.end_env(r);
            self.unforce_all(r.infos[i].dim);
            r.terminated = true;
        } else {
            self.test_completed(r, i);
        }
    }

    /// `GameTestRunner$1.testCompleted`: when the whole batch is done, the next one starts.
    fn test_completed(&mut self, r: &mut Runner, i: usize) {
        if r.batch_tracked.iter().all(|&j| r.infos[j].done) {
            self.unforce_all(r.infos[i].dim);
            let next = r.current + 1;
            self.run_batch(r, next);
        }
    }

    /// Every forced chunk of the level goes (vanilla lets go of all of them).
    fn unforce_all(&mut self, dim: DimId) {
        let chunks: Vec<[i32; 2]> = self.world.forced[dim].iter().copied().collect();
        self.unforce(dim, &chunks);
    }

    /// `TestSummaryDisplayer`: the summary once every tracked test is done.
    fn summary_event(&mut self, r: &mut Runner, i: usize) {
        let dim = dim_id(&r.infos[i].def.dimension).unwrap_or(r.infos[i].dim);
        if !r.summary_dims.contains(&dim) {
            r.summary_dims.push(dim);
        }
        if r.done_count() != r.tracked.len() {
            return;
        }
        let total = r.tracked.len() as i32;
        let (fr, fo) = (r.failed_required() as i32, r.failed_optional() as i32);
        let dims = r.summary_dims.clone();
        self.as_source(r.source, |s| {
            s.send_success(tr!("commands.test.summary", total).color("white"), true);
            if fr > 0 {
                s.send_failure(tr!("commands.test.summary.failed", fr));
            } else {
                s.send_success(tr!("commands.test.summary.all_required_passed").color("green"), true);
            }
            if fo > 0 {
                s.send_system_to_source(tr!("commands.test.summary.optional_failed", fo));
            }
            s.output_player_coordinates(&dims);
            s.output_test_coordinates(&dims);
        });
    }

    // ---- reports ----

    /// `ReportGameListener.testPassed`.
    fn report_passed_event(&mut self, r: &mut Runner, i: usize) {
        let rep = r.infos[i].report;
        r.reports[rep].successes += 1;
        let Report { attempts, successes } = r.reports[rep];
        let info = &r.infos[i];
        if info.has_retries() {
            self.handle_retry(r, i, true);
            return;
        }
        if !info.flaky() {
            let msg = format!("{} passed! ({}ms / {}gameticks)", info.test, info.ran_ms, info.tick);
            self.report_passed(r, i, &msg);
            return;
        }
        if successes >= info.def.required_successes {
            let msg = format!("{} passed {} times of {} attempts.", info.test, successes, attempts);
            self.report_passed(r, i, &msg);
        } else {
            let msg = format!("Flaky test {} succeeded, attempt: {} successes: {}", info.test, attempts, successes);
            self.say("green", msg);
            self.rerun(r, i);
        }
    }

    /// `ReportGameListener.testFailed`.
    fn report_failed_event(&mut self, r: &mut Runner, i: usize) {
        let rep = r.infos[i].report;
        let Report { attempts, successes } = r.reports[rep];
        if !r.infos[i].flaky() {
            self.report_failure(r, i);
            if r.infos[i].has_retries() {
                self.handle_retry(r, i, false);
            }
            return;
        }
        let def = r.infos[i].def.clone();
        let mut msg = format!("Flaky test {} failed, attempt: {}/{}", r.infos[i].test, attempts, def.max_attempts);
        if def.required_successes > 1 {
            msg.push_str(&format!(", successes: {} ({} required)", successes, def.required_successes));
        }
        self.say("yellow", msg);
        if def.max_attempts - attempts + successes >= def.required_successes {
            self.rerun(r, i);
        } else {
            let e = format!(
                "Not enough successes: {} out of {} attempts. Required successes: {}. max attempts: {}.",
                successes, attempts, def.required_successes, def.max_attempts
            );
            r.infos[i].error = Some(Failure { text: Text::literal(e), at: None, assertion: false });
            self.report_failure(r, i);
        }
    }

    /// `ReportGameListener.handleRetry`.
    fn handle_retry(&mut self, r: &mut Runner, i: usize, passed: bool) {
        let Report { attempts, successes } = r.reports[r.infos[i].report];
        let info = &r.infos[i];
        let mut progress = format!("[Run: {attempts:>4}, Ok: {successes:>4}, Fail: {:>4}", attempts - successes);
        if !info.unlimited_tries() {
            progress.push_str(&format!(", Left: {:>4}", info.tries - attempts));
        }
        progress.push(']');
        let head = format!("{} {}! {}ms", info.test, if passed { "passed" } else { "failed" }, info.ran_ms);
        let line = format!("{head:<53}{progress}");
        if passed {
            self.report_passed(r, i, &line);
        } else {
            self.say("red", line);
        }
        let info = &r.infos[i];
        if info.has_tries_left(attempts, successes) {
            self.rerun(r, i);
        }
    }

    /// `ReportGameListener.reportPassed`: the block turns green, players are told.
    fn report_passed(&mut self, r: &Runner, i: usize, message: &str) {
        let (dim, block) = (r.infos[i].dim, r.infos[i].block);
        if let Some(b) = block
            && let Some(mut data) = self.test_block_data(dim, b)
        {
            data.status = Status::Finished;
            self.set_test_block_data(dim, b, &data);
        }
        self.say("green", message.to_owned());
    }

    /// `ReportGameListener.reportFailure`: the error on the block, players told, the failure
    /// marked in the structure and logged.
    fn report_failure(&mut self, r: &Runner, i: usize) {
        let info = &r.infos[i];
        let Some(failure) = info.error.clone() else { return };
        let description = if failure.assertion {
            failure.text.clone()
        } else {
            Text::literal(format!("{}: {}", info.error_kind, crate::commands::console_text(&failure.text)))
        };
        if let (Some(b), dim) = (info.block, info.dim)
            && let Some(mut data) = self.test_block_data(dim, b)
        {
            data.error = Some(description.to_nbt());
            data.status = Status::Finished;
            if let Some(at) = failure.at {
                data.errors.push((at, description.to_nbt()));
            }
            self.set_test_block_data(dim, b, &data);
        }
        let message = crate::commands::console_text(&description);
        let prefix = if info.def.required { "" } else { "(optional)" };
        let line = format!("{prefix}{} failed! {message}", info.test);
        let color = if info.def.required { "red" } else { "yellow" };
        self.say(color, line);
        let at = info.block.map_or_else(String::new, |b| format!("{}, {}, {}", b[0], b[1], b[2]));
        if info.def.required {
            error!("{} failed at {at}! {message}", info.test);
        } else {
            warn!("(optional) {} failed at {at}. {message}", info.test);
        }
    }

    // ---- reruns ----

    /// `GameTestRunner.rerunTest`: a fresh copy of the test at the same block.
    fn rerun(&mut self, r: &mut Runner, i: usize) {
        let old = &r.infos[i];
        let mut copy = Info::new(old.test.clone(), old.def.clone(), old.extra_rotation, old.dim, (old.tries, old.halt_on_failure));
        copy.block = old.block;
        copy.report = old.report;
        let n = r.infos.len();
        r.infos.push(copy);
        // The summary's tracker follows the copy.
        r.tracked.push(n);
        r.scheduled_reruns.push(n);
        if r.stopped {
            self.run_scheduled_reruns(r);
        }
    }

    /// `GameTestRunner.runScheduledRerunTests`.
    fn run_scheduled_reruns(&mut self, r: &mut Runner) {
        if r.scheduled_reruns.is_empty() {
            r.batches.clear();
            r.stopped = true;
            return;
        }
        let scheduled = std::mem::take(&mut r.scheduled_reruns);
        info!(
            "Starting re-run of tests: {}",
            scheduled.iter().map(|&i| r.infos[i].test.clone()).collect::<Vec<_>>().join(",")
        );
        // Batched like any other tests: by environment and dimension.
        let mut groups: Vec<((String, DimId), Vec<usize>)> = Vec::new();
        for &i in &scheduled {
            let key = (r.infos[i].def.environment.clone(), r.infos[i].dim);
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.push(i),
                None => groups.push((key, vec![i])),
            }
        }
        r.batches.clear();
        for ((env, dim), all) in groups {
            for (index, chunk) in all.chunks(r.batch_size).enumerate() {
                r.batches.push(Batch { env: env.clone(), dim, index, infos: chunk.to_vec() });
            }
        }
        r.stopped = false;
        self.run_batch(r, 0);
    }
}

impl Grid {
    pub(super) fn new(first: Pos) -> Grid {
        Grid {
            first,
            next: first,
            row_min: first.map(f64::from),
            row_max: first.map(|v| f64::from(v + 1)),
            in_row: 0,
            last_batch: Vec::new(),
        }
    }
}

