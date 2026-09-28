//! Runs tools/ExtractMobBlocks.java and packs its per-state facts into the table kiln-entity's
//! mob code embeds (`crates/kiln-entity/src/gen/mob_blocks.bin`).
//!
//! Layout: magic "KMB1", state count u32 (little endian), then per state the path type ordinal
//! u8 and a flags u8 (see `FLAGS`).

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

/// Bit order of the per-state flags.
pub const FLAGS: &[&str] = &["pf", "vs_zombie", "vs_pig", "empty_zombie", "empty_pig", "full"];

pub fn run(root: &Path, work: &Path, server_jar: &Path, state_count: usize) -> Result<()> {
    let mut cp = vec![server_jar.to_path_buf()];
    crate::block_props::collect_jars(&work.join("libraries"), &mut cp)?;
    let cp = std::env::join_paths(&cp)?;
    let json_path = work.join("generated/extra/mob_blocks.json");
    std::fs::create_dir_all(json_path.parent().unwrap())?;
    let status = Command::new("java")
        .arg("-cp")
        .arg(&cp)
        .arg(root.join("tools/ExtractMobBlocks.java"))
        .arg(&json_path)
        .current_dir(work)
        .status()
        .context("running java (JDK 25 must be on PATH)")?;
    if !status.success() {
        bail!("ExtractMobBlocks.java failed");
    }
    let json = crate::read_json(&json_path)?;
    let states = json.as_array().context("mob_blocks.json")?;
    if states.len() != state_count {
        bail!("extractor saw {} states, blocks.json has {state_count}", states.len());
    }
    let mut out = b"KMB1".to_vec();
    out.extend_from_slice(&(states.len() as u32).to_le_bytes());
    for (i, s) in states.iter().enumerate() {
        if s["id"].as_u64() != Some(i as u64) {
            bail!("state {i} out of order");
        }
        out.push(u8::try_from(s["pt"].as_u64().context("pt")?)?);
        let mut flags = 0u8;
        for (bit, name) in FLAGS.iter().enumerate() {
            if s[*name].as_bool().with_context(|| format!("state {i}: {name}"))? {
                flags |= 1 << bit;
            }
        }
        out.push(flags);
    }
    let dir = root.join("crates/kiln-entity/src/gen");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("mob_blocks.bin"), out)?;
    println!("mob-blocks: wrote {}", dir.join("mob_blocks.bin").display());
    Ok(())
}
