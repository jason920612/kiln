//! Player state and entity status packets.

use super::{Cases, builtin, is, near, synced};
use kiln_proto::packets::entity::*;
use kiln_proto::packets::player::*;

pub fn player(c: &mut Cases) {
    let a = Abilities {
        invulnerable: true,
        flying: false,
        may_fly: true,
        instabuild: true,
        flying_speed: 0.05,
        walking_speed: 0.1,
    };
    let e = vec![
        is("invulnerable", true),
        is("isFlying", false),
        is("canFly", true),
        is("instabuild", true),
        is("flyingSpeed", 0.05),
        is("walkingSpeed", 0.1),
    ];
    c.play("player_abilities", "ClientboundPlayerAbilitiesPacket", "player_abilities", player_abilities(&a), e);
    let a = Abilities { invulnerable: false, flying: true, may_fly: false, instabuild: false, ..a };
    let e = vec![is("invulnerable", false), is("isFlying", true), is("canFly", false), is("instabuild", false)];
    c.play("player_abilities_flying", "ClientboundPlayerAbilitiesPacket", "player_abilities", player_abilities(&a), e);

    let e = vec![is("health", 13.5), is("food", 17), is("saturation", 2.5)];
    c.play("set_health", "ClientboundSetHealthPacket", "set_health", set_health(13.5, 17, 2.5), e);
    // Keys in the order vanilla's compound (a hash map) writes them.
    let msg = kiln_proto::nbt::Tag::Compound(vec![
        ("with".into(), kiln_proto::nbt::Tag::List(vec![kiln_proto::nbt::Tag::String("Steve".into())])),
        ("translate".into(), kiln_proto::nbt::Tag::String("death.attack.genericKill".into())),
    ]);
    let e = vec![is("playerId", 42)];
    c.play("player_combat_kill", "ClientboundPlayerCombatKillPacket", "player_combat_kill", player_combat_kill(42, &msg), e);
    let e = vec![is("experienceProgress", 0.25), is("experienceLevel", 30), is("totalExperience", 1395)];
    c.play("set_experience", "ClientboundSetExperiencePacket", "set_experience", set_experience(0.25, 30, 1395), e);

    let spawn = SpawnInfo {
        dimension_type: synced("minecraft:dimension_type", "minecraft:the_nether"),
        dimension: "minecraft:the_nether",
        hashed_seed: -1234567890123,
        game_mode: 2,
        previous_game_mode: Some(0),
        is_debug: false,
        is_flat: true,
        death_location: Some(("minecraft:overworld", [-100, -60, 250])),
        portal_cooldown: 300,
        sea_level: 32,
    };
    let si = |f: &str| format!("commonPlayerSpawnInfo.{f}");
    let e = vec![
        is(&si("dimensionType"), "minecraft:the_nether"),
        is(&si("dimension"), "minecraft:the_nether"),
        is(&si("seed"), -1234567890123i64),
        is(&si("gameType"), "ADVENTURE"),
        is(&si("previousGameType"), "SURVIVAL"),
        is(&si("isDebug"), false),
        is(&si("isFlat"), true),
        is(&si("lastDeathLocation.dimension"), "minecraft:overworld"),
        is(&si("lastDeathLocation.pos.x"), -100),
        is(&si("lastDeathLocation.pos.y"), -60),
        is(&si("lastDeathLocation.pos.z"), 250),
        is(&si("portalCooldown"), 300),
        is(&si("seaLevel"), 32),
        is("dataToKeep", 3),
    ];
    c.play("respawn", "ClientboundRespawnPacket", "respawn", respawn(&spawn, respawn_keep::ALL), e);
    let spawn = SpawnInfo {
        dimension_type: synced("minecraft:dimension_type", "minecraft:overworld"),
        dimension: "minecraft:overworld",
        game_mode: 3,
        previous_game_mode: None,
        is_debug: true,
        is_flat: false,
        death_location: None,
        ..spawn
    };
    let e = vec![
        is(&si("dimensionType"), "minecraft:overworld"),
        is(&si("gameType"), "SPECTATOR"),
        is(&si("previousGameType"), "empty"),
        is(&si("isDebug"), true),
        is(&si("lastDeathLocation"), "empty"),
        is("dataToKeep", 1),
    ];
    let p = respawn(&spawn, respawn_keep::ATTRIBUTE_MODIFIERS);
    c.play("respawn_overworld", "ClientboundRespawnPacket", "respawn", p, e);
    let spawn = SpawnInfo { game_mode: 1, previous_game_mode: Some(3), ..spawn };
    let e = vec![is(&si("gameType"), "CREATIVE"), is(&si("previousGameType"), "SPECTATOR"), is("dataToKeep", 0)];
    c.play("respawn_creative", "ClientboundRespawnPacket", "respawn", respawn(&spawn, respawn_keep::NOTHING), e);

    // `login` now writes its spawn info through the shared SpawnInfo encoder.
    let login = kiln_proto::packets::Login {
        entity_id: 42,
        // One level: vanilla keeps them in a HashSet of identity-hashed keys, so the order it
        // re-encodes several in varies between runs.
        dimensions: &["minecraft:overworld"],
        max_players: 100,
        view_distance: 10,
        simulation_distance: 8,
        dimension_type: synced("minecraft:dimension_type", "minecraft:overworld"),
        dimension: "minecraft:overworld",
        game_mode: 1,
        is_flat: true,
        sea_level: 63,
        online_mode: true,
        hashed_seed: 987654321012,
        hardcore: true,
        reduced_debug_info: true,
        show_death_screen: false,
        limited_crafting: true,
    };
    let e = vec![
        is("playerId", 42),
        is("hardcore", true),
        is("reducedDebugInfo", true),
        is("showDeathScreen", false),
        is("doLimitedCrafting", true),
        is(&si("seed"), 987654321012i64),
        is("levels", "[ResourceKey[minecraft:dimension / minecraft:overworld]]"),
        is("maxPlayers", 100),
        is("chunkRadius", 10),
        is("simulationDistance", 8),
        is(&si("dimensionType"), "minecraft:overworld"),
        is(&si("gameType"), "CREATIVE"),
        is(&si("previousGameType"), "empty"),
        is(&si("isFlat"), true),
        is(&si("lastDeathLocation"), "empty"),
        is(&si("seaLevel"), 63),
        is("onlineMode", true),
        is("enforcesSecureChat", false),
    ];
    c.play("login", "ClientboundLoginPacket", "login", kiln_proto::packets::play_login(&login), e);

    let p = set_simulation_distance(12);
    let e = vec![is("simulationDistance", 12)];
    c.play("set_simulation_distance", "ClientboundSetSimulationDistancePacket", "set_simulation_distance", p, e);
    let p = set_chunk_cache_radius(32);
    c.play("set_chunk_cache_radius", "ClientboundSetChunkCacheRadiusPacket", "set_chunk_cache_radius", p, vec![is("radius", 32)]);
    let e = vec![is("cooldownGroup", "minecraft:ender_pearl"), is("duration", 20)];
    c.play("cooldown", "ClientboundCooldownPacket", "cooldown", cooldown("minecraft:ender_pearl", 20), e);
    let e = vec![is("tickRate", 20.0), is("isFrozen", false)];
    c.play("ticking_state", "ClientboundTickingStatePacket", "ticking_state", ticking_state(20.0, false), e);
    let e = vec![is("tickRate", 2.5), is("isFrozen", true)];
    c.play("ticking_state_frozen", "ClientboundTickingStatePacket", "ticking_state", ticking_state(2.5, true), e);
    c.play("ticking_step", "ClientboundTickingStepPacket", "ticking_step", ticking_step(100), vec![is("tickSteps", 100)]);
    c.play("set_camera", "ClientboundSetCameraPacket", "set_camera", set_camera(4242), vec![is("cameraId", 4242)]);
}

pub fn entity_status(c: &mut Cases) {
    let e = vec![is("entityId", 70000), is("eventId", 35)];
    c.play("entity_event", "ClientboundEntityEventPacket", "entity_event", entity_event(70000, 35), e);
    let e = vec![is("entityId", -1), is("eventId", -128)];
    c.play("entity_event_neg", "ClientboundEntityEventPacket", "entity_event", entity_event(-1, 128), e);

    let arrow = synced("minecraft:damage_type", "minecraft:arrow");
    let p = damage_event(10, arrow, Some(0), Some(11), Some([1.5, 64.0, -3.25]));
    let e = vec![
        is("entityId", 10),
        is("sourceType", "minecraft:arrow"),
        is("sourceCauseId", 0),
        is("sourceDirectId", 11),
        is("sourcePosition.x", 1.5),
        is("sourcePosition.y", 64.0),
        is("sourcePosition.z", -3.25),
    ];
    c.play("damage_event", "ClientboundDamageEventPacket", "damage_event", p, e);
    let fall = synced("minecraft:damage_type", "minecraft:fall");
    let p = damage_event(10, fall, None, None, None);
    let e = vec![
        is("sourceType", "minecraft:fall"),
        is("sourceCauseId", -1),
        is("sourceDirectId", -1),
        is("sourcePosition", "empty"),
    ];
    c.play("damage_event_fall", "ClientboundDamageEventPacket", "damage_event", p, e);

    let e = vec![is("id", 3), is("yaw", -45.5)];
    c.play("hurt_animation", "ClientboundHurtAnimationPacket", "hurt_animation", hurt_animation(3, -45.5), e);
    let e = vec![is("itemId", 900), is("playerId", 1), is("amount", 64)];
    c.play("take_item_entity", "ClientboundTakeItemEntityPacket", "take_item_entity", take_item_entity(900, 1, 64), e);

    let speed = builtin("minecraft:attribute", "minecraft:movement_speed");
    let health = builtin("minecraft:attribute", "minecraft:max_health");
    let sprint = [AttributeModifier { id: "minecraft:sprinting", amount: 0.3, operation: ModifierOperation::AddMultipliedTotal }];
    let boosts = [
        AttributeModifier { id: "kiln:bonus", amount: 4.0, operation: ModifierOperation::AddValue },
        AttributeModifier { id: "kiln:scale", amount: -0.5, operation: ModifierOperation::AddMultipliedBase },
    ];
    let attrs = [
        AttributeSnapshot { attribute: speed, base: 0.1, modifiers: &sprint },
        AttributeSnapshot { attribute: health, base: 20.0, modifiers: &boosts },
        AttributeSnapshot { attribute: health, base: 40.0, modifiers: &[] },
    ];
    let a = |i: usize, f: &str| format!("attributes[{i}].{f}");
    let e = vec![
        is("entityId", 8),
        is(&a(0, "attribute"), "minecraft:movement_speed"),
        is(&a(0, "base"), 0.1),
        is(&a(0, "modifiers[0].id"), "minecraft:sprinting"),
        near(&a(0, "modifiers[0].amount"), 0.3),
        is(&a(0, "modifiers[0].operation"), "ADD_MULTIPLIED_TOTAL"),
        is(&a(1, "attribute"), "minecraft:max_health"),
        is(&a(1, "modifiers[0].id"), "kiln:bonus"),
        is(&a(1, "modifiers[0].operation"), "ADD_VALUE"),
        is(&a(1, "modifiers[1].amount"), -0.5),
        is(&a(1, "modifiers[1].operation"), "ADD_MULTIPLIED_BASE"),
        is(&a(2, "base"), 40.0),
        is(&a(2, "modifiers"), "[]"),
    ];
    c.play("update_attributes", "ClientboundUpdateAttributesPacket", "update_attributes", update_attributes(8, &attrs), e);
    let p = update_attributes(8, &[]);
    c.play("update_attributes_empty", "ClientboundUpdateAttributesPacket", "update_attributes", p, vec![is("attributes", "[]")]);

    let regen = builtin("minecraft:mob_effect", "minecraft:regeneration");
    let effect = MobEffect {
        effect: regen,
        amplifier: 1,
        duration: 600,
        flags: effect_flags::VISIBLE | effect_flags::SHOW_ICON,
    };
    let e = vec![
        is("entityId", 5),
        is("effect", "minecraft:regeneration"),
        is("effectAmplifier", 1),
        is("effectDurationTicks", 600),
        is("flags", 6),
    ];
    c.play("update_mob_effect", "ClientboundUpdateMobEffectPacket", "update_mob_effect", update_mob_effect(5, &effect), e);
    let darkness = builtin("minecraft:mob_effect", "minecraft:darkness");
    let effect = MobEffect { effect: darkness, amplifier: 255, duration: -1, flags: effect_flags::AMBIENT | effect_flags::BLEND };
    let e = vec![is("effect", "minecraft:darkness"), is("effectAmplifier", 255), is("effectDurationTicks", -1), is("flags", 9)];
    c.play("update_mob_effect_infinite", "ClientboundUpdateMobEffectPacket", "update_mob_effect", update_mob_effect(5, &effect), e);
    let e = vec![is("entityId", 5), is("effect", "minecraft:regeneration")];
    c.play("remove_mob_effect", "ClientboundRemoveMobEffectPacket", "remove_mob_effect", remove_mob_effect(5, regen), e);

    // Existing entity packets that protocol.toml had listed as deferred.
    let e = vec![is("id", 7), is("action", 2)];
    c.play("animate_magic_crit", "ClientboundAnimatePacket", "animate", animate(7, animation::MAGIC_CRITICAL_HIT), e);
}
