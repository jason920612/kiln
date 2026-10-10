//! The WIT 1.0 freeze (`docs/plugin-api.md` §5): the shape of `wit/kiln-api.wit` is hashed
//! without comments and whitespace, so that doc edits are free and any change to a type, a
//! function, an interface or a world fails here until the digest below is changed on purpose.
//!
//! Changing the digest is a decision with consequences:
//! - additions that old guests cannot notice (a new interface, a new function in a new
//!   interface, a new record, a new capability) are a minor version: bump
//!   `package kiln:api@1.x.0` and this digest. A new case of an existing `variant` or a new
//!   field of an existing `record` is *not* one: the type is part of the signature of every
//!   function that takes it, and the host's check of a guest's export against its own type
//!   would refuse the old plugin's region instance (1.1 first did this with `observed`: see
//!   `the_1_0_declarations_are_all_still_there` and `api_compat.rs`);
//! - anything that changes what a compiled 1.0 plugin sees (a record field, a function
//!   signature, an enum order, a meaning) is a major version, with a two-major deprecation
//!   window: the host links both worlds side by side before the old one goes.
//!
//! `async-tasks.wit` rides on WASI 0.3 and is tracked here too but is not promised stable
//! until WASI 0.3 is: its digest may change within 1.x, with a note in the changelog.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// `kiln:api@1.1.0`, `wit/kiln-api.wit`. (1.0.0 was `7b3032b84e8a93cd0ddff0b9662b262f0d5c3328f08806565f3adeafa72667df`; 1.1 only adds:
/// the `world-read` and `move-hooks` interfaces, the `move-event` record, and the imports and exports of the worlds for them.)
const KILN_API_DIGEST: &str = "260ea48cc9d0088e048144021edf597ce2b5540a61294a09b41e5a6a76254a01";
/// The package version that digest belongs to: the two change together.
const KILN_API_VERSION: &str = "1.1.0";
/// `wit/async-tasks.wit` (unstable until WASI 0.3 is).
const ASYNC_TASKS_DIGEST: &str = "950be11fa751bdac4510b7fb2c16ed6d9c642fd8c1d0ea93e8ee5fcb84249c0f";

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
    assert_eq!(version, KILN_API_VERSION, "the digest and the package version change together ({line})");
}

/// The declarations of every top-level block (`interface`, `world`) of a WIT file by block
/// name: a declaration ends at a `;` or at the `}` that closes a nested block, at depth one.
fn declarations(text: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let tokens = shape(text).replace('{', " { ").replace('}', " } ").replace(';', " ; ");
    let mut out: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let (mut depth, mut name, mut current) = (0, String::new(), Vec::<&str>::new());
    let mut head: Vec<&str> = Vec::new();
    for t in tokens.split_whitespace() {
        match t {
            "{" => {
                depth += 1;
                if depth == 1 {
                    name = head.join(" ");
                    head.clear();
                    out.entry(name.clone()).or_default();
                } else {
                    current.push(t);
                }
            }
            "}" => {
                depth -= 1;
                if depth == 0 {
                    continue;
                }
                current.push(t);
                if depth == 1 {
                    out.get_mut(&name).unwrap().push(current.join(" "));
                    current.clear();
                }
            }
            ";" if depth == 1 => {
                out.get_mut(&name).unwrap().push(current.join(" "));
                current.clear();
            }
            _ if depth == 0 => head.push(t),
            _ => current.push(t),
        }
    }
    out
}

/// What makes a minor version compatible: a plugin built against 1.0 (`tests/fixtures/compat10`
/// keeps the file) must still meet the types it was built for. Every declaration of the 1.0
/// interfaces and worlds is still there, word for word; 1.1 may add declarations, interfaces and
/// world items. (A new case in a `variant`, a new field in a `record` or a new parameter is a
/// changed declaration: the host checks the guest's exported function types against its own, so
/// it would refuse the old plugin's region instance, which `api_compat.rs` also shows.)
#[test]
fn the_1_0_declarations_are_all_still_there() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let old = declarations(&std::fs::read_to_string(root.join("tests/fixtures/compat10/wit/kiln-api.wit")).unwrap());
    let new = declarations(&std::fs::read_to_string(root.join("../../wit/kiln-api.wit")).unwrap());
    assert!(old.len() > 10, "{:?}", old.keys().collect::<Vec<_>>());
    for (block, decls) in &old {
        let now = new.get(block).unwrap_or_else(|| panic!("`{block}` of 1.0 is gone"));
        for d in decls {
            assert!(now.contains(d), "in `{block}`: the 1.0 declaration `{d}` changed or went away");
        }
    }
}
