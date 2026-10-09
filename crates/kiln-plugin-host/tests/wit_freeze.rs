//! The WIT 1.0 freeze (`docs/plugin-api.md` §5): the shape of `wit/kiln-api.wit` is hashed
//! without comments and whitespace, so that doc edits are free and any change to a type, a
//! function, an interface or a world fails here until the digest below is changed on purpose.
//!
//! Changing the digest is a decision with consequences:
//! - additions that old guests cannot notice (a new interface, a new function in a new
//!   interface, a new `observed` case behind the manifest's `kinds`, a new capability) are a
//!   minor version: bump `package kiln:api@1.x.0` and this digest;
//! - anything that changes what a compiled 1.0 plugin sees (a record field, a function
//!   signature, an enum order, a meaning) is a major version, with a two-major deprecation
//!   window: the host links both worlds side by side before the old one goes.
//!
//! `async-tasks.wit` rides on WASI 0.3 and is tracked here too but is not promised stable
//! until WASI 0.3 is: its digest may change within 1.x, with a note in the changelog.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// `kiln:api@1.0.0`, `wit/kiln-api.wit`.
const KILN_API_DIGEST: &str = "FILL";
/// `wit/async-tasks.wit` (unstable until WASI 0.3 is).
const ASYNC_TASKS_DIGEST: &str = "FILL";

/// The text without `//` comments (doc comments included) and with whitespace collapsed to
/// single spaces between tokens.
fn shape(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let code = line.split("//").next().unwrap_or("");
        for token in code.split_whitespace() {
            out.push_str(token);
            out.push(' ');
        }
    }
    out
}

fn digest(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wit").join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let hash = Sha256::digest(shape(&text).as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_api_shape_is_frozen() {
    let now = digest("kiln-api.wit");
    assert_eq!(
        now, KILN_API_DIGEST,
        "wit/kiln-api.wit changed shape. If this is on purpose, follow the semver policy in docs/plugin-api.md section 5, \
         bump the package version and set KILN_API_DIGEST to {now}"
    );
}

#[test]
fn the_async_tasks_world_is_tracked() {
    let now = digest("async-tasks.wit");
    assert_eq!(now, ASYNC_TASKS_DIGEST, "wit/async-tasks.wit changed shape: note it in the changelog and set ASYNC_TASKS_DIGEST to {now}");
}

/// The package version in the WIT is the version the host links (and a plugin's manifest
/// `api` names the major).
#[test]
fn the_package_version_matches_the_manifest_major() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../wit/kiln-api.wit");
    let text = std::fs::read_to_string(path).unwrap();
    let line = text.lines().find(|l| l.starts_with("package ")).expect("a package line");
    let version = line.trim_end_matches(';').rsplit('@').next().unwrap();
    let major: u32 = version.split('.').next().unwrap().parse().unwrap();
    assert_eq!(major, kiln_plugin_host::manifest::API_MAJOR, "package {line}");
}
