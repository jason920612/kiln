//! Play-state protocol features: cookies, resource pack statuses, transfers and dialogs.

use kiln_link::{PlayIn, ToSim};
use kiln_proto::packets::common::{CookieResponse, ResourcePackAction};
use kiln_sim::lobby::PackOffer;
use kiln_sim::testing::{Client, join};
use kiln_sim::{Sim, SimConfig};
use std::sync::atomic::Ordering::Relaxed;
use uuid::Uuid;

fn joined(require_pack: bool) -> (Sim, Client) {
    let mut config = SimConfig::new(4, 4, None);
    config.require_resource_pack = require_pack;
    let mut sim = Sim::new(config);
    let (msg, stats) = join(1, "Lobby", 2);
    stats.count_ids.store(true, Relaxed);
    assert!(sim.step([msg]));
    let mut client = Client::new(1, stats);
    for _ in 0..3 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    (sim, client)
}

fn sent(client: &Client, id: i32) -> u64 {
    client.stats.by_id.lock().unwrap().get(&id).map_or(0, |e| e.0)
}

#[test]
fn cookies_answer_requests_only() {
    use kiln_data::packets::play::clientbound as cb;
    let (mut sim, client) = joined(false);
    sim.store_cookie(1, "kiln:session", b"abc");
    sim.request_cookie(1, "kiln:session");
    assert!(sim.step([]));
    assert_eq!((sent(&client, cb::STORE_COOKIE), sent(&client, cb::COOKIE_REQUEST)), (1, 1));
    assert_eq!(sim.cookie(1, "kiln:session"), None, "not answered yet");
    let answer = |payload: Option<&[u8]>| {
        ToSim::Packet(
            1,
            PlayIn::CookieResponse(CookieResponse { key: "kiln:session".into(), payload: payload.map(<[u8]>::to_vec) }),
        )
    };
    assert!(sim.step([answer(Some(b"abc"))]));
    assert_eq!(sim.cookie(1, "kiln:session"), Some(Some(b"abc".to_vec())));
    assert!(!client.stats.disconnected.load(Relaxed));
    // A second, unrequested answer is a protocol violation, as in vanilla.
    assert!(sim.step([answer(None)]));
    assert!(client.stats.disconnected.load(Relaxed));
}

#[test]
fn resource_pack_statuses_and_required_packs() {
    use kiln_data::packets::play::clientbound as cb;
    let id = Uuid::from_u128(9);
    let (mut sim, client) = joined(false);
    sim.push_resource_pack(1, &PackOffer { id, url: "http://x/p.zip", hash: "", required: false, prompt: None });
    sim.transfer(1, "example.com", 25565);
    assert!(sim.step([ToSim::Packet(1, PlayIn::ResourcePack { id, action: ResourcePackAction::Declined })]));
    assert_eq!((sent(&client, cb::RESOURCE_PACK_PUSH), sent(&client, cb::TRANSFER)), (1, 1));
    assert_eq!(sim.resource_pack_status(1, id), Some(ResourcePackAction::Declined));
    assert!(!client.stats.disconnected.load(Relaxed), "optional packs may be declined");

    let (mut sim, client) = joined(true);
    assert!(sim.step([ToSim::Packet(1, PlayIn::ResourcePack { id, action: ResourcePackAction::Accepted })]));
    assert!(!client.stats.disconnected.load(Relaxed));
    assert!(sim.step([ToSim::Packet(1, PlayIn::ResourcePack { id, action: ResourcePackAction::Declined })]));
    assert!(client.stats.disconnected.load(Relaxed), "the server requires its pack");
}

#[test]
fn dialog_and_transfer_commands_send_packets() {
    use kiln_data::packets::play::clientbound as cb;
    let (mut sim, client) = joined(false);
    assert!(sim.step([
        ToSim::Console("dialog show Lobby minecraft:server_links".into()),
        ToSim::Console("dialog show @a {type:\"minecraft:notice\",title:\"Hi\"}".into()),
        ToSim::Console("dialog clear Lobby".into()),
        ToSim::Console("transfer example.com 25566 Lobby".into()),
    ]));
    assert_eq!(sent(&client, cb::SHOW_DIALOG), 2);
    assert_eq!(sent(&client, cb::CLEAR_DIALOG), 1);
    assert_eq!(sent(&client, cb::TRANSFER), 1);
}
