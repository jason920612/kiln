//! Decodes every serverbound packet the bots send with the vanilla 26.3 codec
//! (`tools/vanilla_decode.py`), which fails on exceptions and trailing bytes.
//!
//! Needs `work/` (server jar and libraries), Python and a JDK:
//! `cargo test -p kiln-bot --test vanilla_codec -- --ignored`

use bytes::BytesMut;
use kiln_bot::proto::{self, Move};
use kiln_data::packets as ids;
use kiln_proto::Reader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use uuid::Uuid;

fn built(f: impl FnOnce(&mut BytesMut)) -> BytesMut {
    let mut b = BytesMut::new();
    f(&mut b);
    b
}

/// (vanilla packet class, packet id + data) for every packet builder in `proto`.
fn vectors() -> Vec<(String, BytesMut)> {
    use ids::configuration::serverbound as csb;
    use ids::play::serverbound as psb;
    let offer = kiln_proto::packets::select_known_packs(&[("minecraft", "core", "26.3")]);
    let pos = [8.5, -60.0, -1234.25];
    let net = |class: &str| format!("net.minecraft.network.protocol.{class}");
    vec![
        (net("handshake.ClientIntentionPacket"), built(|b| proto::intention(b, "localhost", 25565))),
        (net("login.ServerboundHelloPacket"), built(|b| proto::hello(b, "Bot123", Uuid::from_u128(0x1234)))),
        (net("login.ServerboundCustomQueryAnswerPacket"), built(|b| proto::custom_query_answer(b, 42))),
        (net("login.ServerboundLoginAcknowledgedPacket"), built(proto::login_acknowledged)),
        (
            net("cookie.ServerboundCookieResponsePacket"),
            built(|b| proto::cookie_response(b, csb::COOKIE_RESPONSE, "kiln:test")),
        ),
        (
            net("common.ServerboundClientInformationPacket"),
            built(|b| proto::client_information(b, csb::CLIENT_INFORMATION, 2)),
        ),
        (net("common.ServerboundCustomPayloadPacket"), built(|b| proto::brand(b, csb::CUSTOM_PAYLOAD, "kiln-bot"))),
        (net("configuration.ServerboundSelectKnownPacks"), built(|b| proto::select_known_packs(b, &offer[1..]))),
        (net("configuration.ServerboundFinishConfigurationPacket"), built(proto::finish_configuration)),
        (net("configuration.ServerboundAcceptCodeOfConductPacket"), built(proto::accept_code_of_conduct)),
        (net("common.ServerboundKeepAlivePacket"), built(|b| proto::keep_alive(b, psb::KEEP_ALIVE, -123_456_789))),
        (net("common.ServerboundPongPacket"), built(|b| proto::pong(b, psb::PONG, 7))),
        (
            net("game.ServerboundAcceptTeleportationPacket"),
            built(|b| proto::accept_teleportation(b, 1, pos, 90.0, -10.0)),
        ),
        (net("game.ServerboundMovePlayerPacket$Pos"), built(|b| proto::move_player(b, Move::Pos(pos), true))),
        (
            net("game.ServerboundMovePlayerPacket$PosRot"),
            built(|b| proto::move_player(b, Move::PosRot(pos, 45.0, 0.0), true)),
        ),
        (net("game.ServerboundMovePlayerPacket$Rot"), built(|b| proto::move_player(b, Move::Rot(-45.0, 30.0), true))),
        (net("game.ServerboundMovePlayerPacket$StatusOnly"), built(|b| proto::move_player(b, Move::StatusOnly, false))),
        (net("game.ServerboundClientTickEndPacket"), built(proto::client_tick_end)),
        (net("game.ServerboundChunkBatchReceivedPacket"), built(|b| proto::chunk_batch_received(b, 64.0))),
        (net("game.ServerboundPlayerLoadedPacket"), built(proto::player_loaded)),
        (net("game.ServerboundConfigurationAcknowledgedPacket"), built(proto::configuration_acknowledged)),
        (net("game.ServerboundChatPacket"), built(|b| proto::chat(b, "hello #1 from Bot0", 1_790_000_000_000, -99))),
    ]
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
#[ignore = "needs work/ (vanilla server jar), Python and a JDK"]
fn vanilla_decodes_every_serverbound_packet() {
    let root = root();
    let dir = root.join("work/wp-bot/serverbound");
    std::fs::create_dir_all(&dir).unwrap();
    let jobs: Vec<(String, PathBuf)> = vectors()
        .into_iter()
        .map(|(class, packet)| {
            let mut r = Reader::new(&packet);
            r.varint().unwrap(); // the codec reads the body after the packet id
            let file = dir.join(format!("{}.bin", class.rsplit('.').next().unwrap()));
            std::fs::write(&file, r.rest()).unwrap();
            (class, file)
        })
        .collect();

    // A few JVMs at a time: each one bootstraps the game registries.
    let mut failures = Vec::new();
    for batch in jobs.chunks(6) {
        let children: Vec<_> = batch
            .iter()
            .map(|(class, file)| {
                let child = Command::new("python")
                    .arg(root.join("tools/vanilla_decode.py"))
                    .arg(class)
                    .arg(file)
                    .current_dir(&root)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .expect("running python");
                (class, child)
            })
            .collect();
        for (class, child) in children {
            let out = child.wait_with_output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            let ok_line = stdout.lines().find(|l| l.contains("OK: decoded")).map(str::to_owned);
            println!("{class}: {}", ok_line.as_deref().unwrap_or("FAILED"));
            if !out.status.success() || ok_line.is_none() {
                failures.push(format!("{class}\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr)));
            }
        }
    }
    assert!(failures.is_empty(), "vanilla rejected:\n{}", failures.join("\n"));
}
