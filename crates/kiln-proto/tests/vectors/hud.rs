//! HUD, scoreboard and team packets.

use super::{Cases, compound, is, s};
use kiln_proto::nbt::{Tag, text};
use kiln_proto::packets::hud::*;
use kiln_proto::packets::scoreboard::*;
use uuid::Uuid;

pub fn hud(c: &mut Cases) {
    let id = Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef);
    let name = compound(&[("text", s("Wither")), ("color", s("light_purple"))]);
    let add = BossEvent::Add {
        name: &name,
        progress: 0.75,
        color: BossBarColor::Purple,
        overlay: BossBarOverlay::Notched10,
        flags: boss_flags::DARKEN_SCREEN | boss_flags::CREATE_WORLD_FOG,
    };
    let e = vec![
        is("id", id),
        is("operation.@class", "AddOperation"),
        is("operation.name", "Wither"),
        is("operation.name.color", "light_purple"),
        is("operation.progress", 0.75),
        is("operation.color", "PURPLE"),
        is("operation.overlay", "NOTCHED_10"),
        is("operation.darkenScreen", true),
        is("operation.playMusic", false),
        is("operation.createWorldFog", true),
    ];
    c.play("boss_event_add", "ClientboundBossEventPacket", "boss_event", boss_event(id, &add), e);
    let add = BossEvent::Add {
        name: &name,
        progress: 0.0,
        color: BossBarColor::White,
        overlay: BossBarOverlay::Notched20,
        flags: boss_flags::PLAY_BOSS_MUSIC,
    };
    let e = vec![is("operation.color", "WHITE"), is("operation.overlay", "NOTCHED_20"), is("operation.playMusic", true)];
    c.play("boss_event_add_music", "ClientboundBossEventPacket", "boss_event", boss_event(id, &add), e);
    // REMOVE_OPERATION is a lambda singleton.
    let e = vec![is("id", id)];
    c.play("boss_event_remove", "ClientboundBossEventPacket", "boss_event", boss_event(id, &BossEvent::Remove), e);
    let e = vec![is("operation.@class", "UpdateProgressOperation"), is("operation.progress", 0.125)];
    c.play("boss_event_progress", "ClientboundBossEventPacket", "boss_event", boss_event(id, &BossEvent::Progress(0.125)), e);
    let e = vec![is("operation.@class", "UpdateNameOperation"), is("operation.name", "Phase 2")];
    let p = boss_event(id, &BossEvent::Name(&text("Phase 2")));
    c.play("boss_event_name", "ClientboundBossEventPacket", "boss_event", p, e);
    let style = BossEvent::Style { color: BossBarColor::Pink, overlay: BossBarOverlay::Progress };
    let e = vec![is("operation.@class", "UpdateStyleOperation"), is("operation.color", "PINK"), is("operation.overlay", "PROGRESS")];
    c.play("boss_event_style", "ClientboundBossEventPacket", "boss_event", boss_event(id, &style), e);
    let style = BossEvent::Style { color: BossBarColor::Yellow, overlay: BossBarOverlay::Notched12 };
    let e = vec![is("operation.color", "YELLOW"), is("operation.overlay", "NOTCHED_12")];
    c.play("boss_event_style2", "ClientboundBossEventPacket", "boss_event", boss_event(id, &style), e);
    let flags = BossEvent::Flags(boss_flags::DARKEN_SCREEN | boss_flags::PLAY_BOSS_MUSIC | boss_flags::CREATE_WORLD_FOG);
    let e = vec![
        is("operation.@class", "UpdatePropertiesOperation"),
        is("operation.darkenScreen", true),
        is("operation.playMusic", true),
        is("operation.createWorldFog", true),
    ];
    c.play("boss_event_flags", "ClientboundBossEventPacket", "boss_event", boss_event(id, &flags), e);

    let title = compound(&[("text", s("Welcome")), ("bold", Tag::Byte(1)), ("color", s("gold"))]);
    let e = vec![is("text", "Welcome"), is("text.color", "gold")];
    c.play("set_title_text", "ClientboundSetTitleTextPacket", "set_title_text", set_title_text(&title), e);
    let e = vec![is("text", "to the lobby")];
    let p = set_subtitle_text(&text("to the lobby"));
    c.play("set_subtitle_text", "ClientboundSetSubtitleTextPacket", "set_subtitle_text", p, e);
    let e = vec![is("text", "Compass: \u{2191} spawn")];
    let p = set_action_bar_text(&text("Compass: \u{2191} spawn"));
    c.play("set_action_bar_text", "ClientboundSetActionBarTextPacket", "set_action_bar_text", p, e);
    let e = vec![is("fadeIn", 10), is("stay", 70), is("fadeOut", 20)];
    let p = set_titles_animation(10, 70, 20);
    c.play("set_titles_animation", "ClientboundSetTitlesAnimationPacket", "set_titles_animation", p, e);
    c.play("clear_titles_reset", "ClientboundClearTitlesPacket", "clear_titles", clear_titles(true), vec![is("resetTimes", true)]);
    c.play("clear_titles", "ClientboundClearTitlesPacket", "clear_titles", clear_titles(false), vec![is("resetTimes", false)]);
    let lobby = compound(&[("text", s("Lobby")), ("color", s("aqua"))]);
    let header = compound(&[("text", s("Kiln ")), ("extra", Tag::List(vec![lobby]))]);
    let e = vec![is("header", "Kiln Lobby"), is("footer", "")];
    c.play("tab_list", "ClientboundTabListPacket", "tab_list", tab_list(&header, &text("")), e);
}

pub fn scoreboard(c: &mut Cases) {
    let title = compound(&[("text", s("LOBBY")), ("color", s("yellow"))]);
    let gold = compound(&[("color", s("gold")), ("bold", Tag::Byte(1))]);
    let fixed = text("--");

    let obj = Objective { display_name: &title, render_type: RenderType::Integer, number_format: None };
    let e = vec![
        is("objectiveName", "sidebar"),
        is("method", 0),
        is("displayName", "LOBBY"),
        is("displayName.color", "yellow"),
        is("renderType", "INTEGER"),
        is("numberFormat", "empty"),
    ];
    let p = set_objective("sidebar", &ObjectiveMethod::Add(obj));
    c.play("set_objective_add", "ClientboundSetObjectivePacket", "set_objective", p, e);
    let obj = Objective { number_format: Some(NumberFormat::Blank), ..obj };
    let e = vec![is("method", 0), is("numberFormat.@class", "BlankFormat")];
    let p = set_objective("sidebar", &ObjectiveMethod::Add(obj));
    c.play("set_objective_add_blank", "ClientboundSetObjectivePacket", "set_objective", p, e);
    let obj = Objective { render_type: RenderType::Hearts, number_format: Some(NumberFormat::Styled(&gold)), ..obj };
    let e = vec![
        is("method", 2),
        is("renderType", "HEARTS"),
        is("numberFormat.@class", "StyledFormat"),
        is("numberFormat.style.bold", true),
        is("numberFormat.style.color.name", "gold"),
    ];
    let p = set_objective("health", &ObjectiveMethod::Change(obj));
    c.play("set_objective_change_styled", "ClientboundSetObjectivePacket", "set_objective", p, e);
    let obj = Objective { number_format: Some(NumberFormat::Fixed(&fixed)), ..obj };
    let e = vec![is("numberFormat.@class", "FixedFormat"), is("numberFormat.value", "--")];
    let p = set_objective("health", &ObjectiveMethod::Change(obj));
    c.play("set_objective_change_fixed", "ClientboundSetObjectivePacket", "set_objective", p, e);
    let e = vec![is("objectiveName", "health"), is("method", 1)];
    let p = set_objective("health", &ObjectiveMethod::Remove);
    c.play("set_objective_remove", "ClientboundSetObjectivePacket", "set_objective", p, e);

    let e = vec![
        is("owner", "Steve"),
        is("objectiveName", "sidebar"),
        is("score", 15),
        is("display", "empty"),
        is("numberFormat", "empty"),
    ];
    c.play("set_score", "ClientboundSetScorePacket", "set_score", set_score("Steve", "sidebar", 15, None, None), e);
    let line = compound(&[("text", s("Players: 12")), ("color", s("green"))]);
    let e = vec![
        is("score", -3),
        is("display", "Players: 12"),
        is("display.color", "green"),
        is("numberFormat.@class", "BlankFormat"),
    ];
    let p = set_score("#line3", "sidebar", -3, Some(&line), Some(&NumberFormat::Blank));
    c.play("set_score_display", "ClientboundSetScorePacket", "set_score", p, e);
    let e = vec![is("numberFormat.@class", "FixedFormat"), is("numberFormat.value", "--")];
    let p = set_score("#line3", "sidebar", 1_000_000, None, Some(&NumberFormat::Fixed(&fixed)));
    c.play("set_score_fixed", "ClientboundSetScorePacket", "set_score", p, e);

    let e = vec![is("owner", "Steve"), is("objectiveName", "sidebar")];
    c.play("reset_score", "ClientboundResetScorePacket", "reset_score", reset_score("Steve", Some("sidebar")), e);
    let e = vec![is("owner", "Steve"), is("objectiveName", "null")];
    c.play("reset_score_all", "ClientboundResetScorePacket", "reset_score", reset_score("Steve", None), e);

    for (name, slot, want) in [
        ("sidebar", display_slot::SIDEBAR, "SIDEBAR"),
        ("list", display_slot::LIST, "LIST"),
        ("below_name", display_slot::BELOW_NAME, "BELOW_NAME"),
        ("team_red", display_slot::team_sidebar(TeamColor::Red), "TEAM_RED"),
        ("team_white", display_slot::team_sidebar(TeamColor::White), "TEAM_WHITE"),
    ] {
        let e = vec![is("slot", want), is("objectiveName", "obj")];
        let p = set_display_objective(slot, "obj");
        c.play(&format!("set_display_objective_{name}"), "ClientboundSetDisplayObjectivePacket", "set_display_objective", p, e);
    }
    let e = vec![is("slot", "SIDEBAR"), is("objectiveName", "")];
    let p = set_display_objective(display_slot::SIDEBAR, "");
    c.play("set_display_objective_clear", "ClientboundSetDisplayObjectivePacket", "set_display_objective", p, e);

    let display = text("Red Team");
    let prefix = compound(&[("text", s("[R] ")), ("color", s("red"))]);
    let suffix = text(" *");
    let params = TeamParameters {
        display_name: &display,
        prefix: &prefix,
        suffix: &suffix,
        name_tag_visibility: TeamRule::OtherTeams,
        collision_rule: TeamRule::Never,
        color: Some(TeamColor::Red),
        friendly_fire: false,
        see_friendly_invisibles: true,
    };
    let members = ["Steve", "Alex", "5f4d1c2a-9b3e-4f60-8a7d-2e1c0b9a8f7e"];
    let pe = |p: &str| format!("parameters.{p}");
    let e = vec![
        is("name", "red"),
        is("method", 0),
        is(&pe("displayName"), "Red Team"),
        is(&pe("playerPrefix"), "[R] "),
        is(&pe("playerPrefix.color"), "red"),
        is(&pe("playerSuffix"), " *"),
        is(&pe("nameTagVisibility"), "HIDE_FOR_OTHER_TEAMS"),
        is(&pe("collisionRule"), "NEVER"),
        is(&pe("color"), "RED"),
        is(&pe("options"), 2),
        is("players[0]", "Steve"),
        is("players[1]", "Alex"),
        is("players[2]", members[2]),
    ];
    let p = set_player_team("red", &TeamMethod::Add(params, &members));
    c.play("set_player_team_add", "ClientboundSetPlayerTeamPacket", "set_player_team", p, e);
    let params = TeamParameters {
        name_tag_visibility: TeamRule::OwnTeam,
        collision_rule: TeamRule::OwnTeam,
        color: None,
        friendly_fire: true,
        see_friendly_invisibles: false,
        ..params
    };
    let e = vec![
        is("method", 2),
        is(&pe("nameTagVisibility"), "HIDE_FOR_OWN_TEAM"),
        is(&pe("collisionRule"), "PUSH_OWN_TEAM"),
        is(&pe("color"), "empty"),
        is(&pe("options"), 1),
        is("players", "[]"),
    ];
    let p = set_player_team("red", &TeamMethod::Change(params));
    c.play("set_player_team_change", "ClientboundSetPlayerTeamPacket", "set_player_team", p, e);
    let params = TeamParameters {
        name_tag_visibility: TeamRule::Always,
        collision_rule: TeamRule::OtherTeams,
        color: Some(TeamColor::Black),
        friendly_fire: true,
        see_friendly_invisibles: true,
        ..params
    };
    let e = vec![
        is(&pe("nameTagVisibility"), "ALWAYS"),
        is(&pe("collisionRule"), "PUSH_OTHER_TEAMS"),
        is(&pe("color"), "BLACK"),
        is(&pe("options"), 3),
        is("players", "[]"),
    ];
    let p = set_player_team("black", &TeamMethod::Add(params, &[]));
    c.play("set_player_team_add_empty", "ClientboundSetPlayerTeamPacket", "set_player_team", p, e);
    let e = vec![is("name", "red"), is("method", 1), is("parameters", "empty"), is("players", "[]")];
    c.play("set_player_team_remove", "ClientboundSetPlayerTeamPacket", "set_player_team", set_player_team("red", &TeamMethod::Remove), e);
    let e = vec![is("method", 3), is("players[0]", "Notch"), is("parameters", "empty")];
    let p = set_player_team("red", &TeamMethod::Join(&["Notch"]));
    c.play("set_player_team_join", "ClientboundSetPlayerTeamPacket", "set_player_team", p, e);
    let e = vec![is("method", 4), is("players[0]", "Steve"), is("players[1]", "Alex")];
    let p = set_player_team("red", &TeamMethod::Leave(&["Steve", "Alex"]));
    c.play("set_player_team_leave", "ClientboundSetPlayerTeamPacket", "set_player_team", p, e);
}
