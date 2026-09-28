//! Compiled components cached as `.cwasm` files (design §11.1): the key covers the component
//! bytes and the engine's compatibility hash (wasmtime version, target, the settings that
//! change code, such as fuel versus epoch interruption), so a stale file is never used.

use anyhow::Result;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tracing::warn;
use wasmtime::Engine;
use wasmtime::component::Component;

fn key(engine: &Engine, wasm: &[u8]) -> String {
    // Two differently salted SipHash passes: 128 bits of key.
    let half = |salt: u64| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        salt.hash(&mut h);
        wasm.hash(&mut h);
        engine.precompile_compatibility_hash().hash(&mut h);
        h.finish()
    };
    format!("{:016x}{:016x}", half(0x6b69_6c6e), half(0x63_7761_736d))
}

/// The component for `wasm`, from the cache when possible; whether it came from the cache.
pub(crate) fn component(engine: &Engine, wasm: &[u8], dir: Option<&Path>) -> Result<(Component, bool)> {
    let Some(dir) = dir else { return Ok((Component::new(engine, wasm)?, false)) };
    let path = dir.join(format!("{}.cwasm", key(engine, wasm)));
    if path.is_file() {
        // SAFETY: the file is one this cache wrote (serialized by the same engine
        // configuration, as the key says) into a directory only the server writes.
        match unsafe { Component::deserialize_file(engine, &path) } {
            Ok(c) => return Ok((c, true)),
            Err(e) => warn!("plugin cache {}: {e:#}; compiling again", path.display()),
        }
    }
    let c = Component::new(engine, wasm)?;
    match c.serialize() {
        Ok(bytes) => {
            let tmp = path.with_extension("cwasm.tmp");
            let r = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&tmp, &bytes)).and_then(|()| std::fs::rename(&tmp, &path));
            if let Err(e) = r {
                warn!("cannot write plugin cache {}: {e}", path.display());
            }
        }
        Err(e) => warn!("cannot serialize a compiled plugin: {e:#}"),
    }
    Ok((c, false))
}
