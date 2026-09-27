//! Packs tools/ExtractEntityPhysics.java's output into the binary table kiln-entity embeds.
//!
//! Layout (little endian): magic "KEP2", state count u32, shape count u32, block count u32;
//! per shape: coordinate counts u8 ×3 (x, y, z), the coordinates as f64, then the full-cell
//! bits (x-major, then y, then z; LSB first), padded to a byte;
//! per state: collision shape u16, entity-inside shape u16 (0xffff = the full block),
//! fluid u8 (0 none, 1 flowing water, 2 water, 3 flowing lava, 4 lava), amount u8,
//! sturdy faces u8 (bit = 3D data value), flags u16 (see `STATE_FLAGS`);
//! per block (registry order): friction, speed factor, jump factor, bounce restitution and
//! fall distance reduction and explosion resistance as f32; then named shapes (context-dependent blocks' shapes): count u8,
//! per entry a name (length u8, UTF-8) and a shape u16; then the offset blocks with collision
//! (whose collision shape is stored unshifted): count u16, per entry state u16 and maximum
//! horizontal offset f32.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

/// Bit order of the per-state flags.
pub const STATE_FLAGS: &[&str] =
    &["falling", "source", "air", "liquid", "solid", "replaceable", "offset", "suffocating", "large", "cube"];

const FLUIDS: &[&str] =
    &["minecraft:empty", "minecraft:flowing_water", "minecraft:water", "minecraft:flowing_lava", "minecraft:lava"];

pub fn pack(json: &Value, state_count: usize) -> Result<Vec<u8>> {
    let shapes = json["shapes"].as_array().context("shapes")?;
    let states = json["states"].as_array().context("states")?;
    let blocks = json["blocks"].as_array().context("blocks")?;
    if states.len() != state_count {
        bail!("extractor saw {} states, blocks.json has {state_count}", states.len());
    }
    let mut out = b"KEP2".to_vec();
    for n in [states.len(), shapes.len(), blocks.len()] {
        out.extend_from_slice(&(n as u32).to_le_bytes());
    }
    for s in shapes {
        let axes: Vec<&Vec<Value>> =
            ["x", "y", "z"].iter().map(|a| s[*a].as_array().context("coords")).collect::<Result<_>>()?;
        for coords in &axes {
            out.push(u8::try_from(coords.len()).context("too many coordinates")?);
        }
        for coords in &axes {
            for c in *coords {
                out.extend_from_slice(&c.as_f64().context("coord")?.to_le_bytes());
            }
        }
        let bits = s["full"].as_str().context("full")?.as_bytes();
        let mut packed = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            if b == b'1' {
                packed[i / 8] |= 1 << (i % 8);
            }
        }
        out.extend_from_slice(&packed);
    }
    for (i, s) in states.iter().enumerate() {
        if s["id"].as_u64() != Some(i as u64) {
            bail!("state {i} out of order");
        }
        let collision = u16::try_from(s["collision"].as_u64().context("collision")?)?;
        let inside = match s["inside"].as_i64().context("inside")? {
            -1 => 0xffff,
            v => u16::try_from(v)?,
        };
        let fluid = s["fluid"].as_str().context("fluid")?;
        let fluid = FLUIDS.iter().position(|f| *f == fluid).with_context(|| format!("unknown fluid {fluid}"))? as u8;
        let mut flags = 0u16;
        for (bit, name) in STATE_FLAGS.iter().enumerate() {
            if s[*name].as_bool().with_context(|| format!("state {i}: {name}"))? {
                flags |= 1 << bit;
            }
        }
        out.extend_from_slice(&collision.to_le_bytes());
        out.extend_from_slice(&inside.to_le_bytes());
        out.push(fluid);
        out.push(s["amount"].as_u64().context("amount")? as u8);
        out.push(s["sturdy"].as_u64().context("sturdy")? as u8);
        out.extend_from_slice(&flags.to_le_bytes());
    }
    for b in blocks {
        for key in ["friction", "speed", "jump", "bounce", "fall_reduction", "resistance"] {
            // Printed with Float.toString: parsing as f32 recovers the exact value.
            let v: f32 = b[key].to_string().parse().with_context(|| format!("{key} of {}", b["name"]))?;
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    let named = json["named"].as_object().context("named")?;
    out.push(u8::try_from(named.len())?);
    for (name, id) in named {
        out.push(u8::try_from(name.len())?);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&u16::try_from(id.as_u64().context("named shape")?)?.to_le_bytes());
    }
    let offsets: Vec<(usize, f32)> = states
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let v: f32 = s["max_offset"].to_string().parse().ok()?;
            (v != 0.0).then_some((i, v))
        })
        .collect();
    out.extend_from_slice(&u16::try_from(offsets.len())?.to_le_bytes());
    for (i, v) in offsets {
        out.extend_from_slice(&u16::try_from(i)?.to_le_bytes());
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(out)
}

/// Writes `crates/kiln-entity/src/gen/physics.bin` from `input`.
pub fn write(root: &Path, input: &Path, state_count: usize) -> Result<()> {
    let json = crate::read_json(input).context("run `cargo xtask extract` first")?;
    let out = root.join("crates/kiln-entity/src/gen");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("physics.bin"), pack(&json, state_count)?)?;
    println!("codegen: wrote {}", out.join("physics.bin").display());
    Ok(())
}
