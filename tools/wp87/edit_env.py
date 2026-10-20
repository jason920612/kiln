def sub(p, a, b, count=1):
    s = open(p, encoding='utf-8').read()
    assert s.count(a) >= 1, (p, a[:70])
    s = s.replace(a, b, count)
    open(p, 'w', encoding='utf-8').write(s)


sub('crates/kiln-storage/src/lib.rs', "pub use level::{LevelState,", "pub use level::{ClockSave, LevelState,")

# MobRules: new timeline-driven fields
p = 'crates/kiln-sim/src/mobs.rs'
sub(p, """    /// The `minecraft:gameplay/creaking_active` value (the overworld's night).
    pub creaking_active: bool,
""", """    /// The `minecraft:gameplay/creaking_active` value (the overworld's night).
    pub creaking_active: bool,
    /// The `minecraft:gameplay/eyeblossom_open` value; none when no timeline of the level has the track.
    pub eyeblossom_open: Option<bool>,
    /// The timelines' `minecraft:gameplay/bees_stay_in_hive` (the weather layer adds its own).
    pub bees_stay_in_hive: bool,
    /// The `minecraft:gameplay/turtle_egg_hatch_chance` value.
    pub turtle_egg_hatch_chance: f32,
""")
sub(p, """            monsters_burn: true,
            creaking_active: false,
""", """            monsters_burn: true,
            creaking_active: false,
            eyeblossom_open: None,
            bees_stay_in_hive: false,
            turtle_egg_hatch_chance: 0.002,
""")

p = 'crates/kiln-sim/src/lib.rs'
sub(p, """                sky_darken: weather::sky_darken(dim, self.day_time, &self.level_weather[dim]),
                monsters_burn: mobs::monsters_burn(self.day_time),
                creaking_active: dim_type(dim).has_timeline("minecraft:day") && mobs::creaking_active(self.day_time),
""", """                sky_darken: self.sky_darken_of(dim),
                monsters_burn: self.timeline_bool(dim, "minecraft:gameplay/monsters_burn", false),
                creaking_active: self.timeline_bool(dim, "minecraft:gameplay/creaking_active", false),
                eyeblossom_open: self.timelines.has_track(dim_type(dim).timelines, "minecraft:gameplay/eyeblossom_open").then(|| self.timeline_bool(dim, "minecraft:gameplay/eyeblossom_open", false)),
                bees_stay_in_hive: self.timeline_bool(dim, "minecraft:gameplay/bees_stay_in_hive", false),
                turtle_egg_hatch_chance: self.timeline_float(dim, "minecraft:gameplay/turtle_egg_hatch_chance", 0.002),
""")

p = 'crates/kiln-sim/src/weather.rs'
sub(p, """    /// The default clock of the level the command's source is in""", """    /// The boolean attribute `attribute` of level `dim`: its dimension type's timelines change `base` (the level's clocks drive them).
    pub(crate) fn timeline_bool(&self, dim: crate::DimId, attribute: &str, base: bool) -> bool {
        let total = |clock: &str| self.clock_total_of(clock).unwrap_or(0);
        self.timelines.bool_attr(crate::dim_type(dim).timelines, attribute, base, &total)
    }

    /// The float attribute `attribute` of level `dim`.
    pub(crate) fn timeline_float(&self, dim: crate::DimId, attribute: &str, base: f32) -> f32 {
        let total = |clock: &str| self.clock_total_of(clock).unwrap_or(0);
        self.timelines.float_attr(crate::dim_type(dim).timelines, attribute, base, &total)
    }

    /// `Level.updateSkyBrightness` for level `dim`: 15 less the `minecraft:gameplay/sky_light_level` its timelines make (0 when none of
    /// them sets it: no day timeline modifies the sky light of the nether and the End), then the weather layer.
    pub(crate) fn sky_darken_of(&self, dim: crate::DimId) -> i32 {
        const SKY: &str = "minecraft:gameplay/sky_light_level";
        if !self.timelines.has_track(crate::dim_type(dim).timelines, SKY) {
            return 0;
        }
        let level = weather_layer(self.timeline_float(dim, SKY, 15.0), &self.level_weather[dim]);
        (15.0 - level) as i32
    }

    /// The default clock of the level the command's source is in""")
sub(p, """pub(crate) fn sky_light_level(day_time: i64, weather: &LevelWeather) -> f32 {
    let mut v = 15.0 * sky_light_factor(day_time);
    let thunder""", """pub(crate) fn sky_light_level(day_time: i64, weather: &LevelWeather) -> f32 {
    weather_layer(15.0 * sky_light_factor(day_time), weather)
}

/// `WeatherAttributes`: the `sky_light_level` `v` of the timelines changed by the rain and the thunder of the level.
pub(crate) fn weather_layer(mut v: f32, weather: &LevelWeather) -> f32 {
    let thunder""")

sub('crates/kiln-sim/src/village.rs', "crate::weather::sky_darken(OVERWORLD_ID, self.day_time, &self.level_weather[OVERWORLD_ID])", "self.sky_darken_of(OVERWORLD_ID)")
sub('crates/kiln-sim/src/raid.rs', "crate::weather::sky_darken(OVERWORLD, self.day_time, &self.level_weather[OVERWORLD])", "self.sky_darken_of(OVERWORLD)")
sub('crates/kiln-sim/src/blocks.rs', """    fn eyeblossom_open(&self, _pos: BlockPos) -> Option<bool> {
        crate::dim_type(self.env.dim).has_timeline("minecraft:day").then(|| (12600..23401).contains(&self.env.mobs.day_time.rem_euclid(24000)))
    }""", """    fn eyeblossom_open(&self, _pos: BlockPos) -> Option<bool> {
        self.env.mobs.eyeblossom_open
    }""")
sub('crates/kiln-sim/src/blocks.rs', """        let dawn = crate::dim_type(self.env.dim).has_timeline("minecraft:day") && (21062..21905).contains(&self.env.mobs.day_time.rem_euclid(24000));
        if dawn { 1.0 } else { 0.002 }""", """        self.env.mobs.turtle_egg_hatch_chance""")
sub('crates/kiln-sim/src/beehive.rs', """    crate::dim_type(env.dim).has_timeline("minecraft:day") && ((12542..23460).contains(&env.mobs.day_time.rem_euclid(24000)) || env.weather.weather.weather_layer)""", """    env.mobs.bees_stay_in_hive || env.weather.weather.weather_layer""")
print('ok')
