//! Which client-syncable attributes a mob has instances of, against vanilla 26.3 (`tools/AttributeVectors.java`, run by
//! `tools/parity_suites.py` with `KILN_ATTR_VECTORS`): `AttributeMap.getSyncableAttributes` lists only the attributes something asked
//! an instance of, so a freshly summoned pig has none (pairing data lists none) and every mob has its movement speed from its first
//! step on. Skipped without the vectors.

use crate::{OVERWORLD_ID, Sim, SimConfig};
use kiln_entity::mob::{self, MobKind};
use kiln_link::ToSim;
use serde_json::Value;

fn names_of(m: &mob::MobData) -> Vec<String> {
    let mut v: Vec<String> = m.attrs.list.iter().filter(|i| i.created && i.attr.is_client_syncable()).map(|i| i.attr.name().to_owned()).collect();
    v.sort();
    v
}

fn vanilla_names(list: &Value) -> Vec<String> {
    let mut v: Vec<String> = list.as_array().unwrap().iter().map(|a| a[0].as_str().unwrap().to_owned()).collect();
    v.sort();
    v
}

#[test]
fn mob_attribute_instances_match_vanilla() {
    let Some(path) = std::env::var_os("KILN_ATTR_VECTORS") else {
        eprintln!("skipped: set KILN_ATTR_VECTORS (tools/AttributeVectors.java)");
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let mut sim = Sim::new(SimConfig::new(2, 3, None));
    let (msg, stats) = crate::testing::join(1, "Probe", 3);
    assert!(sim.step([msg, ToSim::Console("gamemode spectator Probe".into()), ToSim::Console("difficulty easy".into()), ToSim::Console("tp Probe 8 90 8".into())]));
    let mut client = crate::testing::Client::new(1, stats);
    for _ in 0..8 {
        let mut inbox = Vec::new();
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    let (mut ok, mut failed) = (0, Vec::new());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).unwrap();
        if v["loaded"].as_bool() != Some(true) {
            continue;
        }
        let ty = v["type"].as_str().unwrap();
        let Some(kind) = MobKind::by_name(ty) else { continue };
        // Right after `/summon` (what the viewers' pairing data is made from).
        let spawn = crate::mobs::spawn(kind, [8.5, 80.0, 8.5], None, Some(crate::mobs::Finalize::command(crate::mobs::difficulty_instance(2, 0, 0, 1.0), 7, false, false)));
        let e = crate::entities::Entity::new(1, uuid::Uuid::from_u128(1), spawn);
        let fresh = e.phys.as_deref().and_then(mob::data).map(names_of).unwrap_or_default();
        if fresh != vanilla_names(&v["fresh"]) {
            failed.push(format!("{ty}: new {fresh:?}, vanilla {:?}", vanilla_names(&v["fresh"])));
            continue;
        }
        // After its first tick.
        let mut inbox = vec![ToSim::Console(format!("summon {ty} 8.5 80 8.5"))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
        let mut seen = None;
        for region in sim.dims[OVERWORLD_ID].regions.iter() {
            for e in region.part().0.list.iter().filter(|e| !e.removed && e.kind.name == ty) {
                seen = e.phys.as_deref().and_then(mob::data).map(names_of);
            }
        }
        let want = vanilla_names(&v["ticks"][0]);
        match seen {
            Some(s) if s == want => ok += 1,
            Some(s) => failed.push(format!("{ty}: after a tick {s:?}, vanilla {want:?}")),
            None => {}
        }
        let mut inbox = vec![ToSim::Console(format!("kill @e[type={ty}]"))];
        client.tick(None, &mut inbox);
        assert!(sim.step(inbox));
    }
    for f in &failed {
        eprintln!("FAIL {f}");
    }
    eprintln!("mob attribute instances: {ok} match vanilla, {} differ", failed.len());
    assert!(failed.is_empty(), "{} mobs differ from vanilla", failed.len());
}
