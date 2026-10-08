//! Simulation and world generation must not call the platform libm.
//!
//! `f64::sin`, `powf`, `ln` and the rest go to MSVC's or glibc's C library, which round their last
//! bit differently from Java and from each other: the same seed then gives different worlds and
//! mob decisions per operating system (a goat's long jump was one ulp off vanilla on Linux). Use
//! `kiln_javamath::{trig, atan, strict, pow, mth}` instead, which reproduce `java.lang.Math`.
//!
//! The test scans the non-test code of every crate's `src` for the std methods and fails on any
//! call that is not on the allow list below. Code after the first `#[cfg(test)]` of a file,
//! `tests/`, `examples/` and `benches/` are not scanned.

use std::path::{Path, PathBuf};

/// The std float methods that reach libm. (`sqrt`, `floor`, `round`, `abs`, `mul_add`,
/// `to_radians` and friends are exact IEEE operations and fine.)
const LIBM: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "exp", "exp2", "exp_m1", "ln", "ln_1p",
    "log", "log2", "log10", "powf", "powi", "sin_cos", "cbrt", "hypot",
];

/// Files (path suffixes) allowed to call libm, and why.
const ALLOWED_FILES: &[(&str, &str)] = &[
    // Beyond 1e9 radians the double-double reduction runs out of bits and `sin`/`cos` fall back to
    // the platform libm; no game code passes such an angle (they stay within a few thousand).
    ("kiln-javamath/src/trig.rs", "huge-argument fallback"),
    // Load-generating bot clients choose where to walk; nothing in the server depends on it.
    ("kiln-bot/", "client-side bot behaviour, not simulation"),
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The libm calls on a line of code: `.name(` (but not `.powi(2)`, which is `x * x`) and
/// `f32::name(` / `f64::name(`.
fn calls(line: &str) -> Vec<String> {
    let code = line.split("//").next().unwrap_or("");
    let mut found = Vec::new();
    for name in LIBM {
        let method = format!(".{name}(");
        let mut from = 0;
        while let Some(i) = code[from..].find(&method) {
            let at = from + i;
            from = at + method.len();
            if *name == "powi" && code[from..].trim_start().starts_with("2)") {
                continue;
            }
            found.push(format!(".{name}("));
        }
        for ty in ["f32", "f64"] {
            if code.contains(&format!("{ty}::{name}(")) {
                found.push(format!("{ty}::{name}("));
            }
        }
    }
    found
}

#[test]
fn no_platform_libm_in_server_code() {
    let mut files = Vec::new();
    for e in std::fs::read_dir(crates_dir()).unwrap() {
        let src = e.unwrap().path().join("src");
        if src.is_dir() {
            rust_files(&src, &mut files);
        }
    }
    assert!(files.len() > 100, "found only {} source files; is the layout still crates/*/src?", files.len());
    let mut bad = Vec::new();
    for f in files {
        let path = f.to_string_lossy().replace('\\', "/");
        if ALLOWED_FILES.iter().any(|(suffix, _)| path.contains(suffix)) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in text.lines().enumerate() {
            if line.trim() == "#[cfg(test)]" {
                break;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            for call in calls(line) {
                bad.push(format!("{path}:{}: {call}", n + 1));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "std libm calls in server code (use kiln_javamath::{{trig, atan, strict, pow, mth}}, or add the file to ALLOWED_FILES with a reason):\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_scanner_sees_what_it_should() {
    assert_eq!(calls("let a = x.sin() + y.powf(2.0);"), vec![".sin(", ".powf("]);
    assert!(calls("let a = (x - y).powi(2);").is_empty());
    assert_eq!(calls("let a = (x - y).powi(3);"), vec![".powi("]);
    assert_eq!(calls("f64::hypot(a, b)"), vec!["f64::hypot("]);
    assert!(calls("x.sqrt() // then .sin( in a comment").is_empty());
    assert!(calls("kiln_javamath::trig::sin(x)").is_empty());
}
