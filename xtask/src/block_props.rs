//! Packs the extractor's per-state facts (tools/ExtractBlocks.java) into a compact binary
//! table that kiln-data embeds with `include_bytes!`.
//!
//! Layout (little endian): magic "KBP2", state count u32, shape count u32; then per state
//! light u8 (emission << 4 | dampening), flags u16, full faces u8, hardness f32, shape u16,
//! block entity type u8 (protocol id in `minecraft:block_entity_type`, 0xff = none),
//! keep-block-entity group u8; then per shape a box count u8 followed by six f32 per box.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

pub const FLAGS: &[&str] = &[
    "sky_down",
    "shape_occludes",
    "can_occlude",
    "solid_render",
    "air",
    "liquid",
    "replaceable",
    "random_ticks",
    "block_entity",
    "correct_tool",
    "full_collision",
];

/// Runs the Java extractor against the server jar and its libraries.
pub fn extract(root: &Path, work: &Path, server_jar: &Path) -> Result<()> {
    let mut cp = vec![server_jar.to_path_buf()];
    collect_jars(&work.join("libraries"), &mut cp)?;
    let cp = std::env::join_paths(&cp)?;
    let out = work.join("generated/extra");
    std::fs::create_dir_all(&out)?;
    for (tool, file) in [("ExtractBlocks.java", "block_states.json"), ("ExtractGameRules.java", "game_rules.json")] {
        let status = Command::new("java")
            .arg("-cp")
            .arg(&cp)
            .arg(root.join("tools").join(tool))
            .arg(out.join(file))
            .current_dir(work)
            .status()
            .context("running java (JDK 25 must be on PATH)")?;
        if !status.success() {
            bail!("{tool} failed");
        }
    }
    Ok(())
}

fn collect_jars(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let p = e?.path();
        if p.is_dir() {
            collect_jars(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "jar") {
            out.push(p);
        }
    }
    Ok(())
}

/// `block_entity_types` maps `minecraft:block_entity_type` entries to their protocol ids.
pub fn pack(json: &Value, state_count: usize, block_entity_types: &HashMap<String, u8>) -> Result<Vec<u8>> {
    let states = json.as_array().context("block_states.json")?;
    if states.len() != state_count {
        bail!("extractor saw {} states, blocks.json has {state_count}", states.len());
    }
    let mut shapes: Vec<Vec<[f32; 6]>> = Vec::new();
    let mut shape_ids: HashMap<String, u16> = HashMap::new();
    let mut body = Vec::with_capacity(states.len() * 12);
    for (i, s) in states.iter().enumerate() {
        if s["id"].as_u64() != Some(i as u64) {
            bail!("state {i} out of order");
        }
        let emission = s["emission"].as_u64().context("emission")? as u8;
        let dampening = s["dampening"].as_u64().context("dampening")? as u8;
        let mut flags = 0u16;
        for (bit, name) in FLAGS.iter().enumerate() {
            if s[*name].as_bool().with_context(|| format!("flag {name}"))? {
                flags |= 1 << bit;
            }
        }
        let faces = s["full_faces"].as_u64().context("full_faces")? as u8;
        let hardness = s["hardness"].as_f64().context("hardness")? as f32;
        let boxes: Vec<[f32; 6]> = s["collision"]
            .as_array()
            .context("collision")?
            .iter()
            .map(|b| {
                let v: Vec<f32> = b.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
                [v[0], v[1], v[2], v[3], v[4], v[5]]
            })
            .collect();
        let block_entity = match &s["block_entity_type"] {
            Value::Null => 0xff,
            Value::String(t) => *block_entity_types.get(t).with_context(|| format!("unknown block entity type {t}"))?,
            _ => bail!("state {i}: block_entity_type"),
        };
        let keep_group = s["keep_block_entity_group"].as_u64().context("keep_block_entity_group")? as u8;
        let key = format!("{boxes:?}");
        let shape = *shape_ids.entry(key).or_insert_with(|| {
            shapes.push(boxes);
            (shapes.len() - 1) as u16
        });
        body.push((emission << 4) | (dampening & 15));
        body.extend_from_slice(&flags.to_le_bytes());
        body.push(faces);
        body.extend_from_slice(&hardness.to_le_bytes());
        body.extend_from_slice(&shape.to_le_bytes());
        body.push(block_entity);
        body.push(keep_group);
    }
    let mut out = b"KBP2".to_vec();
    out.extend_from_slice(&(states.len() as u32).to_le_bytes());
    out.extend_from_slice(&(shapes.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    for boxes in &shapes {
        if boxes.len() > 255 {
            bail!("shape with {} boxes", boxes.len());
        }
        out.push(boxes.len() as u8);
        for b in boxes {
            for v in b {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(out)
}
