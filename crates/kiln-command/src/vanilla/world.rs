//! Level-wide commands: `worldborder` (`WorldBorderCommand`), `tick` (`TickCommand`),
//! `forceload` (`ForceLoadCommand`), `random` (`RandomCommand`), `locate` (`LocateCommand`),
//! `place` (`PlaceCommand`), `fillbiome` (`FillBiomeCommand`) and `spreadplayers`
//! (`SpreadPlayersCommand`). The host keeps the state; these check arguments in vanilla's
//! order and send its feedback.

use super::blocks::loaded_block_pos;
use super::{LEVEL_ADMINS, LEVEL_GAMEMASTERS};
use crate::arguments::{ArgumentType, ArgumentValue, ResourceOrTag};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{BorderChange, Host, Located, Placement, Teleport, TickRateAction};
use crate::selector::SelectorTarget;
use crate::text::ClickEvent;
use crate::tr;

type Result<T> = std::result::Result<T, CommandError>;

/// `WorldBorder.MAX_SIZE`.
const MAX_SIZE: f64 = 59_999_968.0;
/// `WorldBorder.MAX_CENTER_COORDINATE`.
const MAX_CENTER: f64 = 29_999_984.0;

/// `Double.toString`: the shortest repr, plain between 10^-3 and 10^7, else `d.dddE±n`.
pub fn java_double(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    let a = v.abs();
    if (1e-3..1e7).contains(&a) {
        let s = format!("{v}");
        if s.contains('.') { s } else { format!("{s}.0") }
    } else {
        let s = format!("{v:e}");
        let (mantissa, exp) = s.split_once('e').expect("exponent");
        let mantissa = if mantissa.contains('.') { mantissa.to_owned() } else { format!("{mantissa}.0") };
        format!("{mantissa}E{exp}")
    }
}

/// `Float.toString`.
pub fn java_float(v: f32) -> String {
    if v.is_finite() && v != 0.0 {
        let a = v.abs();
        if (1e-3..1e7).contains(&a) {
            let s = format!("{v}");
            return if s.contains('.') { s } else { format!("{s}.0") };
        }
        let s = format!("{v:e}");
        let (mantissa, exp) = s.split_once('e').expect("exponent");
        let mantissa = if mantissa.contains('.') { mantissa.to_owned() } else { format!("{mantissa}.0") };
        return format!("{mantissa}E{exp}");
    }
    java_double(v as f64)
}

/// `String.format(Locale.ROOT, "%.<digits>f", v)`: Java rounds the shortest decimal repr of
/// the value half up.
pub fn java_fixed(v: f64, digits: usize) -> String {
    if !v.is_finite() {
        return java_double(v);
    }
    let negative = v.is_sign_negative() && v != 0.0;
    let plain = format!("{}", v.abs());
    let (int, frac) = plain.split_once('.').unwrap_or((&plain, ""));
    let mut all: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
    let int_len = int.len();
    let keep = int_len + digits;
    while all.len() < keep {
        all.push(0);
    }
    let round_up = all.get(keep).is_some_and(|&d| d >= 5);
    all.truncate(keep);
    let mut carry = round_up;
    for d in all.iter_mut().rev() {
        if !carry {
            break;
        }
        if *d == 9 {
            *d = 0;
        } else {
            *d += 1;
            carry = false;
        }
    }
    let mut int_digits: Vec<u8> = all[..int_len].to_vec();
    if carry {
        int_digits.insert(0, 1);
    }
    let mut s = String::new();
    // Java keeps the sign of negative values, even those that round to zero.
    if negative {
        s.push('-');
    }
    s.extend(int_digits.iter().map(|d| (d + b'0') as char));
    if digits > 0 {
        s.push('.');
        s.extend(all[int_len..].iter().map(|d| (d + b'0') as char));
    }
    s
}

/// `Mth.floor`.
fn floor(v: f64) -> i32 {
    v.floor() as i32
}

// ---- worldborder --------------------------------------------------------------------------

fn format_ticks_to_seconds(ticks: i64) -> String {
    java_fixed(ticks as f64 / 20.0, 2)
}

fn border_set_size<S: Host>(s: &mut S, distance: f64, time: i64) -> Result<i32> {
    let dim = s.dimension().to_owned();
    let size = s.world_border(&dim).size;
    if size == distance {
        return Err(CommandError::new(tr!("commands.worldborder.set.failed.nochange")));
    }
    if distance < 1.0 {
        return Err(CommandError::new(tr!("commands.worldborder.set.failed.small")));
    }
    if distance > MAX_SIZE {
        return Err(CommandError::new(tr!("commands.worldborder.set.failed.big", java_double(MAX_SIZE))));
    }
    let text = java_fixed(distance, 1);
    if time > 0 {
        s.change_world_border(&dim, BorderChange::Lerp { from: size, to: distance, ticks: time });
        let key = if distance > size { "commands.worldborder.set.grow" } else { "commands.worldborder.set.shrink" };
        s.send_success(tr!(key, text, format_ticks_to_seconds(time)), true);
    } else {
        s.change_world_border(&dim, BorderChange::Size(distance));
        s.send_success(tr!("commands.worldborder.set.immediate", text), true);
    }
    Ok((distance - size) as i32)
}

fn border_center<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let dim = s.dimension().to_owned();
    let [x, _, z] = s.stack().resolve(c.coordinates("pos"));
    let (x, z) = (x as f32, z as f32);
    let border = s.world_border(&dim);
    if border.center[0] == x as f64 && border.center[1] == z as f64 {
        return Err(CommandError::new(tr!("commands.worldborder.center.failed")));
    }
    if x.abs() as f64 > MAX_CENTER || z.abs() as f64 > MAX_CENTER {
        return Err(CommandError::new(tr!("commands.worldborder.set.failed.far", java_double(MAX_CENTER))));
    }
    s.change_world_border(&dim, BorderChange::Center(x as f64, z as f64));
    s.send_success(tr!("commands.worldborder.center.success", java_fixed(x as f64, 2), java_fixed(z as f64, 2)), true);
    Ok(0)
}

pub fn worldborder<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let distance = || ArgumentType::Double { min: -MAX_SIZE, max: MAX_SIZE };
    d.register(
        literal("worldborder")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("add").then(
                    argument("distance", distance())
                        .executes(|c, s: &mut S| {
                            let dim = s.dimension().to_owned();
                            let size = s.world_border(&dim).size;
                            border_set_size(s, size + c.double("distance"), 0)
                        })
                        .then(argument("time", ArgumentType::time_min(0)).executes(|c, s: &mut S| {
                            let dim = s.dimension().to_owned();
                            let b = s.world_border(&dim);
                            border_set_size(s, b.size + c.double("distance"), b.lerp_time + c.time("time") as i64)
                        })),
                ),
            )
            .then(
                literal("set").then(
                    argument("distance", distance())
                        .executes(|c, s: &mut S| border_set_size(s, c.double("distance"), 0))
                        .then(
                            argument("time", ArgumentType::time_min(0))
                                .executes(|c, s: &mut S| border_set_size(s, c.double("distance"), c.time("time") as i64)),
                        ),
                ),
            )
            .then(literal("center").then(argument("pos", ArgumentType::vec2()).executes(border_center)))
            .then(
                literal("damage")
                    .then(literal("amount").then(
                        argument("damagePerBlock", ArgumentType::float_range(0.0, f32::MAX)).executes(|c, s: &mut S| {
                            let dim = s.dimension().to_owned();
                            let v = c.float("damagePerBlock");
                            if s.world_border(&dim).damage_per_block == v as f64 {
                                return Err(CommandError::new(tr!("commands.worldborder.damage.amount.failed")));
                            }
                            s.change_world_border(&dim, BorderChange::DamagePerBlock(v as f64));
                            s.send_success(tr!("commands.worldborder.damage.amount.success", java_fixed(v as f64, 2)), true);
                            Ok(v as i32)
                        }),
                    ))
                    .then(literal("buffer").then(
                        argument("distance", ArgumentType::float_range(0.0, f32::MAX)).executes(|c, s: &mut S| {
                            let dim = s.dimension().to_owned();
                            let v = c.float("distance");
                            if s.world_border(&dim).safe_zone == v as f64 {
                                return Err(CommandError::new(tr!("commands.worldborder.damage.buffer.failed")));
                            }
                            s.change_world_border(&dim, BorderChange::SafeZone(v as f64));
                            s.send_success(tr!("commands.worldborder.damage.buffer.success", java_fixed(v as f64, 2)), true);
                            Ok(v as i32)
                        }),
                    )),
            )
            .then(literal("get").executes(|_, s: &mut S| {
                let dim = s.dimension().to_owned();
                let size = s.world_border(&dim).size;
                s.send_success(tr!("commands.worldborder.get", java_fixed(size, 0)), false);
                Ok(floor(size + 0.5))
            }))
            .then(
                literal("warning")
                    .then(literal("distance").then(argument("distance", ArgumentType::integer_min(0)).executes(
                        |c, s: &mut S| {
                            let dim = s.dimension().to_owned();
                            let v = c.integer("distance");
                            if s.world_border(&dim).warning_blocks == v {
                                return Err(CommandError::new(tr!("commands.worldborder.warning.distance.failed")));
                            }
                            s.change_world_border(&dim, BorderChange::WarningBlocks(v));
                            s.send_success(tr!("commands.worldborder.warning.distance.success", v), true);
                            Ok(v)
                        },
                    )))
                    .then(literal("time").then(argument("time", ArgumentType::time_min(0)).executes(|c, s: &mut S| {
                        let dim = s.dimension().to_owned();
                        let v = c.time("time");
                        if s.world_border(&dim).warning_time == v {
                            return Err(CommandError::new(tr!("commands.worldborder.warning.time.failed")));
                        }
                        s.change_world_border(&dim, BorderChange::WarningTime(v));
                        s.send_success(tr!("commands.worldborder.warning.time.success", format_ticks_to_seconds(v as i64)), true);
                        Ok(v)
                    }))),
            ),
    );
}

// ---- tick ---------------------------------------------------------------------------------

/// `TickCommand.nanosToMilisString`: float division, one decimal.
fn nanos_to_millis(nanos: i64) -> String {
    java_fixed((nanos as f32 / 1_000_000f32) as f64, 1)
}

fn tick_query<S: Host>(s: &mut S) -> Result<i32> {
    let info = s.tick_rate();
    let average = nanos_to_millis(info.average_tick_nanos);
    let rate = java_fixed(info.rate as f64, 1);
    if info.sprinting {
        s.send_success(tr!("commands.tick.status.sprinting"), false);
        s.send_success(tr!("commands.tick.query.rate.sprinting", rate.clone(), average), false);
    } else {
        if info.frozen {
            s.send_success(tr!("commands.tick.status.frozen"), false);
        } else if info.nanos_per_tick < info.average_tick_nanos {
            s.send_success(tr!("commands.tick.status.lagging"), false);
        } else {
            s.send_success(tr!("commands.tick.status.running"), false);
        }
        let target = nanos_to_millis(info.nanos_per_tick);
        s.send_success(tr!("commands.tick.query.rate.running", rate, average, target), false);
    }
    let mut times = info.tick_times.clone();
    times.sort_unstable();
    let n = times.len();
    if n > 0 {
        let p50 = nanos_to_millis(times[n / 2]);
        let p95 = nanos_to_millis(times[(n as f64 * 0.95) as usize]);
        let p99 = nanos_to_millis(times[(n as f64 * 0.99) as usize]);
        s.send_success(tr!("commands.tick.query.percentiles", p50, p95, p99, n as i32), false);
    }
    Ok(info.rate as i32)
}

fn tick_step<S: Host>(s: &mut S, ticks: i32) -> Result<i32> {
    if !s.change_tick_rate(TickRateAction::Step(ticks)) {
        return Err(CommandError::new(tr!("commands.tick.step.fail")));
    }
    s.send_success(tr!("commands.tick.step.success", ticks), true);
    Ok(1)
}

fn tick_freeze<S: Host>(s: &mut S, frozen: bool) -> Result<i32> {
    s.change_tick_rate(TickRateAction::Freeze(frozen));
    let key = if frozen { "commands.tick.status.frozen" } else { "commands.tick.status.running" };
    s.send_success(tr!(key), true);
    Ok(frozen as i32)
}

pub fn tick<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("tick")
            .requires(LEVEL_ADMINS)
            .then(literal("query").executes(|_, s: &mut S| tick_query(s)))
            .then(literal("rate").then(
                argument("rate", ArgumentType::float_range(1.0, 10000.0))
                    .suggests_server(|_, _: &S, b| b.suggest("20"))
                    .executes(|c, s: &mut S| {
                        let rate = c.float("rate");
                        s.change_tick_rate(TickRateAction::Rate(rate));
                        s.send_success(tr!("commands.tick.rate.success", java_fixed(rate as f64, 1)), true);
                        Ok(rate as i32)
                    }),
            ))
            .then(
                literal("step")
                    .executes(|_, s: &mut S| tick_step(s, 1))
                    .then(literal("stop").executes(|_, s: &mut S| {
                        if !s.change_tick_rate(TickRateAction::StopStepping) {
                            return Err(CommandError::new(tr!("commands.tick.step.stop.fail")));
                        }
                        s.send_success(tr!("commands.tick.step.stop.success"), true);
                        Ok(1)
                    }))
                    .then(
                        argument("time", ArgumentType::time_min(1))
                            .suggests_server(|_, _: &S, b| {
                                b.suggest("1t");
                                b.suggest("1s");
                            })
                            .executes(|c, s: &mut S| tick_step(s, c.time("time"))),
                    ),
            )
            .then(
                literal("sprint")
                    .then(literal("stop").executes(|_, s: &mut S| {
                        if !s.change_tick_rate(TickRateAction::StopSprinting) {
                            return Err(CommandError::new(tr!("commands.tick.sprint.stop.fail")));
                        }
                        s.send_success(tr!("commands.tick.sprint.stop.success"), true);
                        Ok(1)
                    }))
                    .then(
                        argument("time", ArgumentType::time_min(1))
                            .suggests_server(|_, _: &S, b| {
                                b.suggest("60s");
                                b.suggest("1d");
                                b.suggest("3d");
                            })
                            .executes(|c, s: &mut S| {
                                if s.change_tick_rate(TickRateAction::Sprint(c.time("time"))) {
                                    s.send_success(tr!("commands.tick.sprint.stop.success"), true);
                                }
                                s.send_success(tr!("commands.tick.status.sprinting"), true);
                                Ok(1)
                            }),
                    ),
            )
            .then(literal("unfreeze").executes(|_, s: &mut S| tick_freeze(s, false)))
            .then(literal("freeze").executes(|_, s: &mut S| tick_freeze(s, true))),
    );
}

// ---- forceload ----------------------------------------------------------------------------

/// `ChunkPos.toString`.
fn chunk_text(c: [i32; 2]) -> String {
    format!("[{}, {}]", c[0], c[1])
}

/// `ChunkPos.pack`.
fn pack(c: [i32; 2]) -> i64 {
    (c[0] as i64 & 0xFFFF_FFFF) | ((c[1] as i64 & 0xFFFF_FFFF) << 32)
}

/// `ColumnPosArgument.getColumnPos`.
fn column<S: Host>(c: &CommandContext<S>, s: &S, name: &str) -> [i32; 2] {
    let [x, _, z] = s.stack().resolve_block(c.coordinates(name));
    [x, z]
}

fn change_force_load<S: Host>(s: &mut S, from: [i32; 2], to: [i32; 2], add: bool) -> Result<i32> {
    let (min_x, min_z) = (from[0].min(to[0]), from[1].min(to[1]));
    let (max_x, max_z) = (from[0].max(to[0]), from[1].max(to[1]));
    if min_x < -30_000_000 || min_z < -30_000_000 || max_x >= 30_000_000 || max_z >= 30_000_000 {
        return Err(CommandError::pos_out_of_world());
    }
    let (cx0, cz0, cx1, cz1) = (min_x >> 4, min_z >> 4, max_x >> 4, max_z >> 4);
    let count = (cx1 as i64 - cx0 as i64 + 1) * (cz1 as i64 - cz0 as i64 + 1);
    if count > 256 {
        return Err(CommandError::new(tr!("commands.forceload.toobig", 256, count)));
    }
    let dim = s.dimension().to_owned();
    let (mut changed, mut only) = (0, None);
    for x in cx0..=cx1 {
        for z in cz0..=cz1 {
            if s.set_chunk_forced(&dim, [x, z], add) {
                changed += 1;
                only = if changed == 1 { Some([x, z]) } else { None };
            }
        }
    }
    if changed == 0 {
        let key = if add { "commands.forceload.added.failure" } else { "commands.forceload.removed.failure" };
        return Err(CommandError::new(tr!(key)));
    }
    let verb = if add { "added" } else { "removed" };
    let text = match only {
        Some(c) => tr!(format!("commands.forceload.{verb}.single"), chunk_text(c), dim.clone()),
        None => tr!(
            format!("commands.forceload.{verb}.multiple"),
            changed,
            dim.clone(),
            chunk_text([cx0, cz0]),
            chunk_text([cx1, cz1])
        ),
    };
    s.send_success(text, true);
    Ok(changed)
}

pub fn forceload<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("forceload")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("add").then(
                    argument("from", ArgumentType::ColumnPos)
                        .executes(|c, s: &mut S| {
                            let from = column(c, s, "from");
                            change_force_load(s, from, from, true)
                        })
                        .then(argument("to", ArgumentType::ColumnPos).executes(|c, s: &mut S| {
                            let (from, to) = (column(c, s, "from"), column(c, s, "to"));
                            change_force_load(s, from, to, true)
                        })),
                ),
            )
            .then(
                literal("remove")
                    .then(
                        argument("from", ArgumentType::ColumnPos)
                            .executes(|c, s: &mut S| {
                                let from = column(c, s, "from");
                                change_force_load(s, from, from, false)
                            })
                            .then(argument("to", ArgumentType::ColumnPos).executes(|c, s: &mut S| {
                                let (from, to) = (column(c, s, "from"), column(c, s, "to"));
                                change_force_load(s, from, to, false)
                            })),
                    )
                    .then(literal("all").executes(|_, s: &mut S| {
                        let dim = s.dimension().to_owned();
                        for c in s.forced_chunks(&dim) {
                            s.set_chunk_forced(&dim, c, false);
                        }
                        s.send_success(tr!("commands.forceload.removed.all", dim), true);
                        Ok(0)
                    })),
            )
            .then(
                literal("query")
                    .executes(|_, s: &mut S| {
                        let dim = s.dimension().to_owned();
                        let mut forced = s.forced_chunks(&dim);
                        let n = forced.len() as i32;
                        if n == 0 {
                            s.send_failure(tr!("commands.forceload.added.none", dim));
                            return Ok(0);
                        }
                        forced.sort_unstable_by_key(|&c| pack(c));
                        let list = forced.iter().map(|&c| chunk_text(c)).collect::<Vec<_>>().join(", ");
                        let text = if n == 1 {
                            tr!("commands.forceload.list.single", dim, list)
                        } else {
                            tr!("commands.forceload.list.multiple", n, dim, list)
                        };
                        s.send_success(text, false);
                        Ok(n)
                    })
                    .then(argument("pos", ArgumentType::ColumnPos).executes(|c, s: &mut S| {
                        let [x, z] = column(c, s, "pos");
                        let chunk = [x >> 4, z >> 4];
                        let dim = s.dimension().to_owned();
                        if s.forced_chunks(&dim).contains(&chunk) {
                            s.send_success(tr!("commands.forceload.query.success", chunk_text(chunk), dim), false);
                            Ok(1)
                        } else {
                            Err(CommandError::new(tr!("commands.forceload.query.failure", chunk_text(chunk), dim)))
                        }
                    })),
            ),
    );
}

// ---- random -------------------------------------------------------------------------------

fn random_sample<S: Host>(c: &CommandContext<S>, s: &mut S, roll: bool) -> Result<i32> {
    let range = c.int_range("range");
    let sequence = c.get("sequence").map(|_| c.identifier("sequence").clone());
    let min = range.min.unwrap_or(i32::MIN);
    let max = range.max.unwrap_or(i32::MAX);
    let span = max as i64 - min as i64;
    if span == 0 {
        return Err(CommandError::new(tr!("commands.random.error.range_too_small")));
    }
    if span >= i32::MAX as i64 {
        return Err(CommandError::new(tr!("commands.random.error.range_too_large")));
    }
    let value = s.random_between(sequence.as_ref(), min, max);
    if roll {
        let name = s.source_name();
        s.broadcast_system_message(tr!("commands.random.roll", name, value, min, max));
    } else {
        s.send_success(tr!("commands.random.sample.success", value), false);
    }
    Ok(value)
}

fn random_reset<S: Host>(c: &CommandContext<S>, s: &mut S, params: Option<(i32, bool, bool)>) -> Result<i32> {
    let id = c.identifier("sequence").clone();
    s.reset_random_sequence(&id, params);
    s.send_success(tr!("commands.random.reset.success", id.to_string()), false);
    Ok(1)
}

fn random_reset_all<S: Host>(s: &mut S, defaults: Option<(i32, bool, bool)>) -> Result<i32> {
    let n = s.clear_random_sequences(defaults);
    s.send_success(tr!("commands.random.reset.all.success", n), false);
    Ok(n)
}

pub fn random<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let sample_tree = |name: &'static str, roll: bool| {
        literal(name).then(
            argument("range", ArgumentType::IntRange)
                .executes(move |c, s: &mut S| random_sample(c, s, roll))
                .then(
                    argument("sequence", ArgumentType::ResourceLocation)
                        .suggests_server(|_, s: &S, b| {
                            let ids = s.random_sequence_ids();
                            b.suggest_resources(ids.iter().map(String::as_str), "");
                        })
                        .requires(LEVEL_GAMEMASTERS)
                        .executes(move |c, s: &mut S| random_sample(c, s, roll)),
                ),
        )
    };
    let seeded = |c: &CommandContext<S>, world: bool, id: bool| (c.integer("seed"), world, id);
    d.register(
        literal("random")
            .then(sample_tree("value", false))
            .then(sample_tree("roll", true))
            .then(
                literal("reset")
                    .requires(LEVEL_GAMEMASTERS)
                    .then(
                        literal("*").executes(|_, s: &mut S| random_reset_all(s, None)).then(
                            argument("seed", ArgumentType::integer())
                                .executes(move |c, s: &mut S| random_reset_all(s, Some(seeded(c, true, true))))
                                .then(
                                    argument("includeWorldSeed", ArgumentType::Bool)
                                        .executes(move |c, s: &mut S| {
                                            random_reset_all(s, Some(seeded(c, c.bool("includeWorldSeed"), true)))
                                        })
                                        .then(argument("includeSequenceId", ArgumentType::Bool).executes(
                                            move |c, s: &mut S| {
                                                let p = seeded(c, c.bool("includeWorldSeed"), c.bool("includeSequenceId"));
                                                random_reset_all(s, Some(p))
                                            },
                                        )),
                                ),
                        ),
                    )
                    .then(
                        argument("sequence", ArgumentType::ResourceLocation)
                            .suggests_server(|_, s: &S, b| {
                                let ids = s.random_sequence_ids();
                                b.suggest_resources(ids.iter().map(String::as_str), "");
                            })
                            .executes(|c, s: &mut S| random_reset(c, s, None))
                            .then(
                                argument("seed", ArgumentType::integer())
                                    .executes(move |c, s: &mut S| random_reset(c, s, Some(seeded(c, true, true))))
                                    .then(
                                        argument("includeWorldSeed", ArgumentType::Bool)
                                            .executes(move |c, s: &mut S| {
                                                random_reset(c, s, Some(seeded(c, c.bool("includeWorldSeed"), true)))
                                            })
                                            .then(argument("includeSequenceId", ArgumentType::Bool).executes(
                                                move |c, s: &mut S| {
                                                    let p =
                                                        seeded(c, c.bool("includeWorldSeed"), c.bool("includeSequenceId"));
                                                    random_reset(c, s, Some(p))
                                                },
                                            )),
                                    ),
                            ),
                    ),
            ),
    );
}

// ---- locate -------------------------------------------------------------------------------

/// `LocateCommand.showLocateResult`: the coordinates are green, click to teleport.
fn show_located<S: Host>(s: &mut S, origin: [i32; 3], found: &Located, printable: String, key: &str, absolute_y: bool) -> i32 {
    let [x, y, z] = found.pos;
    let distance = if absolute_y {
        let d = [(origin[0] - x) as f64, (origin[1] - y) as f64, (origin[2] - z) as f64];
        ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f32).sqrt().floor() as i32
    } else {
        let (dx, dz) = (x - origin[0], z - origin[2]);
        ((dx * dx + dz * dz) as f32).sqrt().floor() as i32
    };
    let y_text = if absolute_y { y.to_string() } else { "~".to_owned() };
    let coords = tr!("chat.coordinates", x, y_text.clone(), z)
        .bracketed()
        .color("green")
        .click(ClickEvent::SuggestCommand(format!("/tp @s {x} {y_text} {z}")))
        .hover(tr!("chat.coordinates.tooltip"));
    s.send_success(tr!(key, printable, coords, distance), false);
    distance
}

/// `ResourceOrTagArgument.Result.asPrintable`, with the found entry after a tag.
fn printable(arg: &ResourceOrTag, found: &str) -> String {
    match arg {
        ResourceOrTag::Resource(id) => id.to_string(),
        ResourceOrTag::Tag(id) => format!("#{id} ({found})"),
    }
}

fn arg_printable(arg: &ResourceOrTag) -> String {
    match arg {
        ResourceOrTag::Resource(id) => id.to_string(),
        ResourceOrTag::Tag(id) => format!("#{id}"),
    }
}

fn origin<S: Host>(s: &S) -> [i32; 3] {
    s.origin().map(|v| v.floor() as i32)
}

pub fn locate<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("locate")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("structure").then(
                argument("structure", ArgumentType::ResourceOrTagKey { registry: "minecraft:worldgen/structure" }).executes(
                    |c, s: &mut S| {
                        let arg = c.resource_or_tag("structure").clone();
                        let invalid = || CommandError::new(tr!("commands.locate.structure.invalid", arg_printable(&arg)));
                        let structures = match &arg {
                            ResourceOrTag::Resource(id) => {
                                if !s.structure_ids().iter().any(|i| i == id.as_str()) {
                                    return Err(invalid());
                                }
                                vec![id.to_string()]
                            }
                            ResourceOrTag::Tag(id) => s.structure_tag(id.as_str()).ok_or_else(invalid)?,
                        };
                        let dim = s.dimension().to_owned();
                        let at = origin(s);
                        let Some(found) = s.locate_structure(&dim, at, &structures) else {
                            return Err(CommandError::new(tr!("commands.locate.structure.not_found", arg_printable(&arg))));
                        };
                        let p = printable(&arg, &found.id);
                        Ok(show_located(s, at, &found, p, "commands.locate.structure.success", false))
                    },
                ),
            ))
            .then(literal("biome").then(
                argument("biome", ArgumentType::ResourceOrTag { registry: "minecraft:worldgen/biome" }).executes(
                    |c, s: &mut S| {
                        let arg = c.resource_or_tag("biome").clone();
                        let dim = s.dimension().to_owned();
                        let at = origin(s);
                        let test = |id: &str| arg.test("minecraft:worldgen/biome", id);
                        let Some(found) = s.locate_biome(&dim, at, &test) else {
                            return Err(CommandError::new(tr!("commands.locate.biome.not_found", arg_printable(&arg))));
                        };
                        let p = printable(&arg, &found.id);
                        Ok(show_located(s, at, &found, p, "commands.locate.biome.success", true))
                    },
                ),
            ))
            .then(literal("poi").then(
                argument("poi", ArgumentType::ResourceOrTag { registry: "minecraft:point_of_interest_type" }).executes(
                    |c, s: &mut S| {
                        let arg = c.resource_or_tag("poi").clone();
                        let dim = s.dimension().to_owned();
                        let at = origin(s);
                        let test = |id: &str| arg.test("minecraft:point_of_interest_type", id);
                        let Some(found) = s.locate_poi(&dim, at, &test) else {
                            return Err(CommandError::new(tr!("commands.locate.poi.not_found", arg_printable(&arg))));
                        };
                        let p = printable(&arg, &found.id);
                        Ok(show_located(s, at, &found, p, "commands.locate.poi.success", false))
                    },
                ),
            )),
    );
}

// ---- place --------------------------------------------------------------------------------

/// `PlaceCommand.checkLoaded`: every chunk from `from` to `to` is loaded.
fn check_loaded<S: Host>(s: &S, dim: &str, from: [i32; 2], to: [i32; 2]) -> Result<()> {
    for x in from[0]..=to[0] {
        for z in from[1]..=to[1] {
            if !s.is_chunk_loaded(dim, x, z) {
                return Err(CommandError::pos_unloaded());
            }
        }
    }
    Ok(())
}

fn place_at<S: Host>(c: &CommandContext<S>, s: &mut S, name: &str) -> Result<[i32; 3]> {
    let dim = s.dimension().to_owned();
    if c.has(name) { loaded_block_pos(c, s, name, &dim) } else { Ok(origin(s)) }
}

fn place_feature<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let (id, inline) = match c.get("feature") {
        Some(ArgumentValue::Identifier(id)) => (Some(id.clone()), None),
        Some(ArgumentValue::Nbt(tag)) => (None, Some(tag.clone())),
        other => unreachable!("feature argument {other:?}"),
    };
    let pos = place_at(c, s, "pos")?;
    let dim = s.dimension().to_owned();
    let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
    check_loaded(s, &dim, [cx - 1, cz - 1], [cx + 1, cz + 1])?;
    s.place(&dim, &Placement::Feature { id: id.clone(), inline }, pos)?;
    let [x, y, z] = pos;
    let text = match id {
        Some(id) => tr!("commands.place.feature.success", id.to_string(), x, y, z),
        None => tr!("commands.place.feature.success.inline", x, y, z),
    };
    s.send_success(text, true);
    Ok(1)
}

fn place_jigsaw<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let pool = c.identifier("pool").clone();
    if !s.registry_ids("minecraft:worldgen/template_pool").iter().any(|i| i == pool.as_str()) {
        return Err(CommandError::new(tr!("commands.place.jigsaw.invalid", pool.to_string())));
    }
    let target = c.identifier("target").clone();
    let max_depth = c.integer("max_depth");
    let pos = place_at(c, s, "position")?;
    let dim = s.dimension().to_owned();
    check_loaded(s, &dim, [pos[0] >> 4, pos[2] >> 4], [pos[0] >> 4, pos[2] >> 4])?;
    s.place(&dim, &Placement::Jigsaw { pool, target, max_depth }, pos)?;
    s.send_success(tr!("commands.place.jigsaw.success", pos[0], pos[1], pos[2]), true);
    Ok(1)
}

fn place_structure<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let id = c.identifier("structure").clone();
    if !s.structure_ids().iter().any(|i| i == id.as_str()) {
        return Err(CommandError::new(tr!("commands.place.structure.invalid", id.to_string())));
    }
    let pos = place_at(c, s, "pos")?;
    let dim = s.dimension().to_owned();
    s.place(&dim, &Placement::Structure(id.clone()), pos)?;
    s.send_success(tr!("commands.place.structure.success", id.to_string(), pos[0], pos[1], pos[2]), true);
    Ok(1)
}

/// `TemplateRotationArgument` names, in `Rotation` order.
const ROTATIONS: [&str; 4] = ["none", "clockwise_90", "180", "counterclockwise_90"];
/// `TemplateMirrorArgument` names, in `Mirror` order.
const MIRRORS: [&str; 3] = ["none", "left_right", "front_back"];

fn place_template<S: Host>(c: &CommandContext<S>, s: &mut S, strict: bool) -> Result<i32> {
    let id = c.identifier("template").clone();
    let pos = place_at(c, s, "pos")?;
    let rotation = match c.get("rotation") {
        Some(ArgumentValue::String(r)) => ROTATIONS.iter().position(|n| n == r).unwrap_or(0) as u8,
        _ => 0,
    };
    let mirror = match c.get("mirror") {
        Some(ArgumentValue::String(m)) => MIRRORS.iter().position(|n| n == m).unwrap_or(0) as u8,
        _ => 0,
    };
    let integrity = if c.has("integrity") { c.float("integrity") } else { 1.0 };
    let seed = if c.has("seed") { c.integer("seed") } else { 0 };
    let dim = s.dimension().to_owned();
    s.place(&dim, &Placement::Template { id: id.clone(), rotation, mirror, integrity, seed, strict }, pos)?;
    s.send_success(tr!("commands.place.template.success", id.to_string(), pos[0], pos[1], pos[2]), true);
    Ok(1)
}

pub fn place<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("place")
            .requires(LEVEL_GAMEMASTERS)
            .then(
                literal("feature").then(
                    argument("feature", ArgumentType::Feature)
                        .executes(place_feature)
                        .then(argument("pos", ArgumentType::BlockPos).executes(place_feature)),
                ),
            )
            .then(
                literal("jigsaw").then(
                    argument("pool", ArgumentType::ResourceKey { registry: "minecraft:worldgen/template_pool" }).then(
                        argument("target", ArgumentType::ResourceLocation).then(
                            argument("max_depth", ArgumentType::integer_range(1, 20))
                                .executes(place_jigsaw)
                                .then(argument("position", ArgumentType::BlockPos).executes(place_jigsaw)),
                        ),
                    ),
                ),
            )
            .then(
                literal("structure").then(
                    argument("structure", ArgumentType::ResourceKey { registry: "minecraft:worldgen/structure" })
                        .executes(place_structure)
                        .then(argument("pos", ArgumentType::BlockPos).executes(place_structure)),
                ),
            )
            .then(
                literal("template").then(
                    argument("template", ArgumentType::ResourceLocation)
                        .executes(|c, s: &mut S| place_template(c, s, false))
                        .then(
                            argument("pos", ArgumentType::BlockPos)
                                .executes(|c, s: &mut S| place_template(c, s, false))
                                .then(
                                    argument("rotation", ArgumentType::TemplateRotation)
                                        .executes(|c, s: &mut S| place_template(c, s, false))
                                        .then(
                                            argument("mirror", ArgumentType::TemplateMirror)
                                                .executes(|c, s: &mut S| place_template(c, s, false))
                                                .then(
                                                    argument("integrity", ArgumentType::float_range(0.0, 1.0))
                                                        .executes(|c, s: &mut S| place_template(c, s, false))
                                                        .then(
                                                            argument("seed", ArgumentType::integer())
                                                                .executes(|c, s: &mut S| place_template(c, s, false))
                                                                .then(
                                                                    literal("strict")
                                                                        .executes(|c, s: &mut S| place_template(c, s, true)),
                                                                ),
                                                        ),
                                                ),
                                        ),
                                ),
                        ),
                ),
            ),
    );
}

// ---- fillbiome ----------------------------------------------------------------------------

/// `QuartPos.toBlock(QuartPos.fromBlock(v))`.
fn quantize(v: i32) -> i32 {
    (v >> 2) << 2
}

fn fill_biome<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let dim = s.dimension().to_owned();
    let from = loaded_block_pos(c, s, "from", &dim)?;
    let to = loaded_block_pos(c, s, "to", &dim)?;
    let biome = c.identifier("biome").clone();
    let filter = c.get("filter").map(|_| c.resource_or_tag("filter").clone());
    let (a, b) = (from.map(quantize), to.map(quantize));
    let min: [i32; 3] = std::array::from_fn(|i| a[i].min(b[i]));
    let max: [i32; 3] = std::array::from_fn(|i| a[i].max(b[i]));
    let volume: i64 = (0..3).map(|i| (max[i] - min[i] + 1) as i64).product();
    let limit = s.game_rule("minecraft:max_block_modifications").command_result();
    if volume > limit as i64 {
        return Err(CommandError::new(tr!("commands.fillbiome.toobig", limit, volume)));
    }
    let test = |id: &str| filter.as_ref().is_none_or(|f| f.test("minecraft:worldgen/biome", id));
    let Some(count) = s.fill_biome(&dim, min, max, biome.as_str(), &test) else {
        return Err(CommandError::pos_unloaded());
    };
    if count == 0 {
        return Err(CommandError::new(tr!("commands.fillbiome.no_changes")));
    }
    s.send_success(tr!("commands.fillbiome.success.count", count, min[0], min[1], min[2], max[0], max[1], max[2]), true);
    Ok(count)
}

pub fn fillbiome<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("fillbiome").requires(LEVEL_GAMEMASTERS).then(
            argument("from", ArgumentType::BlockPos).then(
                argument("to", ArgumentType::BlockPos).then(
                    argument("biome", ArgumentType::resource("minecraft:worldgen/biome")).executes(fill_biome).then(
                        literal("replace").then(
                            argument("filter", ArgumentType::ResourceOrTag { registry: "minecraft:worldgen/biome" })
                                .executes(fill_biome),
                        ),
                    ),
                ),
            ),
        ),
    );
}

// ---- spreadplayers ------------------------------------------------------------------------

#[derive(Clone, Copy, Default, PartialEq)]
struct Position {
    x: f64,
    z: f64,
}

impl Position {
    fn dist(&self, o: &Position) -> f64 {
        let (dx, dz) = (self.x - o.x, self.z - o.z);
        (dx * dx + dz * dz).sqrt()
    }

    fn length(&self) -> f64 {
        (self.x * self.x + self.z * self.z).sqrt()
    }

    fn normalize(&mut self) {
        let l = self.length();
        self.x /= l;
        self.z /= l;
    }

    fn move_away(&mut self, o: &Position) {
        self.x -= o.x;
        self.z -= o.z;
    }

    fn clamp(&mut self, min_x: f64, min_z: f64, max_x: f64, max_z: f64) -> bool {
        let mut changed = false;
        if self.x < min_x {
            self.x = min_x;
            changed = true;
        } else if self.x > max_x {
            self.x = max_x;
            changed = true;
        }
        if self.z < min_z {
            self.z = min_z;
            changed = true;
        } else if self.z > max_z {
            self.z = max_z;
            changed = true;
        }
        changed
    }

    fn randomize(&mut self, rng: &mut SplitMix, min_x: f64, min_z: f64, max_x: f64, max_z: f64) {
        self.x = next_double(rng, min_x, max_x);
        self.z = next_double(rng, min_z, max_z);
    }

    /// `getSpawnY`: the top of the highest non-air block with two air blocks above it, below
    /// `max_height`.
    fn spawn_y<S: Host>(&self, s: &mut S, dim: &str, max_height: i32) -> i32 {
        let (x, z) = (self.x.floor() as i32, self.z.floor() as i32);
        let (min_y, _) = s.build_height(dim);
        let mut y = max_height + 1;
        let mut above2 = kiln_data::blocks_types::is_air(s.block_state(dim, [x, y, z]));
        y -= 1;
        let mut above1 = kiln_data::blocks_types::is_air(s.block_state(dim, [x, y, z]));
        while y > min_y {
            y -= 1;
            let here = kiln_data::blocks_types::is_air(s.block_state(dim, [x, y, z]));
            if !here && above1 && above2 {
                return y + 1;
            }
            above2 = above1;
            above1 = here;
        }
        max_height + 1
    }

    /// `isSafe`: below `max_height`, not a liquid, in `#entities_can_teleport_to`.
    fn is_safe<S: Host>(&self, s: &mut S, dim: &str, max_height: i32) -> bool {
        let y = self.spawn_y(s, dim, max_height) - 1;
        let state = s.block_state(dim, [self.x.floor() as i32, y, self.z.floor() as i32]);
        y < max_height && !kiln_data::block_props::liquid(state) && block_in_tag(state, "minecraft:entities_can_teleport_to")
    }
}

fn block_in_tag(state: u16, tag: &str) -> bool {
    let Some(ids) = crate::blocks::registry_tag("minecraft:block", tag) else { return false };
    let block = kiln_data::blocks_types::block_of(state);
    kiln_data::synced_id("minecraft:block", block.name)
        .or_else(|| kiln_data::builtin_id("minecraft:block", block.name))
        .is_some_and(|id| ids.contains(&id))
}

/// A thread-local style random (vanilla uses `RandomSource.createThreadLocalInstance`, whose
/// draws no test can predict).
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// `Mth.nextDouble(random, min, max)`.
fn next_double(rng: &mut SplitMix, min: f64, max: f64) -> f64 {
    if min >= max { min } else { (rng.next() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) * (max - min) + min }
}

#[allow(clippy::too_many_arguments)]
fn spread_players<S: Host>(
    c: &CommandContext<S>,
    s: &mut S,
    max_height: Option<i32>,
) -> Result<i32> {
    let [cx, _, cz] = s.stack().resolve(c.coordinates("center"));
    let center = (cx as f32, cz as f32);
    let spread = c.float("spreadDistance");
    let range = c.float("maxRange");
    let teams = c.bool("respectTeams");
    let targets = c.selector("targets").entities(s)?;
    let dim = s.dimension().to_owned();
    let (min_y, max_y) = s.build_height(&dim);
    let max_height = max_height.unwrap_or(max_y);
    if max_height < min_y {
        return Err(CommandError::new(tr!("commands.spreadplayers.failed.invalid.height", max_height, min_y)));
    }
    let seed = s.origin()[0].to_bits() ^ (targets.len() as u64).rotate_left(17) ^ s.game_time() as u64;
    let mut rng = SplitMix(seed ^ 0x5DEE_CE66_D1A4_F87B);
    let (min_x, min_z) = ((center.0 - range) as f64, (center.1 - range) as f64);
    let (max_x, max_z) = ((center.0 + range) as f64, (center.1 + range) as f64);
    let count = if teams {
        let mut seen: Vec<Option<String>> = Vec::new();
        for t in &targets {
            let team = if t.is_player() { t.team().map(str::to_owned) } else { None };
            if !seen.contains(&team) {
                seen.push(team);
            }
        }
        seen.len()
    } else {
        targets.len()
    };
    let mut positions: Vec<Position> = (0..count)
        .map(|_| {
            let mut p = Position::default();
            p.randomize(&mut rng, min_x, min_z, max_x, max_z);
            p
        })
        .collect();
    // `spreadPositions`.
    let mut moved = true;
    let mut min_dist = f32::MAX as f64;
    let mut iteration = 0;
    while iteration < 10_000 && moved {
        moved = false;
        min_dist = f32::MAX as f64;
        for i in 0..positions.len() {
            let mut neighbours = 0;
            let mut push = Position::default();
            for j in 0..positions.len() {
                if i == j {
                    continue;
                }
                let d = positions[i].dist(&positions[j]);
                min_dist = min_dist.min(d);
                if d < spread as f64 {
                    neighbours += 1;
                    push.x += positions[j].x - positions[i].x;
                    push.z += positions[j].z - positions[i].z;
                }
            }
            if neighbours > 0 {
                push.x /= neighbours as f64;
                push.z /= neighbours as f64;
                if push.length() > 0.0 {
                    push.normalize();
                    let away = push;
                    positions[i].move_away(&away);
                } else {
                    positions[i].randomize(&mut rng, min_x, min_z, max_x, max_z);
                }
                moved = true;
            }
            if positions[i].clamp(min_x, min_z, max_x, max_z) {
                moved = true;
            }
        }
        if !moved {
            for i in 0..positions.len() {
                let p = positions[i];
                if !p.is_safe(s, &dim, max_height) {
                    positions[i].randomize(&mut rng, min_x, min_z, max_x, max_z);
                    moved = true;
                }
            }
        }
        iteration += 1;
    }
    if min_dist == f32::MAX as f64 {
        min_dist = 0.0;
    }
    if iteration >= 10_000 {
        let key = if teams { "commands.spreadplayers.failed.teams" } else { "commands.spreadplayers.failed.entities" };
        return Err(CommandError::new(tr!(
            key,
            positions.len() as i32,
            java_float(center.0),
            java_float(center.1),
            java_fixed(min_dist, 2)
        )));
    }
    // `setPlayerPositions`.
    let mut total = 0.0;
    let mut next = 0;
    let mut by_team: Vec<(Option<String>, usize)> = Vec::new();
    for t in &targets {
        let index = if teams {
            let team = if t.is_player() { t.team().map(str::to_owned) } else { None };
            match by_team.iter().find(|(k, _)| *k == team) {
                Some(&(_, i)) => i,
                None => {
                    by_team.push((team, next));
                    next += 1;
                    next - 1
                }
            }
        } else {
            next += 1;
            next - 1
        };
        let p = positions[index];
        let y = p.spawn_y(s, &dim, max_height) as f64;
        let to = Teleport {
            dimension: dim.clone(),
            pos: [p.x.floor() + 0.5, y, p.z.floor() + 0.5],
            relative: [false; 3],
            rotation: None,
            relative_rotation: [false; 2],
            facing: None,
        };
        s.teleport(t, &to)?;
        let mut nearest = f64::MAX;
        for (j, q) in positions.iter().enumerate() {
            if j != index {
                nearest = nearest.min(p.dist(q));
            }
        }
        total += nearest;
    }
    let average = if targets.len() < 2 { 0.0 } else { total / targets.len() as f64 };
    let key = if teams { "commands.spreadplayers.success.teams" } else { "commands.spreadplayers.success.entities" };
    s.send_success(
        tr!(key, positions.len() as i32, java_float(center.0), java_float(center.1), java_fixed(average, 2)),
        true,
    );
    Ok(positions.len() as i32)
}

pub fn spreadplayers<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("spreadplayers").requires(LEVEL_GAMEMASTERS).then(
            argument("center", ArgumentType::vec2()).then(
                argument("spreadDistance", ArgumentType::float_range(0.0, f32::MAX)).then(
                    argument("maxRange", ArgumentType::float_range(1.0, f32::MAX))
                        .then(argument("respectTeams", ArgumentType::Bool).then(
                            argument("targets", ArgumentType::entities()).executes(|c, s: &mut S| spread_players(c, s, None)),
                        ))
                        .then(literal("under").then(argument("maxHeight", ArgumentType::integer()).then(
                            argument("respectTeams", ArgumentType::Bool).then(
                                argument("targets", ArgumentType::entities())
                                    .executes(|c, s: &mut S| spread_players(c, s, Some(c.integer("maxHeight")))),
                            ),
                        ))),
                ),
            ),
        ),
    );
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_number_formats() {
        assert_eq!(java_double(59_999_968.0), "5.9999968E7");
        assert_eq!(java_double(2.9999984E7), "2.9999984E7");
        assert_eq!(java_double(1.5), "1.5");
        assert_eq!(java_double(100.0), "100.0");
        assert_eq!(java_float(0.0), "0.0");
        assert_eq!(java_float(12.5), "12.5");
        assert_eq!(java_fixed(0.125, 2), "0.13");
        assert_eq!(java_fixed(1.005, 2), "1.01");
        assert_eq!(java_fixed(59_999_968.0, 0), "59999968");
        assert_eq!(java_fixed(59_999_968.0, 1), "59999968.0");
        assert_eq!(java_fixed(0.2f32 as f64, 2), "0.20");
        assert_eq!(java_fixed(9.995, 2), "10.00");
        assert_eq!(java_fixed(15.0, 2), "15.00");
        assert_eq!(java_fixed(-2.5, 0), "-3");
    }
}
