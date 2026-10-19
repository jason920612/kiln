import re


def sub(p, a, b, count=1):
    s = open(p, encoding='utf-8').read()
    assert s.count(a) >= 1, (p, a[:70])
    s = s.replace(a, b, count)
    open(p, 'w', encoding='utf-8').write(s)


p = 'crates/kiln-sim/src/lib.rs'
sub(p, "mod weather;\n", "mod weather;\nmod timelines;\n")
sub(p, """    /// The End's clock (`minecraft:the_end`, the End's `default_clock`).
    end_time: i64,
    end_clock: i32,
""", """    /// The state of the overworld clock besides its total.
    overworld_run: weather::ClockRun,
    /// The other world clocks (`minecraft:the_end`, the End's `default_clock`, and the data packs' own).
    other_clocks: Vec<weather::ClockSlot>,
    /// The timelines of the world: what follows the clocks, the markers of the clocks.
    timelines: std::sync::Arc<timelines::Timelines>,
""")
sub(p, """    /// Rate, fraction and pause of the overworld and End clocks.
    clock_runs: [weather::ClockRun; 2],
""", "")
sub(p, """            end_time: 0,
            end_clock: kiln_data::synced_id("minecraft:world_clock", "minecraft:the_end").expect("end clock"),
""", """            overworld_run: Default::default(),
            other_clocks: Sim::load_clocks(&saved_clocks),
            timelines: timelines::Timelines::load(&roots),
""")
sub(p, """            clock_runs: Default::default(),
""", "")
sub(p, """        let level = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.state());
""", """        let level = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.state());
        let saved_clocks = storage.as_ref().filter(|s| s.level.exists()).map(|s| s.level.clocks()).unwrap_or_default();
""")

# sleep
p = 'crates/kiln-sim/src/sleep.rs'
sub(p, """            if self.rule_bool("minecraft:advance_time") && crate::dim_type(dim).overworld_clock() {
                if let Some(to) = crate::weather::marker_move(self.day_time, 0) {
                    self.day_time = to;
                    self.clock_runs[0].partial = 0.0;
                }
                let pkt = self.time_packet();
                self.broadcast(pkt);
            }""", """            if self.rule_bool("minecraft:advance_time")
                && let Some(clock) = crate::dim_type(dim).default_clock
            {
                // `ClockManager.moveToTimeMarker(clock, WAKE_UP_FROM_SLEEP)`: the level's clock goes to the next moment that marker names.
                self.move_to_wake_up_marker(clock);
                let pkt = self.time_packet();
                self.broadcast(pkt);
            }""")
sub(p, """    /// `ServerLevel.wakeUpAllPlayers`.""", """    /// The clock moves to the next occurrence of the marker `minecraft:wake_up_from_sleep` of its timelines (nothing if it has none or the
    /// clock stands at it).
    fn move_to_wake_up_marker(&mut self, clock: &str) {
        let Some((_, period, ticks, _)) = self.time_markers_of(clock).into_iter().find(|(id, ..)| id == "minecraft:wake_up_from_sleep") else { return };
        let Some(total) = self.clock_total_of(clock) else { return };
        if let Some(to) = crate::weather::marker_move_in(total, ticks, period.unwrap_or(24000)) {
            if clock == "minecraft:overworld" {
                self.day_time = to;
                self.overworld_run.partial = 0.0;
            } else if let Some(c) = self.other_clocks.iter_mut().find(|c| c.name == clock) {
                c.total = to;
                c.run.partial = 0.0;
            }
        }
    }

    /// `ServerLevel.wakeUpAllPlayers`.""")

# command_data
p = 'crates/kiln-sim/src/command_data.rs'
sub(p, """        match clock.as_str() {
            "minecraft:overworld" => self.sim.day_time,
            "minecraft:the_end" => self.sim.end_time,
            _ => 0,
        }""", """        self.sim.clock_total_of(clock.as_str()).unwrap_or(0)""")

# commands
p = 'crates/kiln-sim/src/commands.rs'
sub(p, """    fn time_markers(&self, clock: Option<&Identifier>) -> Vec<String> {
        crate::weather::time_markers(clock.map_or(OVERWORLD, Identifier::as_str)).iter().map(|(m, _)| (*m).to_owned()).collect()
    }

    fn timelines(&self, clock: Option<&Identifier>) -> Vec<String> {
        crate::weather::timelines(clock.map_or(OVERWORLD, Identifier::as_str)).iter().map(|(t, _)| (*t).to_owned()).collect()
    }""", """    fn time_markers(&self, clock: Option<&Identifier>) -> Vec<String> {
        let clock = clock.map_or_else(|| self.source_default_clock(), |c| c.as_str().to_owned());
        self.time_markers_of(&clock).into_iter().map(|(m, ..)| m).collect()
    }

    fn timelines(&self, clock: Option<&Identifier>) -> Vec<String> {
        let clock = clock.map_or_else(|| self.source_default_clock(), |c| c.as_str().to_owned());
        self.timelines_of(&clock).into_iter().map(|(t, _)| t).collect()
    }""")

# weather: default clock of the source's level, for suggestions
p = 'crates/kiln-sim/src/weather.rs'
sub(p, """    /// The time markers of a clock (from its timelines)""", """    /// The default clock of the level the command's source is in (the overworld's when it has none).
    pub(crate) fn source_default_clock(&self) -> String {
        let dim = crate::dim_id(kiln_command::host::Source::dimension(self)).unwrap_or(OVERWORLD_ID);
        crate::dim_type(dim).default_clock.unwrap_or("minecraft:overworld").to_owned()
    }

    /// The time markers of a clock (from its timelines)""")
print('done')
