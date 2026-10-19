//! wp83: prints what Kiln sends a client in the configuration phase for the data packs of a world,
//! in the form `tools/wp83/CheckRegistries.java` compares with vanilla's:
//!
//! ```text
//! entry <registry> <name> <snbt of the data, or - when the client takes the entry from minecraft:core>
//! tags <registry> <tag> <id,id,...>
//! ```
//!
//! usage: wp83_dump <world dir>   (the packs are in <world dir>/datapacks)

use kiln_proto::Reader;
use kiln_sim::{Sim, SimConfig};

fn main() {
    let world = std::path::PathBuf::from(std::env::args().nth(1).expect("world dir"));
    let config = SimConfig::new(2, 2, Some(world));
    let sync = config.data_sync.clone();
    let sim = Sim::new(config);
    match sync.registry_packets() {
        Some(packets) => {
            for p in packets.iter() {
                let mut r = Reader::new(p);
                r.varint().unwrap();
                let registry = r.string(32767).unwrap().to_owned();
                for _ in 0..r.len().unwrap() {
                    let name = r.string(32767).unwrap().to_owned();
                    if r.bool().unwrap() {
                        let at = p.len() - r.remaining();
                        let (tag, used) = kiln_proto::nbt::read_network(&p[at..]).unwrap();
                        r.bytes(used).unwrap();
                        println!("entry {registry} {name} {}", kiln_command::snbt::to_snbt(&tag));
                    } else {
                        println!("entry {registry} {name} -");
                    }
                }
            }
        }
        None => {
            for (registry, names) in kiln_data::registries::SYNCHRONIZED {
                for name in *names {
                    println!("entry {registry} {name} -");
                }
            }
        }
    }
    let tags = sync.config_tags().map(|p| decode_tags(&p)).unwrap_or_else(|| {
        kiln_data::registries::TAGS
            .iter()
            .map(|(r, tags)| ((*r).to_owned(), tags.iter().map(|(t, ids)| ((*t).to_owned(), ids.to_vec())).collect()))
            .collect()
    });
    for (registry, list) in tags {
        for (tag, ids) in list {
            let ids: Vec<String> = ids.iter().map(ToString::to_string).collect();
            println!("tags {registry} {tag} {}", ids.join(","));
        }
    }
    drop(sim);
}

fn decode_tags(p: &bytes::Bytes) -> Vec<(String, Vec<(String, Vec<i32>)>)> {
    let mut r = Reader::new(p);
    r.varint().unwrap();
    let mut out = Vec::new();
    for _ in 0..r.len().unwrap() {
        let registry = r.string(32767).unwrap().to_owned();
        let mut tags = Vec::new();
        for _ in 0..r.len().unwrap() {
            let tag = r.string(32767).unwrap().to_owned();
            let ids = (0..r.len().unwrap()).map(|_| r.varint().unwrap()).collect();
            tags.push((tag, ids));
        }
        out.push((registry, tags));
    }
    out
}
