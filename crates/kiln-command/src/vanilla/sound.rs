//! `particle`, `playsound` and `stopsound` (vanilla `ParticleCommand`, `PlaySoundCommand`,
//! `StopSoundCommand`): packets to the players in range, feedback by how many got them.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::{ArgumentType, ParticleArg};
use crate::dispatcher::{Builder, CommandContext, Dispatcher, SuggestionProvider, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::reader::StringReader;
use crate::selector::SelectorTarget;
use crate::snbt::to_snbt;
use crate::tr;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::world_fx::{self, LevelParticles, Particle, ParticleOptions, ParticleRandomization, Sound, SoundSource};

type Result<T> = std::result::Result<T, CommandError>;

/// `CommandResponseTracker` over players: feedback for one, for many, or the error for none.
struct Tracker<E> {
    count: i32,
    only: Option<E>,
}

impl<E: Clone> Tracker<E> {
    fn new() -> Self {
        Self { count: 0, only: None }
    }

    fn track(&mut self, e: &E, nonzero: bool) {
        if !nonzero {
            return;
        }
        self.count += 1;
        self.only = if self.count == 1 { Some(e.clone()) } else { None };
    }
}

/// `SoundSource` by its `getName`, in `values()` order.
const SOURCES: [(&str, SoundSource); 11] = [
    ("master", SoundSource::Master),
    ("music", SoundSource::Music),
    ("record", SoundSource::Records),
    ("weather", SoundSource::Weather),
    ("block", SoundSource::Blocks),
    ("hostile", SoundSource::Hostile),
    ("neutral", SoundSource::Neutral),
    ("player", SoundSource::Players),
    ("ambient", SoundSource::Ambient),
    ("voice", SoundSource::Voice),
    ("ui", SoundSource::Ui),
];

fn source_name(source: SoundSource) -> &'static str {
    SOURCES.iter().find(|(_, s)| *s == source).map_or("master", |(n, _)| n)
}

/// `BlockPos.closerToCenterThan`: the block holding `player` against `pos`.
fn closer_to_center<E: SelectorTarget>(player: &E, pos: [f64; 3], distance: f64) -> bool {
    let p = player.position();
    let d: f64 = (0..3).map(|i| (p[i].floor() + 0.5 - pos[i]).powi(2)).sum();
    d < distance * distance
}

// ---- particle ----------------------------------------------------------------------------

/// The keys a particle type's options need, in the order DFU reports missing ones.
fn required_keys(kind: &str) -> &'static [&'static str] {
    match kind {
        "dust" => &["scale", "color"],
        "dust_color_transition" => &["scale", "to_color", "from_color"],
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" => &["block_state"],
        "entity_effect" | "tinted_leaves" | "flash" => &["color"],
        "effect" | "instant_effect" => &["power", "color"],
        "dragon_breath" => &["power"],
        "sculk_charge" => &["roll"],
        "shriek" => &["delay"],
        "item" => &["item"],
        "vibration" => &["arrival_in_ticks", "destination"],
        "trail" => &["duration", "color", "target"],
        "geyser" | "geyser_plume" => &["water_blocks"],
        "geyser_base" | "geyser_poof" => &["burst_impulse_base", "water_blocks"],
        _ => &[],
    }
}

fn get<'a>(options: &'a Tag, key: &str) -> Option<&'a Tag> {
    match options {
        Tag::Compound(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn float(t: &Tag) -> Option<f32> {
    match *t {
        Tag::Float(v) => Some(v),
        Tag::Double(v) => Some(v as f32),
        Tag::Byte(v) => Some(v.into()),
        Tag::Short(v) => Some(v.into()),
        Tag::Int(v) => Some(v as f32),
        Tag::Long(v) => Some(v as f32),
        _ => None,
    }
}

/// `ExtraCodecs.RGB_COLOR_CODEC` / `ARGB_COLOR_CODEC`: an int, or float components 0..1.
fn color(t: &Tag, alpha: bool) -> Option<i32> {
    let c = |v: f32| ((v.clamp(0.0, 1.0) * 255.0) as i32) & 0xFF;
    match t {
        Tag::Int(v) => Some(*v),
        Tag::List(items) => {
            let f: Vec<f32> = items.iter().map(float).collect::<Option<_>>()?;
            match (f.len(), alpha) {
                (3, false) => Some(c(f[0]) << 16 | c(f[1]) << 8 | c(f[2])),
                (4, true) => Some(c(f[3]) << 24 | c(f[0]) << 16 | c(f[1]) << 8 | c(f[2])),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A block state as `BlockState.CODEC` reads it: `"id[props]"` or `{Name, Properties}`.
fn block_state(t: &Tag) -> Option<i32> {
    let text = match t {
        Tag::String(s) => s.clone(),
        Tag::Compound(_) => {
            let name = get(t, "Name")?.as_str()?.to_owned();
            match get(t, "Properties") {
                Some(Tag::Compound(p)) if !p.is_empty() => {
                    let props: Vec<String> = p.iter().filter_map(|(k, v)| Some(format!("{k}={}", v.as_str()?))).collect();
                    format!("{name}[{}]", props.join(","))
                }
                _ => name,
            }
        }
        _ => return None,
    };
    let mut reader = StringReader::new(&text);
    crate::blocks::parse_block_state(&mut reader).ok().filter(|_| !reader.can_read()).map(|b| i32::from(b.state))
}

/// `ParticleArgument.readParticle` + the options codec: the particle, or the message of
/// `particle.invalidOptions`. `Ok(None)`: valid options Kiln cannot encode.
pub(crate) fn decode_particle(arg: &ParticleArg) -> std::result::Result<Option<(i32, ParticleOptions<'static>)>, String> {
    let kind = arg.id.path();
    let registry_id = kiln_data::builtin_id("minecraft:particle_type", arg.id.as_str()).expect("checked");
    let o = &arg.options;
    let missing: Vec<String> = required_keys(kind)
        .iter()
        .filter(|k| get(o, k).is_none())
        .map(|k| format!("No key {k} in MapLike[{}]", to_snbt(o)))
        .collect();
    if !missing.is_empty() {
        return Err(missing.join("; "));
    }
    let bad = || format!("Invalid options {}", to_snbt(o));
    let f = |k: &str| get(o, k).and_then(float).ok_or_else(bad);
    let i = |k: &str| match get(o, k) {
        Some(Tag::Int(v)) => Ok(*v),
        _ => Err(bad()),
    };
    let options = match kind {
        "dust" => ParticleOptions::Dust { color: get(o, "color").and_then(|t| color(t, false)).ok_or_else(bad)?, scale: f("scale")? },
        "dust_color_transition" => ParticleOptions::DustColorTransition {
            from: get(o, "from_color").and_then(|t| color(t, false)).ok_or_else(bad)?,
            to: get(o, "to_color").and_then(|t| color(t, false)).ok_or_else(bad)?,
            scale: f("scale")?,
        },
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" => {
            ParticleOptions::Block(get(o, "block_state").and_then(block_state).ok_or_else(bad)?)
        }
        "entity_effect" | "tinted_leaves" | "flash" => {
            ParticleOptions::Color(get(o, "color").and_then(|t| color(t, true)).ok_or_else(bad)?)
        }
        "effect" | "instant_effect" => {
            ParticleOptions::Spell { color: get(o, "color").and_then(|t| color(t, false)).ok_or_else(bad)?, power: f("power")? }
        }
        "dragon_breath" => ParticleOptions::Power(f("power")?),
        "sculk_charge" => ParticleOptions::SculkCharge { roll: f("roll")? },
        "shriek" => ParticleOptions::Shriek { delay: i("delay")? },
        "geyser" | "geyser_plume" => ParticleOptions::Geyser { water_blocks: i("water_blocks")? },
        "geyser_base" | "geyser_poof" => {
            ParticleOptions::GeyserBase { water_blocks: i("water_blocks")?, burst_impulse_base: f("burst_impulse_base")? }
        }
        // Item stacks, vibrations and trails: accepted, not sent.
        "item" | "vibration" | "trail" => return Ok(None),
        _ => ParticleOptions::None,
    };
    Ok(Some((registry_id, options)))
}

fn particle<S: Host>(c: &CommandContext<S>, s: &mut S, depth: u8, force: bool) -> Result<i32> {
    let arg = c.particle("name");
    let decoded = decode_particle(arg).map_err(|m| CommandError::new(tr!("particle.invalidOptions", m)))?;
    let pos = if depth >= 1 { s.stack().resolve(c.coordinates("pos")) } else { s.stack().position };
    let (delta, speed, count) = if depth >= 2 {
        // `Vec3Argument.getVec3`: `~` offsets from the source's position, as vanilla does.
        let d = s.stack().resolve(c.coordinates("delta"));
        ([d[0] as f32, d[1] as f32, d[2] as f32], c.float("speed"), c.integer("count"))
    } else {
        ([0.0; 3], 0.0, 0)
    };
    let viewers = if c.get("viewers").is_some() { c.selector("viewers").players(s)? } else { s.players() };
    let dimension = s.dimension().to_owned();
    let packet = decoded.map(|(kind, options)| {
        world_fx::level_particles(&LevelParticles {
            particle: Particle { kind, options },
            override_limiter: force,
            always_show: false,
            pos,
            offset: delta,
            max_speed: [speed; 3],
            count,
            randomization: ParticleRandomization::Default,
        })
    });
    let mut shown = 0;
    for p in &viewers {
        // `ServerLevel.sendParticles(player, ...)`.
        if p.dimension() != dimension || !closer_to_center(p, pos, if force { 512.0 } else { 32.0 }) {
            continue;
        }
        if let Some(pkt) = &packet {
            s.send_packet(p, pkt.clone());
        }
        shown += 1;
    }
    if shown == 0 {
        return Err(CommandError::new(tr!("commands.particle.failed")));
    }
    s.send_success(tr!("commands.particle.success", arg.id.to_string()), true);
    Ok(shown)
}

pub fn particle_command<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let viewers = |mode: &str, force: bool| {
        literal(mode)
            .executes(move |c, s: &mut S| particle(c, s, 3, force))
            .then(argument("viewers", ArgumentType::players()).executes(move |c, s: &mut S| particle(c, s, 3, force)))
    };
    d.register(
        literal("particle").requires(LEVEL_GAMEMASTERS).then(
            argument("name", ArgumentType::Particle).executes(|c, s: &mut S| particle(c, s, 0, false)).then(
                argument("pos", ArgumentType::vec3()).executes(|c, s: &mut S| particle(c, s, 1, false)).then(
                    argument("delta", ArgumentType::Vec3 { center_correct: false }).then(
                        argument("speed", ArgumentType::float_range(0.0, f32::MAX)).then(
                            argument("count", ArgumentType::integer_min(0))
                                .executes(|c, s: &mut S| particle(c, s, 2, false))
                                .then(viewers("force", true))
                                .then(viewers("normal", false)),
                        ),
                    ),
                ),
            ),
        ),
    );
}

// ---- playsound / stopsound ----------------------------------------------------------------

/// `depth` 0: the calling player (none for the console); 1: `targets`; 2..: with the position,
/// volume, pitch and minimum volume.
fn play<S: Host>(c: &CommandContext<S>, s: &mut S, source: SoundSource, depth: u8) -> Result<i32> {
    let targets = if depth == 0 {
        s.source_entity().filter(SelectorTarget::is_player).into_iter().collect()
    } else {
        c.selector("targets").players(s)?
    };
    let depth = depth.saturating_sub(1);
    let sound = c.identifier("sound").clone();
    let pos = if depth >= 1 { s.stack().resolve(c.coordinates("pos")) } else { s.stack().position };
    let volume = if depth >= 2 { c.float("volume") } else { 1.0 };
    let pitch = if depth >= 3 { c.float("pitch") } else { 1.0 };
    let min_volume = if depth >= 4 { c.float("minVolume") } else { 0.0 };
    // `SoundEvent.getRange` of a variable-range event.
    let range = if volume > 1.0 { 16.0 * volume } else { 16.0 };
    let max = f64::from(range * range);
    let seed = s.random_seed();
    let dimension = s.dimension().to_owned();
    let mut tracker = Tracker::new();
    for p in &targets {
        if p.dimension() != dimension {
            continue;
        }
        let at = p.position();
        let (dx, dy, dz) = (pos[0] - at[0], pos[1] - at[1], pos[2] - at[2]);
        let dist = dx * dx + dy * dy + dz * dz;
        let (mut heard_at, mut heard_volume) = (pos, volume);
        if dist > max {
            if min_volume <= 0.0 {
                continue;
            }
            let d = dist.sqrt();
            heard_at = [at[0] + dx / d * 2.0, at[1] + dy / d * 2.0, at[2] + dz / d * 2.0];
            heard_volume = min_volume;
        }
        let snd = Sound::Direct { id: sound.as_str(), fixed_range: None };
        s.send_packet(p, world_fx::sound(&snd, source, heard_at, heard_volume, pitch, seed));
        tracker.track(p, true);
    }
    match (tracker.count, tracker.only) {
        (0, _) => Err(CommandError::new(tr!("commands.playsound.failed"))),
        (_, Some(one)) => {
            s.send_success(tr!("commands.playsound.success.single", sound.to_string(), one.display_name()), true);
            Ok(1)
        }
        (n, None) => {
            s.send_success(tr!("commands.playsound.success.multiple", sound.to_string(), n), true);
            Ok(n)
        }
    }
}

pub fn playsound<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut sound = argument("sound", ArgumentType::ResourceLocation)
        .suggests(SuggestionProvider::Named("minecraft:available_sounds"))
        .executes(|c, s: &mut S| play(c, s, SoundSource::Master, 0));
    for (name, source) in SOURCES {
        let node: Builder<S> = literal(name).executes(move |c, s: &mut S| play(c, s, source, 0)).then(
            argument("targets", ArgumentType::players()).executes(move |c, s: &mut S| play(c, s, source, 1)).then(
                argument("pos", ArgumentType::vec3()).executes(move |c, s: &mut S| play(c, s, source, 2)).then(
                    argument("volume", ArgumentType::float_range(0.0, f32::MAX))
                        .executes(move |c, s: &mut S| play(c, s, source, 3))
                        .then(
                            argument("pitch", ArgumentType::float_range(0.0, 2.0))
                                .executes(move |c, s: &mut S| play(c, s, source, 4))
                                .then(
                                    argument("minVolume", ArgumentType::float_range(0.0, 1.0))
                                        .executes(move |c, s: &mut S| play(c, s, source, 5)),
                                ),
                        ),
                ),
            ),
        );
        sound = sound.then(node);
    }
    d.register(literal("playsound").requires(LEVEL_GAMEMASTERS).then(sound));
}

fn stop<S: Host>(c: &CommandContext<S>, s: &mut S, source: Option<SoundSource>, with_sound: bool) -> Result<i32> {
    let targets = c.selector("targets").players(s)?;
    let sound = with_sound.then(|| c.identifier("sound").clone());
    let pkt = world_fx::stop_sound(source, sound.as_ref().map(|id| id.as_str()));
    for p in &targets {
        s.send_packet(p, pkt.clone());
    }
    let text = match (source, &sound) {
        (Some(src), Some(id)) => tr!("commands.stopsound.success.source.sound", id.to_string(), source_name(src)),
        (Some(src), None) => tr!("commands.stopsound.success.source.any", source_name(src)),
        (None, Some(id)) => tr!("commands.stopsound.success.sourceless.sound", id.to_string()),
        (None, None) => tr!("commands.stopsound.success.sourceless.any"),
    };
    s.send_success(text, true);
    Ok(targets.len() as i32)
}

pub fn stopsound<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let sound_arg = || argument("sound", ArgumentType::ResourceLocation).suggests(SuggestionProvider::Named("minecraft:available_sounds"));
    let mut targets = argument("targets", ArgumentType::players())
        .executes(|c, s: &mut S| stop(c, s, None, false))
        .then(literal("*").then(sound_arg().executes(|c, s: &mut S| stop(c, s, None, true))));
    for (name, source) in SOURCES {
        targets = targets.then(
            literal(name)
                .executes(move |c, s: &mut S| stop(c, s, Some(source), false))
                .then(sound_arg().executes(move |c, s: &mut S| stop(c, s, Some(source), true))),
        );
    }
    d.register(literal("stopsound").requires(LEVEL_GAMEMASTERS).then(targets));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Identifier;

    fn arg(id: &str, snbt: &str) -> ParticleArg {
        let options = if snbt.is_empty() {
            Tag::Compound(Vec::new())
        } else {
            crate::snbt::parse_tag(&mut StringReader::new(snbt)).unwrap()
        };
        ParticleArg { id: Identifier::parse(id).unwrap(), options }
    }

    #[test]
    fn particle_options_as_vanilla_decodes_them() {
        assert_eq!(
            decode_particle(&arg("minecraft:dust", "")).unwrap_err(),
            "No key scale in MapLike[{}]; No key color in MapLike[{}]"
        );
        let (_, o) = decode_particle(&arg("minecraft:dust", "{color:[1.0,0.0,0.0],scale:1.0}")).unwrap().unwrap();
        assert_eq!(o, ParticleOptions::Dust { color: 0xFF0000, scale: 1.0 });
        let (_, o) = decode_particle(&arg("minecraft:block", "{block_state:\"minecraft:stone\"}")).unwrap().unwrap();
        assert!(matches!(o, ParticleOptions::Block(id) if id > 0));
        assert!(matches!(decode_particle(&arg("minecraft:flame", "{x:1}")), Ok(Some((_, ParticleOptions::None)))));
    }
}
