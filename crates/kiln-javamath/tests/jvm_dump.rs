//! Agreement of kiln-javamath's transcendental functions with a real JVM.
//!
//! `tests/jvm_vectors.txt` is a small dump made by `tools/JvmRef.java` on JDK 25 (`java
//! JvmRef.java 400`); `KILN_JVM_DUMP=<file>` points the test at a bigger one (`java JvmRef.java
//! 200000`) and `--nocapture` prints the agreement table.
//!
//! The functions with no HotSpot intrinsic (`atan2`, `asin`, `acos`, `log1p`: `StrictMath`, fdlibm)
//! must agree on every line. `sin`, `cos`, `log` and `pow` are intrinsics (Intel's libm stubs,
//! accurate to under an ulp but not correctly rounded); the correctly rounded results here match
//! them on all but a small fraction of arguments, bounded below. `Mth.SIN` (the table the game
//! actually reads, `float`s of `Math.sin`) must agree on every entry.

use kiln_javamath::{atan, pow, strict, trig};
use std::collections::BTreeMap;

fn run(text: &str) -> BTreeMap<String, (u64, u64)> {
    let mut stats: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    // The dump starts with `Mth.SIN`'s inputs (`i / 10430.378350470453`), before any other function.
    let mut in_table = true;
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let name = it.next().unwrap();
        let v: Vec<f64> = it.map(|t| f64::from_bits(t.parse::<i64>().unwrap() as u64)).collect();
        let (got, want) = match name {
            "sin" => (trig::sin(v[0]), v[1]),
            "cos" => (trig::cos(v[0]), v[1]),
            "atan2" => (atan::atan2(v[0], v[1]), v[2]),
            "asin" => (strict::asin(v[0]), v[1]),
            "acos" => (strict::acos(v[0]), v[1]),
            "log1p" => (strict::log1p(v[0]), v[1]),
            "log" => (pow::log(v[0]), v[1]),
            "pow" => (pow::pow(v[0], v[1]), v[2]),
            other => panic!("unknown function {other}"),
        };
        in_table &= name == "sin";
        if in_table {
            let e = stats.entry("Mth.SIN".to_string()).or_default();
            e.0 += 1;
            e.1 += u64::from((got as f32).to_bits() != (want as f32).to_bits());
        }
        let same = got.to_bits() == want.to_bits() || (got.is_nan() && want.is_nan());
        let e = stats.entry(name.to_string()).or_default();
        e.0 += 1;
        e.1 += u64::from(!same);
        if !same && std::env::var_os("KILN_JVM_VERBOSE").is_some() {
            eprintln!("{name}{v:?}: got {:#x} want {:#x}", got.to_bits(), want.to_bits());
        }
    }
    stats
}

#[test]
fn agrees_with_the_jvm() {
    let path = std::env::var_os("KILN_JVM_DUMP").map(std::path::PathBuf::from);
    let text = match &path {
        Some(p) => std::fs::read_to_string(p).unwrap(),
        None => std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/jvm_vectors.txt")).unwrap(),
    };
    let stats = run(&text);
    eprintln!("{:<8} {:>9} {:>9} {:>9}", "fn", "lines", "differ", "agree %");
    for (name, (n, bad)) in &stats {
        eprintln!("{name:<8} {n:>9} {bad:>9} {:>9.4}", 100.0 * (*n - *bad) as f64 / *n as f64);
    }
    for name in ["atan2", "asin", "acos", "log1p", "Mth.SIN"] {
        let (n, bad) = stats[name];
        assert_eq!(bad, 0, "{name} differs from the JDK in {bad} of {n} lines");
    }
    for (name, floor) in [("sin", 99.0), ("cos", 99.0), ("log", 99.0), ("pow", 99.0)] {
        let (n, bad) = stats[name];
        assert!(100.0 * (n - bad) as f64 / n as f64 >= floor, "{name} agrees with the JDK on only {} of {n} lines", n - bad);
    }
}
