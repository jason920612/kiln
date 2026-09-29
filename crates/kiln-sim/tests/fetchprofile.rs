//! `fetchprofile`: names and ids resolve through the lookup the server was given, off the tick
//! thread and answered later; without one, offline names resolve to the profile the server
//! would give them; players and their entities answer without asking anyone.

use kiln_link::{LookedUpProfile, ProfileLookup, Property, ToSim};
use kiln_sim::testing::join;
use kiln_sim::{Sim, SimConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

struct Fake(AtomicUsize);

const ALICE: Uuid = Uuid::from_u128(0x0a11_ce00_0000_4000_8000_0000_0000_0001);

impl ProfileLookup for Fake {
    fn by_name(&self, name: &str) -> Result<Option<LookedUpProfile>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        match name {
            "Alice" => Ok(Some(LookedUpProfile {
                id: ALICE,
                name: "Alice".into(),
                properties: vec![Property { name: "textures".into(), value: "dGV4".into(), signature: Some("c2ln".into()) }],
            })),
            "Broken" => Err("the session server is down".into()),
            _ => Ok(None),
        }
    }

    fn by_id(&self, id: Uuid) -> Result<Option<LookedUpProfile>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok((id == ALICE).then(|| LookedUpProfile { id, name: "Alice".into(), properties: Vec::new() }))
    }
}

fn console(sim: &mut Sim, command: &str) -> Vec<String> {
    assert!(sim.step([ToSim::Console(command.into())]));
    sim.take_console()
}

/// Steps until `n` lines were printed, feeding the answers that other threads sent.
fn wait_lines(sim: &mut Sim, rx: &crossbeam_channel::Receiver<ToSim>, n: usize) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..400 {
        let inbox: Vec<ToSim> = rx.try_iter().collect();
        assert!(sim.step(inbox));
        out.extend(sim.take_console());
        if out.len() >= n {
            return out;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("no answer: {out:?}");
}

fn lookup_sim() -> (Sim, crossbeam_channel::Receiver<ToSim>, Arc<Fake>) {
    let (tx, rx) = crossbeam_channel::unbounded();
    let fake = Arc::new(Fake(AtomicUsize::new(0)));
    let mut config = SimConfig::new(4, 2, None);
    config.online_mode = true;
    config.profile_lookup = Some(fake.clone());
    config.replies = Some(tx);
    let mut sim = Sim::new(config);
    sim.capture_console();
    (sim, rx, fake)
}

#[test]
fn a_lookup_is_answered_after_its_command_ran() {
    let (mut sim, rx, fake) = lookup_sim();
    assert!(console(&mut sim, "fetchprofile name Alice").is_empty(), "nothing yet: the lookup runs on another thread");
    let out = wait_lines(&mut sim, &rx, 1);
    assert!(out[0].starts_with("commands.fetchprofile.name.success[Alice, ["), "{out:?}");
    assert!(out[0].contains("commands.fetchprofile.copy_text[[Alice head]]"), "{out:?}");
    assert_eq!(fake.0.load(Ordering::SeqCst), 1);
    // By id, and a name nobody has.
    console(&mut sim, &format!("fetchprofile id {ALICE}"));
    let out = wait_lines(&mut sim, &rx, 1);
    assert!(out[0].starts_with("commands.fetchprofile.id.success["), "{out:?}");
    console(&mut sim, "fetchprofile name Nobody");
    assert_eq!(wait_lines(&mut sim, &rx, 1), ["commands.fetchprofile.name.failure[Nobody]"]);
    // A lookup that fails (the service is down) is a failed lookup.
    console(&mut sim, "fetchprofile name Broken");
    assert_eq!(wait_lines(&mut sim, &rx, 1), ["commands.fetchprofile.name.failure[Broken]"]);
}

#[test]
fn players_answer_without_the_network() {
    let (mut sim, rx, fake) = lookup_sim();
    let (msg, _stats) = join(1, "Steve", 2);
    assert!(sim.step([msg]));
    sim.take_console();
    console(&mut sim, "fetchprofile name steve");
    let out = wait_lines(&mut sim, &rx, 1);
    assert!(out[0].starts_with("commands.fetchprofile.name.success[steve, ["), "{out:?}");
    assert_eq!(fake.0.load(Ordering::SeqCst), 0, "an online player is known without asking anyone");
    // The entity form is immediate.
    let out = console(&mut sim, "fetchprofile entity Steve");
    assert!(out[0].starts_with("commands.fetchprofile.entity.success["), "{out:?}");
}

#[test]
fn offline_servers_answer_from_the_name() {
    let mut sim = Sim::new(SimConfig::new(4, 2, None));
    sim.capture_console();
    assert!(console(&mut sim, "fetchprofile name Notch").is_empty());
    // The answer comes with the next tick.
    let out = console(&mut sim, &format!("fetchprofile id {ALICE}"));
    assert!(out.iter().any(|l| l.starts_with("commands.fetchprofile.name.success[Notch, [")), "{out:?}");
    // No name for an id, offline: the profile does not exist.
    assert!(sim.step([]));
    let out = sim.take_console();
    assert_eq!(out, [format!("commands.fetchprofile.id.failure[{ALICE}]")]);
}
