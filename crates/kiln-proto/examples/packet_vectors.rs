//! Writes clientbound packet vectors for `tools/packet_vectors.py`, which decodes them with
//! vanilla's codecs: `<name>.bin` (packet body without the id), `<name>.expect` (decoded field
//! values, `path=value` exact or `path~value` approximate) and `manifest.txt`
//! (`name class state/packet id`). The vectors are defined in `tests/vectors`.
//!
//! usage: cargo run -p kiln-proto --example packet_vectors -- <out dir>

#[path = "../tests/vectors/mod.rs"]
mod vectors;

use kiln_proto::Reader;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use vectors::E;

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: packet_vectors <out dir>"));
    fs::create_dir_all(&dir).unwrap();
    let mut manifest = String::new();
    let cases = vectors::cases().0;
    for case in &cases {
        let mut r = Reader::new(&case.packet);
        let id = r.varint().unwrap();
        fs::write(dir.join(format!("{}.bin", case.name)), r.rest()).unwrap();
        let mut expect = String::new();
        for e in &case.expect {
            match e {
                E::Is(p, v) => writeln!(expect, "{p}={v}"),
                E::Near(p, v) => writeln!(expect, "{p}~{v:?}"),
            }
            .unwrap();
        }
        fs::write(dir.join(format!("{}.expect", case.name)), expect).unwrap();
        writeln!(manifest, "{} net.minecraft.network.protocol.{} {} {id}", case.name, case.class, case.key).unwrap();
    }
    fs::write(dir.join("manifest.txt"), manifest).unwrap();
    println!("wrote {} vectors to {}", cases.len(), dir.display());
}
