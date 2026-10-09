#[test]
fn list_unsimulated() {
    let mut out = Vec::new();
    for t in kiln_data::entities::TYPES.iter() {
        if !kiln_entity::persist::is_simulated(t.name) {
            out.push(t.name);
        }
    }
    eprintln!("UNSIM {}: {:?}", out.len(), out);
}
