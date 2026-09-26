//! Data component types and default item components for kiln-item, from the output of
//! `tools/ItemVectors.java defaults` (run by `cargo xtask extract`).
//!
//! `components.rs` lists the component types in registry order. `item_defaults.bin` holds each
//! item's default components as network-encoded values, deduplicated. Layout (little endian):
//! magic "KID1", item count u32, value count u32; per value its component type u16, byte length
//! u32 and bytes; per item (in id order) a value count u16 and that many value indices u16.

use crate::codegen::{HEADER, const_name};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// Runs the Java extractor against the server jar and its libraries.
pub fn extract(root: &Path, work: &Path, server_jar: &Path) -> Result<()> {
    let mut cp = vec![server_jar.to_path_buf()];
    crate::block_props::collect_jars(&work.join("libraries"), &mut cp)?;
    let cp = std::env::join_paths(&cp)?;
    let out = work.join("generated/extra");
    std::fs::create_dir_all(&out)?;
    let status = Command::new("java")
        .arg("-cp")
        .arg(cp)
        .arg(root.join("tools/ItemVectors.java"))
        .arg("defaults")
        .arg(out.join("item_components.json"))
        .current_dir(work)
        .status()
        .context("running java (JDK 25 must be on PATH)")?;
    if !status.success() {
        bail!("item extractor failed");
    }
    Ok(())
}

pub fn generate(json: &Value, out: &Path) -> Result<()> {
    let components = json["components"].as_array().context("components")?;
    let items = json["items"].as_array().context("items")?;

    let mut s = String::from(HEADER);
    writeln!(s, "//! Data component types in registry (network id) order.\n")?;
    writeln!(s, "pub const NAMES: &[&str] = &[")?;
    for c in components {
        writeln!(s, "    {:?},", c["name"].as_str().context("name")?)?;
    }
    writeln!(s, "];\n\n/// Whether the type has a persistent codec (is saved and hashed).")?;
    writeln!(s, "pub const PERSISTENT: &[bool] = &[")?;
    for c in components {
        writeln!(s, "    {},", c["persistent"].as_bool().context("persistent")?)?;
    }
    writeln!(s, "];\n")?;
    for (id, c) in components.iter().enumerate() {
        writeln!(s, "pub const {}: u16 = {id};", const_name(c["name"].as_str().unwrap()))?;
    }
    std::fs::write(out.join("components.rs"), s)?;

    let mut values: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut index: HashMap<(u16, Vec<u8>), u16> = HashMap::new();
    let mut body = Vec::new();
    for item in items {
        let mut comps: Vec<(u16, Vec<u8>)> = item["components"]
            .as_array()
            .context("item components")?
            .iter()
            .map(|c| {
                let ty = c[0].as_u64().context("type")? as u16;
                let bytes = hex(c[1].as_str().context("value")?)?;
                Ok((ty, bytes))
            })
            .collect::<Result<_>>()?;
        comps.sort();
        body.extend_from_slice(&(comps.len() as u16).to_le_bytes());
        for key in comps {
            let next = values.len();
            let i = *index.entry(key.clone()).or_insert_with(|| {
                values.push(key);
                next as u16
            });
            body.extend_from_slice(&i.to_le_bytes());
        }
    }
    ensure!(values.len() <= u16::MAX as usize, "{} distinct default values no longer fit in u16", values.len());
    let mut bin = b"KID1".to_vec();
    bin.extend_from_slice(&(items.len() as u32).to_le_bytes());
    bin.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for (ty, bytes) in &values {
        bin.extend_from_slice(&ty.to_le_bytes());
        bin.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        bin.extend_from_slice(bytes);
    }
    bin.extend_from_slice(&body);
    std::fs::write(out.join("item_defaults.bin"), bin)?;
    println!("codegen: {} component types, {} items, {} distinct default values", components.len(), items.len(), values.len());
    Ok(())
}

fn hex(s: &str) -> Result<Vec<u8>> {
    ensure!(s.len() % 2 == 0, "odd hex length");
    (0..s.len()).step_by(2).map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?)).collect()
}
