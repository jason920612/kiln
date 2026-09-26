//! The clientbound encoders must keep producing the bytes vanilla's codecs accepted: every
//! vector in `vectors/` is compared with `testdata/clientbound.txt` (packet id + body, hex),
//! which `tools/packet_vectors.py --bless` writes after vanilla decoded and re-encoded each one
//! to identical bytes.

mod vectors;

use std::collections::HashMap;

#[test]
fn encoders_match_vanilla_checked_bytes() {
    let golden: HashMap<&str, &str> = include_str!("testdata/clientbound.txt")
        .lines()
        .filter_map(|l| l.split_once(' '))
        .collect();
    let cases = vectors::cases().0;
    let mut failures = Vec::new();
    for case in &cases {
        let got: String = case.packet.iter().map(|b| format!("{b:02x}")).collect();
        match golden.get(case.name.as_str()) {
            None => failures.push(format!("{}: not in testdata/clientbound.txt (run tools/packet_vectors.py --bless)", case.name)),
            Some(want) if *want != got => failures.push(format!("{}:\n  got  {got}\n  want {want}", case.name)),
            Some(_) => {}
        }
    }
    assert!(failures.is_empty(), "{} of {} vectors differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
    assert_eq!(golden.len(), cases.len(), "testdata/clientbound.txt has vectors the tests no longer build");
}
