//! Configuration/play common packets, cookies and pings.

use super::{Cases, compound, is, s, synced};
use kiln_proto::packets::common::*;
use uuid::Uuid;

const CONFIG: &str = "configuration";

/// Adds the configuration and play variants of a packet that exists in both.
fn both(c: &mut Cases, name: &str, class: &str, packet_name: &str, make: impl Fn(Phase) -> bytes::Bytes, expect: impl Fn() -> Vec<super::E>) {
    c.add(&format!("{name}_config"), class, CONFIG, packet_name, make(Phase::Configuration), expect());
    c.add(name, class, "play", packet_name, make(Phase::Play), expect());
}

pub fn common(c: &mut Cases) {
    let id = Uuid::from_u128(0xfedc_ba98_7654_4321_8fed_cba9_8765_4321);
    let prompt = compound(&[("text", s("Please accept")), ("color", s("red"))]);
    let pack = ResourcePack {
        id,
        url: "https://example.com/pack.zip",
        hash: "0123456789abcdef0123456789abcdef01234567",
        required: true,
        prompt: Some(&prompt),
    };
    let push = "common.ClientboundResourcePackPushPacket";
    both(c, "resource_pack_push", push, "resource_pack_push", |p| resource_pack_push(p, &pack), || {
        vec![
            is("id", id),
            is("url", "https://example.com/pack.zip"),
            is("hash", "0123456789abcdef0123456789abcdef01234567"),
            is("required", true),
            is("prompt", "Please accept"),
            is("prompt.color", "red"),
        ]
    });
    let optional = ResourcePack { hash: "", required: false, prompt: None, ..pack };
    let p = resource_pack_push(Phase::Play, &optional);
    let e = vec![is("hash", ""), is("required", false), is("prompt", "empty")];
    c.add("resource_pack_push_optional", push, "play", "resource_pack_push", p, e);
    let pop = "common.ClientboundResourcePackPopPacket";
    both(c, "resource_pack_pop", pop, "resource_pack_pop", |p| resource_pack_pop(p, Some(id)), || vec![is("id", id)]);
    let p = resource_pack_pop(Phase::Play, None);
    c.add("resource_pack_pop_all", pop, "play", "resource_pack_pop", p, vec![is("id", "empty")]);

    let transfer_class = "common.ClientboundTransferPacket";
    both(c, "transfer", transfer_class, "transfer", |p| transfer(p, "lobby.example.net", 25577), || {
        vec![is("host", "lobby.example.net"), is("port", 25577)]
    });

    let request = "cookie.ClientboundCookieRequestPacket";
    for (name, phase, state) in [
        ("cookie_request_login", CookiePhase::Login, "login"),
        ("cookie_request_config", CookiePhase::Configuration, CONFIG),
        ("cookie_request", CookiePhase::Play, "play"),
    ] {
        c.add(name, request, state, "cookie_request", cookie_request(phase, "kiln:session"), vec![is("key", "kiln:session")]);
    }
    let store = "common.ClientboundStoreCookiePacket";
    both(c, "store_cookie", store, "store_cookie", |p| store_cookie(p, "kiln:session", &[1, 2, 3, 0xff]), || {
        vec![is("key", "kiln:session"), is("payload", "010203ff")]
    });
    let big = vec![0x5a; MAX_COOKIE_LEN];
    let p = store_cookie(Phase::Play, "kiln:big", &big);
    c.add("store_cookie_max", store, "play", "store_cookie", p, vec![is("key", "kiln:big")]);

    let label = compound(&[("text", s("Discord")), ("color", s("blue"))]);
    let links = [
        (LinkLabel::Known(KnownLink::BugReport), "https://example.com/bugs"),
        (LinkLabel::Custom(&label), "https://discord.example.com"),
        (LinkLabel::Known(KnownLink::Announcements), "https://example.com/news"),
    ];
    both(c, "server_links", "common.ClientboundServerLinksPacket", "server_links", |p| server_links(p, &links), || {
        vec![
            is("links[0].type.value", "BUG_REPORT"),
            is("links[0].link", "https://example.com/bugs"),
            is("links[1].type.value", "Discord"),
            is("links[1].type.value.color", "blue"),
            is("links[1].link", "https://discord.example.com"),
            is("links[2].type.value", "ANNOUNCEMENTS"),
        ]
    });
    let p = server_links(Phase::Play, &[]);
    c.add("server_links_empty", "common.ClientboundServerLinksPacket", "play", "server_links", p, vec![is("links", "[]")]);

    // Entries in the order vanilla's HashMap re-encodes them.
    let details = [("kiln_version", "0.1.0"), ("world", "lobby")];
    let report = "common.ClientboundCustomReportDetailsPacket";
    both(c, "custom_report_details", report, "custom_report_details", |p| custom_report_details(p, &details), || {
        vec![is("details[kiln_version]", "0.1.0"), is("details[world]", "lobby")]
    });
    let p = custom_report_details(Phase::Play, &[]);
    c.add("custom_report_details_empty", report, "play", "custom_report_details", p, vec![is("details", "{}")]);

    let quick = synced("minecraft:dialog", "minecraft:server_links");
    let show = "common.ClientboundShowDialogPacket";
    let p = show_dialog_play(&Dialog::Registered(quick));
    c.add("show_dialog_registered", show, "play", "show_dialog", p, vec![is("dialog", "minecraft:server_links")]);
    let body = compound(&[("type", s("minecraft:plain_message")), ("contents", s("Be nice."))]);
    // "title", "body" and "type" share a HashMap bucket: vanilla's insertion order.
    let notice = compound(&[("title", s("Rules")), ("body", body), ("type", s("minecraft:notice"))]);
    let e = || vec![is("dialog", "<direct>"), is("dialog.value.common.title", "Rules")];
    c.add("show_dialog_inline", show, "play", "show_dialog", show_dialog_play(&Dialog::Inline(&notice)), e());
    let p = show_dialog_configuration(&notice);
    c.add("show_dialog_config", "common.ClientboundShowDialogPacket#CONTEXT_FREE_STREAM_CODEC", CONFIG, "show_dialog", p, e());
    both(c, "clear_dialog", "common.ClientboundClearDialogPacket", "clear_dialog", clear_dialog, Vec::new);

    let conduct = "Be excellent to each other. No griefing.";
    let e = vec![is("codeOfConduct", conduct)];
    let p = code_of_conduct(conduct);
    c.add("code_of_conduct", "configuration.ClientboundCodeOfConductPacket", CONFIG, "code_of_conduct", p, e);

    both(c, "ping", "common.ClientboundPingPacket", "ping", |p| ping(p, -123456), || vec![is("id", -123456)]);
    let p = keep_alive_configuration(1 << 40);
    c.add("keep_alive_config", "common.ClientboundKeepAlivePacket", CONFIG, "keep_alive", p, vec![is("id", 1i64 << 40)]);
    let p = pong_response(1_700_000_000_123);
    c.add("pong_response", "ping.ClientboundPongResponsePacket", "play", "pong_response", p, vec![is("time", 1_700_000_000_123i64)]);
}
