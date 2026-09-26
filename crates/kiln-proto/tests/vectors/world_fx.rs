//! Sounds, particles, level/game events, the world border and block animations.

use super::{Cases, E, builtin, is};
use bytes::BufMut;
use kiln_data::blocks::default_state as state;
use kiln_proto::WriteExt;
use kiln_proto::packets::game_event;
use kiln_proto::packets::world_fx::{self, *};

fn particle(name: &str) -> i32 {
    builtin("minecraft:particle_type", &format!("minecraft:{name}"))
}

pub fn world_fx(c: &mut Cases) {
    sounds(c);
    particles(c);
    events(c);
    border(c);
    blocks(c);
}

fn sounds(c: &mut Cases) {
    let pickup = builtin("minecraft:sound_event", "minecraft:entity.experience_orb.pickup");
    let p = sound(&Sound::Registered(pickup), SoundSource::Players, [1.5, 64.25, -3.125], 0.5, 1.25, -42);
    let e = vec![
        is("sound", "minecraft:entity.experience_orb.pickup"),
        is("source", "PLAYERS"),
        is("x", 12),
        is("y", 514),
        is("z", -25),
        is("volume", 0.5),
        is("pitch", 1.25),
        is("seed", -42),
    ];
    c.play("sound", "ClientboundSoundPacket", "sound", p, e);
    let direct = Sound::Direct { id: "kiln:lobby.ding", fixed_range: Some(32.0) };
    let p = sound(&direct, SoundSource::Ui, [-0.99, 0.0, 1e6], 1.0, 1.0, 0);
    let e = vec![
        is("sound", "<direct>"),
        is("sound.value.location", "kiln:lobby.ding"),
        is("sound.value.fixedRange", 32.0),
        is("source", "UI"),
        is("x", -7),
        is("z", 8_000_000),
    ];
    c.play("sound_direct", "ClientboundSoundPacket", "sound", p, e);
    let direct = Sound::Direct { id: "minecraft:ui.button.click", fixed_range: None };
    let e = vec![is("sound.value.location", "minecraft:ui.button.click"), is("sound.value.fixedRange", "empty"), is("source", "MASTER")];
    let p = sound(&direct, SoundSource::Master, [0.0; 3], 1.0, 1.0, 1);
    c.play("sound_direct_range", "ClientboundSoundPacket", "sound", p, e);
    let thunder = builtin("minecraft:sound_event", "minecraft:entity.lightning_bolt.thunder");
    let p = sound_entity(&Sound::Registered(thunder), SoundSource::Weather, 77, 10_000.0, 0.8, i64::MAX);
    let e = vec![
        is("sound", "minecraft:entity.lightning_bolt.thunder"),
        is("source", "WEATHER"),
        is("id", 77),
        is("volume", 10000.0),
        is("seed", i64::MAX),
    ];
    c.play("sound_entity", "ClientboundSoundEntityPacket", "sound_entity", p, e);

    let stop = |c: &mut Cases, name: &str, source, sound, e| {
        c.play(name, "ClientboundStopSoundPacket", "stop_sound", stop_sound(source, sound), e)
    };
    stop(c, "stop_sound_all", None, None, vec![is("source", "null"), is("name", "null")]);
    stop(c, "stop_sound_source", Some(SoundSource::Music), None, vec![is("source", "MUSIC"), is("name", "null")]);
    let e = vec![is("source", "null"), is("name", "minecraft:music.menu")];
    stop(c, "stop_sound_name", None, Some("minecraft:music.menu"), e);
    let e = vec![is("source", "RECORDS"), is("name", "minecraft:music_disc.cat")];
    stop(c, "stop_sound_both", Some(SoundSource::Records), Some("minecraft:music_disc.cat"), e);
    let all = [
        SoundSource::Master,
        SoundSource::Music,
        SoundSource::Records,
        SoundSource::Weather,
        SoundSource::Blocks,
        SoundSource::Hostile,
        SoundSource::Neutral,
        SoundSource::Players,
        SoundSource::Ambient,
        SoundSource::Voice,
        SoundSource::Ui,
    ];
    let names = ["MASTER", "MUSIC", "RECORDS", "WEATHER", "BLOCKS", "HOSTILE", "NEUTRAL", "PLAYERS", "AMBIENT", "VOICE", "UI"];
    for (src, name) in all.into_iter().zip(names) {
        let p = stop_sound(Some(src), None);
        c.play(&format!("stop_sound_{}", name.to_lowercase()), "ClientboundStopSoundPacket", "stop_sound", p, vec![is("source", name)]);
    }
}

fn particles(c: &mut Cases) {
    let base = LevelParticles {
        particle: Particle { kind: particle("flame"), options: ParticleOptions::None },
        override_limiter: false,
        always_show: true,
        pos: [0.5, 70.0, -10.25],
        offset: [0.25, 0.5, 1.0],
        max_speed: [0.01, 0.02, 0.03],
        count: 40,
        randomization: ParticleRandomization::Default,
    };
    let e = vec![
        is("particle", "minecraft:flame"),
        is("overrideLimiter", false),
        is("alwaysShow", true),
        is("x", 0.5),
        is("y", 70.0),
        is("z", -10.25),
        is("xDist", 0.25),
        is("yDist", 0.5),
        is("zDist", 1.0),
        is("xMaxSpeed", 0.01),
        is("yMaxSpeed", 0.02),
        is("zMaxSpeed", 0.03),
        is("count", 40),
        is("randomizationType", "DEFAULT"),
    ];
    c.play("level_particles_flame", "ClientboundLevelParticlesPacket", "level_particles", level_particles(&base), e);

    let mut add = |name: &str, kind: &str, options: ParticleOptions, mut e: Vec<E>| {
        let p = LevelParticles { particle: Particle { kind: particle(kind), options }, ..base };
        e.insert(0, is("particle", format!("minecraft:{kind}")));
        c.play(&format!("level_particles_{name}"), "ClientboundLevelParticlesPacket", "level_particles", level_particles(&p), e);
    };
    add("dust", "dust", ParticleOptions::Dust { color: 0xff8000, scale: 1.5 }, vec![
        is("particle.color", 0xff8000),
        is("particle.scale", 1.5),
    ]);
    add("dust_transition", "dust_color_transition", ParticleOptions::DustColorTransition { from: 0x00ff00, to: 0x0000ff, scale: 0.5 }, vec![
        is("particle.fromColor", 0x00ff00),
        is("particle.toColor", 0x0000ff),
        is("particle.scale", 0.5),
    ]);
    add("block", "block", ParticleOptions::Block(state::STONE as i32), vec![is("particle.state", "Block{minecraft:stone}")]);
    add("block_marker", "block_marker", ParticleOptions::Block(state::OAK_LOG as i32), vec![
        is("particle.state", "Block{minecraft:oak_log}[axis=y]"),
    ]);
    add("falling_dust", "falling_dust", ParticleOptions::Block(state::DIRT as i32), vec![is("particle.state", "Block{minecraft:dirt}")]);
    add("dust_pillar", "dust_pillar", ParticleOptions::Block(state::STONE as i32), vec![]);
    add("block_crumble", "block_crumble", ParticleOptions::Block(state::STONE as i32), vec![]);
    add("entity_effect", "entity_effect", ParticleOptions::Color(0x80ff0000u32 as i32), vec![is("particle.color", 0x80ff0000u32 as i32)]);
    add("tinted_leaves", "tinted_leaves", ParticleOptions::Color(-1), vec![is("particle.color", -1)]);
    add("flash", "flash", ParticleOptions::Color(0x7fffffff), vec![is("particle.color", 0x7fffffff)]);
    add("effect", "effect", ParticleOptions::Spell { color: 0x3355ff, power: 2.0 }, vec![
        is("particle.color", 0x3355ff),
        is("particle.power", 2.0),
    ]);
    add("instant_effect", "instant_effect", ParticleOptions::Spell { color: 0, power: 0.5 }, vec![is("particle.power", 0.5)]);
    add("dragon_breath", "dragon_breath", ParticleOptions::Power(0.75), vec![is("particle.power", 0.75)]);
    let block_dest = PositionSource::Block([10, -20, 30]);
    add("vibration_block", "vibration", ParticleOptions::Vibration { destination: block_dest, arrival_ticks: 20 }, vec![
        is("particle.destination.@class", "BlockPositionSource"),
        is("particle.destination.pos.x", 10),
        is("particle.destination.pos.y", -20),
        is("particle.destination.pos.z", 30),
        is("particle.arrivalInTicks", 20),
    ]);
    let entity_dest = PositionSource::Entity { id: 1234, y_offset: 1.62 };
    add("vibration_entity", "vibration", ParticleOptions::Vibration { destination: entity_dest, arrival_ticks: 7 }, vec![
        is("particle.destination.@class", "EntityPositionSource"),
        is("particle.destination.entityOrUuidOrId.value.value", 1234),
        is("particle.destination.yOffset", 1.62),
        is("particle.arrivalInTicks", 7),
    ]);
    add("sculk_charge", "sculk_charge", ParticleOptions::SculkCharge { roll: 2.5 }, vec![is("particle.roll", 2.5)]);
    add("shriek", "shriek", ParticleOptions::Shriek { delay: 15 }, vec![is("particle.delay", 15)]);
    add("trail", "trail", ParticleOptions::Trail { target: [1.5, -2.25, 300.0], color: 0x00ffaa, duration: 40 }, vec![
        is("particle.target.x", 1.5),
        is("particle.target.y", -2.25),
        is("particle.target.z", 300.0),
        is("particle.color", 0x00ffaa),
        is("particle.duration", 40),
    ]);
    add("geyser", "geyser", ParticleOptions::Geyser { water_blocks: 5 }, vec![is("particle.waterBlocks", 5)]);
    add("geyser_plume", "geyser_plume", ParticleOptions::Geyser { water_blocks: 0 }, vec![is("particle.waterBlocks", 0)]);
    add("geyser_base", "geyser_base", ParticleOptions::GeyserBase { water_blocks: 3, burst_impulse_base: 0.4 }, vec![
        is("particle.waterBlocks", 3),
        is("particle.burstImpulseBase", 0.4),
    ]);
    add("geyser_poof", "geyser_poof", ParticleOptions::GeyserBase { water_blocks: 1, burst_impulse_base: 0.0 }, vec![]);
    // Item particles take an item stack template: item id, count, empty component patch.
    let mut raw = bytes::BytesMut::new();
    raw.put_varint(builtin("minecraft:item", "minecraft:diamond"));
    raw.put_slice(&[1, 0, 0]);
    add("item_raw", "item", ParticleOptions::Raw(&raw), vec![is("particle.itemStack.count", 1)]);

    for (name, r, want) in [
        ("alternative", ParticleRandomization::Alternative, "ALTERNATIVE"),
        ("alternative_speed", ParticleRandomization::AlternativeWithSpeed, "ALTERNATIVE_WITH_SPEED"),
    ] {
        let p = LevelParticles { randomization: r, override_limiter: true, always_show: false, count: 0, ..base };
        let e = vec![is("randomizationType", want), is("overrideLimiter", true), is("alwaysShow", false), is("count", 0)];
        c.play(&format!("level_particles_{name}"), "ClientboundLevelParticlesPacket", "level_particles", level_particles(&p), e);
    }
}

fn events(c: &mut Cases) {
    let p = level_event(2001, [-100, 12, 3000], state::STONE as i32, false);
    let e = vec![is("type", 2001), is("pos.x", -100), is("pos.y", 12), is("pos.z", 3000), is("data", 1), is("globalEvent", false)];
    c.play("level_event", "ClientboundLevelEventPacket", "level_event", p, e);
    let p = level_event(1023, [0, 64, 0], 0, true);
    c.play("level_event_global", "ClientboundLevelEventPacket", "level_event", p, vec![is("type", 1023), is("globalEvent", true)]);

    for (name, id, value) in [
        ("change_game_mode", world_fx::game_event::CHANGE_GAME_MODE, 1.0),
        ("rain_level", world_fx::game_event::RAIN_LEVEL_CHANGE, 0.5),
        ("immediate_respawn", world_fx::game_event::IMMEDIATE_RESPAWN, 1.0),
        ("chunks_load_start", world_fx::game_event::LEVEL_CHUNKS_LOAD_START, 0.0),
        ("limited_crafting", world_fx::game_event::LIMITED_CRAFTING, 0.0),
    ] {
        let e = vec![is("event.id", id), is("param", value)];
        c.play(&format!("game_event_{name}"), "ClientboundGameEventPacket", "game_event", game_event(id, value), e);
    }
}

fn border(c: &mut Cases) {
    let w = WorldBorder {
        center: [100.5, -2000.25],
        size: 500.0,
        target_size: 100.0,
        lerp_ticks: 1200,
        absolute_max_size: 29_999_984,
        warning_blocks: 5,
        warning_time: 15,
    };
    let e = vec![
        is("newCenterX", 100.5),
        is("newCenterZ", -2000.25),
        is("oldSize", 500.0),
        is("newSize", 100.0),
        is("lerpTime", 1200),
        is("newAbsoluteMaxSize", 29_999_984),
        is("warningBlocks", 5),
        is("warningTime", 15),
    ];
    c.play("initialize_border", "ClientboundInitializeBorderPacket", "initialize_border", initialize_border(&w), e);
    let still = WorldBorder { target_size: 500.0, lerp_ticks: 0, ..w };
    let e = vec![is("oldSize", 500.0), is("newSize", 500.0), is("lerpTime", 0)];
    c.play("initialize_border_still", "ClientboundInitializeBorderPacket", "initialize_border", initialize_border(&still), e);
    let e = vec![is("newCenterX", -8.0), is("newCenterZ", 8.0)];
    c.play("set_border_center", "ClientboundSetBorderCenterPacket", "set_border_center", set_border_center(-8.0, 8.0), e);
    let e = vec![is("oldSize", 60_000_000.0), is("newSize", 16.0), is("lerpTime", 1i64 << 40)];
    let p = set_border_lerp_size(6.0e7, 16.0, 1 << 40);
    c.play("set_border_lerp_size", "ClientboundSetBorderLerpSizePacket", "set_border_lerp_size", p, e);
    c.play("set_border_size", "ClientboundSetBorderSizePacket", "set_border_size", set_border_size(1234.5), vec![is("size", 1234.5)]);
    let p = set_border_warning_delay(30);
    c.play("set_border_warning_delay", "ClientboundSetBorderWarningDelayPacket", "set_border_warning_delay", p, vec![is("warningDelay", 30)]);
    let p = set_border_warning_distance(12);
    let e = vec![is("warningBlocks", 12)];
    c.play("set_border_warning_distance", "ClientboundSetBorderWarningDistancePacket", "set_border_warning_distance", p, e);
}

fn blocks(c: &mut Cases) {
    let e = vec![is("id", 55), is("pos.x", 1), is("pos.y", -64), is("pos.z", -1), is("progress", 5)];
    let p = block_destruction(55, [1, -64, -1], Some(5));
    c.play("block_destruction", "ClientboundBlockDestructionPacket", "block_destruction", p, e);
    let p = block_destruction(55, [1, -64, -1], None);
    c.play("block_destruction_clear", "ClientboundBlockDestructionPacket", "block_destruction", p, vec![is("progress", 255)]);

    let note_block = builtin("minecraft:block", "minecraft:note_block");
    let p = block_event([5, 70, 5], 0, 12, note_block);
    let e = vec![is("pos.x", 5), is("pos.y", 70), is("b0", 0), is("b1", 12), is("block", "minecraft:note_block")];
    c.play("block_event", "ClientboundBlockEventPacket", "block_event", p, e);
    let chest = builtin("minecraft:block", "minecraft:chest");
    let p = block_event([-5, 0, 7], 1, 255, chest);
    let e = vec![is("b0", 1), is("b1", 255), is("block", "minecraft:chest")];
    c.play("block_event_chest", "ClientboundBlockEventPacket", "block_event", p, e);

    let changes = [
        ([0, 0, 0], state::STONE as u32),
        ([15, 15, 15], state::DIRT as u32),
        ([3, 7, 11], state::OAK_LOG as u32),
        ([8, 1, 2], state::AIR as u32),
    ];
    let p = section_blocks_update([-2, -4, 1_000_000], &changes);
    let e = vec![
        is("sectionPos.x", -2),
        is("sectionPos.y", -4),
        is("sectionPos.z", 1_000_000),
        is("positions[0]", 0),
        is("positions[1]", 0xfff),
        is("positions[2]", 3 << 8 | 11 << 4 | 7),
        is("positions[3]", 8 << 8 | 2 << 4 | 1),
        is("states[0]", "Block{minecraft:stone}"),
        is("states[1]", "Block{minecraft:dirt}"),
        is("states[2]", "Block{minecraft:oak_log}[axis=y]"),
        is("states[3]", "Block{minecraft:air}"),
    ];
    c.play("section_blocks_update", "ClientboundSectionBlocksUpdatePacket", "section_blocks_update", p, e);
    let p = section_blocks_update([0, 19, -1], &[]);
    let e = vec![is("sectionPos.y", 19), is("sectionPos.z", -1), is("positions", "[]")];
    c.play("section_blocks_update_empty", "ClientboundSectionBlocksUpdatePacket", "section_blocks_update", p, e);

    let p = open_sign_editor([3, 4, 5], true);
    let e = vec![is("pos.x", 3), is("pos.y", 4), is("pos.z", 5), is("slot", "FRONT")];
    c.play("open_sign_editor_front", "ClientboundOpenSignEditorPacket", "open_sign_editor", p, e);
    let p = open_sign_editor([3, 4, 5], false);
    c.play("open_sign_editor_back", "ClientboundOpenSignEditorPacket", "open_sign_editor", p, vec![is("slot", "BACK")]);
}
