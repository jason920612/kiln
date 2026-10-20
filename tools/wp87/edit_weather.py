import sys

p = 'crates/kiln-sim/src/weather.rs'
s = open(p, encoding='utf-8').read()
a = s.index("/// Clocks Kiln runs, by `minecraft:world_clock` id.")
b = s.index("/// `TimeCommand.wrapTime`.")
new_head = '''/// `ClockTimeMarker.occursAt` and `resolveTimeToMoveTo` for a marker `ticks` into a timeline of `period` ticks (24000 for the day).
pub(crate) fn marker_move(total: i64, ticks: i64) -> Option<i64> {
    marker_move_in(total, ticks, 24000)
}

/// [`marker_move`] for a timeline that repeats every `period` ticks.
pub(crate) fn marker_move_in(total: i64, ticks: i64, period: i64) -> Option<i64> {
    if total.rem_euclid(period) == ticks {
        return None;
    }
    let d = ticks - total.rem_euclid(period);
    Some(total + if d > 0 { d } else { period + d })
}

/// A world clock besides the overworld's (`ServerClockManager.ServerClockInstance`): the registry entry, its network id and its state.
#[derive(Clone, Debug)]
pub(crate) struct ClockSlot {
    pub name: &'static str,
    pub net_id: i32,
    pub total: i64,
    pub run: ClockRun,
}

'''
s = s[:a] + new_head + s[b:]
a = s.index("impl Sim {\n    /// Advances the world clocks")
b = s.index("#[cfg(test)]\nmod tests {")
new_impl = open('tools/wp87/weather_clock_impl.rs.txt', encoding='utf-8').read()
s = s[:a] + new_impl + s[b:]
open(p, 'w', encoding='utf-8').write(s)
print("ok")
