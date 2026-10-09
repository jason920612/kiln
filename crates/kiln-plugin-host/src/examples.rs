//! The example plugins in `plugins/examples`, built for `wasm32-wasip2` on demand (tests and
//! tools): `cargo build --release --target wasm32-wasip2` in `plugins/`, then each plugin is
//! laid out as `<dir>/<id>/plugin.toml` + `plugin.wasm`, the layout
//! [`PluginRuntime::load_dir`](crate::PluginRuntime::load_dir) reads.

use crate::Manifest;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Example crate directory names and their component file stems.
pub const EXAMPLES: [(&str, &str); 13] = [
    ("arena", "arena"),
    ("chat-format", "chat_format"),
    ("claims", "claims"),
    ("counter", "counter"),
    ("gatekeeper", "gatekeeper"),
    ("heartbeat", "heartbeat"),
    ("homes", "homes"),
    ("ledger", "ledger"),
    ("noop", "noop"),
    ("petting", "petting"),
    ("scoreboard-hud", "scoreboard_hud"),
    ("shop", "shop"),
    ("spawn-protection", "spawn_protection"),
];

fn plugins_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("plugins")
}

/// Builds the examples once per process; returns the directory holding one plugin directory
/// per example.
pub fn build() -> Result<PathBuf> {
    static BUILT: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    BUILT.get_or_init(|| build_now().map_err(|e| format!("{e:#}"))).clone().map_err(anyhow::Error::msg)
}

fn build_now() -> Result<PathBuf> {
    let root = plugins_root();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args(["build", "--release", "--quiet", "--target", "wasm32-wasip2", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        // The outer build's settings are for the host target.
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .status()
        .context("running cargo for the example plugins (is the wasm32-wasip2 target installed?)")?;
    if !status.success() {
        bail!("building the example plugins failed ({status})");
    }
    let out = root.join("target").join("kiln-plugins");
    for (dir, stem) in EXAMPLES {
        let manifest = std::fs::read_to_string(root.join("examples").join(dir).join("plugin.toml"))?;
        let id = Manifest::parse(&manifest)?.id;
        let dest = out.join(&id);
        std::fs::create_dir_all(&dest)?;
        std::fs::write(dest.join("plugin.toml"), manifest)?;
        let wasm = root.join("target").join("wasm32-wasip2").join("release").join(format!("{stem}.wasm"));
        std::fs::copy(&wasm, dest.join("plugin.wasm")).with_context(|| format!("{}", wasm.display()))?;
    }
    Ok(out)
}

/// One example's manifest (with `extra` TOML appended to its `[config]` table, if any) and
/// component bytes.
pub fn load(id: &str, extra_config: &str) -> Result<(Manifest, Vec<u8>)> {
    let dir = build()?.join(id);
    let mut text = std::fs::read_to_string(dir.join("plugin.toml"))?;
    if !extra_config.is_empty() {
        if !text.contains("[config]") {
            text.push_str("\n[config]\n");
        }
        text.push('\n');
        text.push_str(extra_config);
    }
    Ok((Manifest::parse(&text)?, std::fs::read(dir.join("plugin.wasm"))?))
}

/// A plugin directory named `name` (under the examples' build directory) with the examples
/// `ids` (all when empty), `extra` config appended per id.
pub fn custom_dir(name: &str, ids: &[&str], extra: &[(&str, &str)]) -> Result<PathBuf> {
    let built = build()?;
    let out = built.with_file_name(name);
    let _ = std::fs::remove_dir_all(&out);
    for entry in std::fs::read_dir(&built)? {
        let entry = entry?;
        let id = entry.file_name().to_string_lossy().into_owned();
        // Plugin directories only (an embedder may keep its `.cache` next to them).
        if !entry.path().join("plugin.toml").is_file() || (!ids.is_empty() && !ids.contains(&id.as_str())) {
            continue;
        }
        let add = extra.iter().filter(|(i, _)| *i == id).map(|(_, e)| *e).collect::<Vec<_>>().join("\n");
        let (manifest, wasm) = load(&id, &add)?;
        let dest = out.join(&id);
        std::fs::create_dir_all(&dest)?;
        let mut text = std::fs::read_to_string(built.join(&id).join("plugin.toml"))?;
        if !add.is_empty() {
            if !text.contains("[config]") {
                text.push_str("\n[config]\n");
            }
            text.push('\n');
            text.push_str(&add);
        }
        debug_assert_eq!(Manifest::parse(&text)?.id, manifest.id);
        std::fs::write(dest.join("plugin.toml"), text)?;
        std::fs::write(dest.join("plugin.wasm"), wasm)?;
    }
    Ok(out)
}
