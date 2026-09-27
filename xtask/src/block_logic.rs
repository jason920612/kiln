//! Block behaviour facts for kiln-blocks, from `tools/ExtractBlockLogic.java` (run by
//! `cargo xtask extract`).
//!
//! `block_logic.bin` (little endian): magic "KBL1", state count u32, signal row count u16;
//! per state 8 bytes: u32 (sturdy FULL/CENTER/RIGID face masks, 6 bits each, then push
//! reaction 3 bits, then wall cover tests 5 bits), flags u8, fluid u8 (kind 2 bits, source,
//! falling, amount 4 bits), signal row u8, reserved u8; then per signal row the weak and the
//! strong signal toward each direction (12 bytes).
//!
//! `block_classes.rs` names each block's Java class, its superclasses and interfaces, and the
//! constructor parameters behaviour needs.

use crate::codegen::HEADER;
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

pub const PUSH_REACTIONS: &[&str] = &["PUSH_PULL", "PUSH", "POPPED", "IMMOVEABLE", "IGNORE_ENTITY"];
const FLAGS: &[&str] = &["signal_source", "analog", "conductor", "solid", "lava_ignites"];

pub fn extract(root: &Path, work: &Path, server_jar: &Path) -> Result<()> {
    let mut cp = vec![server_jar.to_path_buf()];
    crate::block_props::collect_jars(&work.join("libraries"), &mut cp)?;
    let cp = std::env::join_paths(&cp)?;
    let out = work.join("generated/extra");
    std::fs::create_dir_all(&out)?;
    let status = Command::new("java")
        .arg("-cp")
        .arg(cp)
        .arg(root.join("tools/ExtractBlockLogic.java"))
        .arg(&out)
        .current_dir(work)
        .status()
        .context("running java (JDK 25 must be on PATH)")?;
    if !status.success() {
        bail!("ExtractBlockLogic.java failed");
    }
    Ok(())
}

pub fn pack(states: &Value, state_count: usize) -> Result<Vec<u8>> {
    let states = states.as_array().context("block_logic.json")?;
    ensure!(states.len() == state_count, "extractor saw {} states, blocks.json has {state_count}", states.len());
    let mut rows: Vec<[u8; 12]> = vec![[0; 12]];
    let mut row_ids: HashMap<[u8; 12], u8> = HashMap::from([([0; 12], 0)]);
    let mut body = Vec::with_capacity(states.len() * 8);
    for (i, s) in states.iter().enumerate() {
        ensure!(s["id"].as_u64() == Some(i as u64), "state {i} out of order");
        let sturdy = s["sturdy"].as_array().context("sturdy")?;
        let mut word = 0u32;
        for (t, m) in sturdy.iter().enumerate() {
            word |= (m.as_u64().context("sturdy mask")? as u32 & 63) << (6 * t);
        }
        let push = s["push"].as_str().context("push")?;
        let push = PUSH_REACTIONS.iter().position(|p| *p == push).with_context(|| format!("push reaction {push}"))?;
        word |= (push as u32) << 18;
        word |= (s["wall_cover"].as_u64().context("wall_cover")? as u32 & 31) << 21;
        let mut flags = 0u8;
        for (bit, name) in FLAGS.iter().enumerate() {
            if s[*name].as_bool().with_context(|| format!("flag {name}"))? {
                flags |= 1 << bit;
            }
        }
        let kind = match s["fluid"].as_str().context("fluid")? {
            "minecraft:empty" => 0u8,
            "minecraft:water" | "minecraft:flowing_water" => 1,
            "minecraft:lava" | "minecraft:flowing_lava" => 2,
            other => bail!("unknown fluid {other}"),
        };
        let amount = s["amount"].as_u64().context("amount")? as u8;
        ensure!(amount <= 8, "fluid amount {amount}");
        let fluid = kind
            | (s["source"].as_bool().context("source")? as u8) << 2
            | (s["falling"].as_bool().context("falling")? as u8) << 3
            | amount << 4;
        let mut row = [0u8; 12];
        for (j, v) in s["weak"].as_array().context("weak")?.iter().chain(s["strong"].as_array().context("strong")?).enumerate() {
            row[j] = v.as_u64().context("signal")? as u8;
        }
        let next = rows.len();
        let row = *row_ids.entry(row).or_insert_with(|| {
            rows.push(row);
            next as u8
        });
        ensure!(rows.len() <= 256, "more than 256 distinct signal rows");
        body.extend_from_slice(&word.to_le_bytes());
        body.extend_from_slice(&[flags, fluid, row, 0]);
    }
    let mut out = b"KBL1".to_vec();
    out.extend_from_slice(&(states.len() as u32).to_le_bytes());
    out.extend_from_slice(&(rows.len() as u16).to_le_bytes());
    out.extend_from_slice(&body);
    for r in &rows {
        out.extend_from_slice(r);
    }
    Ok(out)
}

/// Parameters kiln-blocks reads, as (Rust field, Java `Class.field` key, kind).
const PARAMS: &[(&str, &str, Kind)] = &[
    ("ticks_to_stay_pressed", "ButtonBlock.ticksToStayPressed", Kind::Int),
    ("arrows_press", "ButtonBlock.type.canButtonBeActivatedByArrows", Kind::Bool),
    ("open_by_hand", "DoorBlock.type.canOpenByHand", Kind::Bool),
    ("open_by_hand", "TrapDoorBlock.type.canOpenByHand", Kind::Bool),
    ("plate_mobs_only", "BasePressurePlateBlock.type.pressurePlateSensitivity", Kind::Mobs),
    ("max_weight", "WeightedPressurePlateBlock.maxWeight", Kind::Int),
    ("base_state", "StairBlock.baseState", Kind::Int),
    ("support_tag", "AttachedStemBlock.supportBlocks", Kind::Str),
    ("support_tag", "StemBlock.stemSupportBlocks", Kind::Str),
    ("support_tag", "NetherFungusBlock.supportBlocks", Kind::Str),
    ("support_tag", "NetherRootsBlock.supportBlocks", Kind::Str),
];

#[derive(Clone, Copy)]
enum Kind {
    Int,
    Bool,
    Mobs,
    Str,
}

pub fn gen_classes(blocks: &Value) -> Result<String> {
    let blocks = blocks.as_array().context("block_classes.json")?;
    let mut classes = BTreeSet::new();
    let mut interfaces = BTreeSet::new();
    for b in blocks {
        for c in b["supers"].as_array().context("supers")? {
            classes.insert(c.as_str().context("class")?.to_string());
        }
        for i in b["interfaces"].as_array().context("interfaces")? {
            interfaces.insert(i.as_str().context("interface")?.to_string());
        }
    }
    ensure!(interfaces.len() <= 32, "more than 32 block interfaces");
    let interfaces: Vec<String> = interfaces.into_iter().collect();

    let mut s = String::from(HEADER);
    s.push_str("//! Java class of each block (in `minecraft:block` registry order), its superclasses and\n");
    s.push_str("//! interfaces, and the constructor parameters block behaviour reads.\n\n");
    s.push_str("/// A vanilla block class (`net.minecraft.world.level.block`).\n");
    s.push_str("#[allow(non_camel_case_types)]\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum BlockClass {\n");
    for c in &classes {
        writeln!(s, "    {c},")?;
    }
    s.push_str("}\n\n/// Interfaces a block class implements, as bits of `BlockClassInfo::interfaces`.\npub mod interface {\n");
    for (bit, i) in interfaces.iter().enumerate() {
        writeln!(s, "    pub const {}: u32 = 1 << {bit};", screaming(i))?;
    }
    s.push_str("}\n\n");
    s.push_str("pub struct BlockClassInfo {\n    /// The block's own class first, then its superclasses down to `Block`.\n");
    s.push_str("    pub classes: &'static [BlockClass],\n    pub interfaces: u32,\n    pub params: BlockParams,\n}\n\n");
    s.push_str("#[derive(Clone, Copy, Debug, Default)]\npub struct BlockParams {\n");
    let mut fields: Vec<(&str, Kind)> = Vec::new();
    for &(f, _, k) in PARAMS {
        if !fields.iter().any(|(g, _)| *g == f) {
            fields.push((f, k));
        }
    }
    for (f, k) in &fields {
        let ty = match k {
            Kind::Int => "i32",
            Kind::Bool | Kind::Mobs => "bool",
            Kind::Str => "Option<&'static str>",
        };
        writeln!(s, "    pub {f}: {ty},")?;
    }
    s.push_str("}\n\nuse BlockClass as C;\n\npub static BLOCK_CLASSES: &[BlockClassInfo] = &[\n");
    for b in blocks {
        let supers: Vec<&str> = b["supers"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        let mut bits = 0u32;
        for i in b["interfaces"].as_array().unwrap() {
            bits |= 1 << interfaces.iter().position(|x| x == i.as_str().unwrap()).unwrap();
        }
        let params = &b["params"];
        let mut p = String::new();
        for (f, _) in &fields {
            for &(g, key, k) in PARAMS {
                if g != *f || params.get(key).is_none() {
                    continue;
                }
                let v = &params[key];
                let text = match k {
                    Kind::Int => v.as_i64().with_context(|| format!("{key}"))?.to_string(),
                    Kind::Bool => v.as_bool().with_context(|| format!("{key}"))?.to_string(),
                    Kind::Mobs => (v.as_str() == Some("MOBS")).to_string(),
                    Kind::Str => format!("Some({:?})", v.as_str().with_context(|| format!("{key}"))?),
                };
                write!(p, "{f}: {text}, ")?;
            }
        }
        let classes: Vec<String> = supers.iter().map(|c| format!("C::{c}")).collect();
        writeln!(
            s,
            "    BlockClassInfo {{ classes: &[{}], interfaces: {bits:#x}, params: BlockParams {{ {p}..DEFAULT }} }}, // {}",
            classes.join(", "),
            b["name"].as_str().context("name")?
        )?;
    }
    s.push_str("];\n\nconst DEFAULT: BlockParams = BlockParams {");
    for (f, k) in &fields {
        let v = match k {
            Kind::Int => "0",
            Kind::Bool | Kind::Mobs => "false",
            Kind::Str => "None",
        };
        write!(s, " {f}: {v},")?;
    }
    s.push_str(" };\n");
    Ok(s)
}

/// `block_items.rs`: each block item's block and, for standing/wall items, the wall block and
/// the direction of the standing block's support.
pub fn gen_block_items(items: &Value) -> Result<String> {
    let mut rows: Vec<(String, String, Option<(String, String)>)> = Vec::new();
    for i in items.as_array().context("block_items.json")? {
        let wall = match (&i["wall"], &i["attach"]) {
            (Value::String(w), Value::String(a)) => Some((w.clone(), a.clone())),
            _ => None,
        };
        rows.push((i["item"].as_str().context("item")?.into(), i["block"].as_str().context("block")?.into(), wall));
    }
    rows.sort();
    let mut s = String::from(HEADER);
    s.push_str("//! Block items (sorted by item id): the block placed, and for standing-and-wall items the\n");
    s.push_str("//! wall block with the direction the standing block attaches toward.\n\n");
    s.push_str("pub static BLOCK_ITEMS: &[(&str, &str, Option<(&str, &str)>)] = &[\n");
    for (item, block, wall) in rows {
        match wall {
            Some((w, a)) => writeln!(s, "    ({item:?}, {block:?}, Some(({w:?}, {a:?}))),")?,
            None => writeln!(s, "    ({item:?}, {block:?}, None),")?,
        }
    }
    s.push_str("];\n");
    Ok(s)
}

fn screaming(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}
