//! Developer tasks.
//!
//! All tasks read and write the work directory: `<workspace>/work` unless `KILN_WORK` points
//! elsewhere. It holds Mojang's jars and generated data and is never committed.

mod block_props;
mod bytecode;
mod codegen;
mod completeness;
mod entities;
mod fetch;
mod http;
mod items;
mod report;
mod zip;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const USAGE: &str = "usage: cargo xtask <task>

tasks:
  fetch [--version <id> | --latest | --snapshot] [--no-spec] [--force]
      download a vanilla server (default: the version kiln-data is pinned to), verify it
      and run its data generator into the work directory
  extract [blocks | items]
      run tools/ExtractBlocks.java (per-state light, collision, hardness) and
      tools/ItemVectors.java (component types, default item components) against the server
      jar into <work>/generated/extra; needed by codegen
  codegen
      generate crates/kiln-data/src/gen and crates/kiln-item/src/gen from the work directory
  completeness
      check crates/kiln-proto/protocol.toml against the generated packet list
  snapshot-report [<version>]
      fetch <version> (default: latest snapshot) into <work>/snapshots/<version> and write
      <work>/reports/<version>.md comparing it with the work directory

environment:
  KILN_WORK   work directory (default: <workspace>/work)";

fn main() -> Result<()> {
    let mut args = Args(std::env::args().skip(1).collect());
    let task = if args.0.is_empty() { String::new() } else { args.0.remove(0) };
    let root = workspace_root();
    let work = work_dir(&root)?;
    match task.as_str() {
        "fetch" => fetch::run(&work, args),
        "extract" => {
            let which = args.positional();
            args.finish()?;
            let jar = fetch::server_jar(&work)?;
            if which.as_deref() != Some("items") {
                block_props::extract(&root, &work, &jar)?;
            }
            if which.as_deref() != Some("blocks") {
                items::extract(&root, &work, &jar)?;
            }
            Ok(())
        }
        "codegen" => {
            args.finish()?;
            codegen::run(&root, &work)
        }
        "completeness" => {
            args.finish()?;
            completeness::run(&root, &work)
        }
        "snapshot-report" => report::run(&root, &work, args),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// `KILN_WORK` (relative to the current directory) or `<workspace>/work`.
fn work_dir(root: &Path) -> Result<PathBuf> {
    match std::env::var_os("KILN_WORK") {
        Some(dir) if !dir.is_empty() => Ok(std::path::absolute(dir)?),
        _ => Ok(root.join("work")),
    }
}

pub(crate) fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

/// Command-line arguments after the task name.
pub(crate) struct Args(Vec<String>);

impl Args {
    pub(crate) fn flag(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|a| a != name);
        self.0.len() != before
    }

    pub(crate) fn value(&mut self, name: &str) -> Result<Option<String>> {
        let Some(i) = self.0.iter().position(|a| a == name) else { return Ok(None) };
        if i + 1 >= self.0.len() {
            bail!("{name} needs a value");
        }
        let value = self.0.remove(i + 1);
        self.0.remove(i);
        Ok(Some(value))
    }

    pub(crate) fn positional(&mut self) -> Option<String> {
        let i = self.0.iter().position(|a| !a.starts_with("--"))?;
        Some(self.0.remove(i))
    }

    pub(crate) fn finish(self) -> Result<()> {
        if !self.0.is_empty() {
            bail!("unexpected arguments: {}\n\n{USAGE}", self.0.join(" "));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Args {
        Args(s.split_whitespace().map(String::from).collect())
    }

    #[test]
    fn parses_flags_values_and_positionals() {
        let mut a = args("--no-spec --version 26.4-snapshot-1 extra");
        assert!(a.flag("--no-spec"));
        assert!(!a.flag("--snapshot"));
        assert_eq!(a.value("--version").unwrap().as_deref(), Some("26.4-snapshot-1"));
        assert_eq!(a.value("--version").unwrap(), None);
        assert_eq!(a.positional().as_deref(), Some("extra"));
        assert!(a.finish().is_ok());

        assert!(args("--version").value("--version").is_err());
        assert!(args("--bogus").finish().is_err());
    }
}
