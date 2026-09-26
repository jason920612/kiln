//! Facts read from the unobfuscated server jar's bytecode with `javap`.

use crate::zip::Zip;
use anyhow::{Context, Result, bail, ensure};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;

pub fn javap(jar: &Path, class: &str) -> Result<String> {
    let out = Command::new("javap")
        .arg("-cp")
        .arg(jar)
        .args(["-c", "-p", class])
        .output()
        .context("running javap (JDK 25 must be on PATH)")?;
    if !out.status.success() {
        bail!("javap {class} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// `javap -c -p` listings of many classes (all of which must exist), in the order given.
pub fn javap_many(jar: &Path, classes: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(classes.len());
    // Batches keep the command line well under Windows' 32 KiB limit.
    for batch in classes.chunks(150) {
        let res = Command::new("javap")
            .arg("-cp")
            .arg(jar)
            .args(["-c", "-p"])
            .args(batch)
            .output()
            .context("running javap (JDK 25 must be on PATH)")?;
        if !res.status.success() {
            bail!("javap failed: {}", String::from_utf8_lossy(&res.stderr));
        }
        let listings = split_listings(&String::from_utf8(res.stdout)?);
        ensure!(
            listings.len() == batch.len(),
            "javap printed {} classes for {} requested",
            listings.len(),
            batch.len()
        );
        for (class, listing) in batch.iter().zip(listings) {
            let header = listing.lines().next().unwrap_or_default();
            ensure!(header.contains(class.as_str()), "javap output out of order at {class}: {header}");
            out.push(listing);
        }
    }
    Ok(out)
}

/// Splits concatenated javap output per class: members are indented, so only class headers
/// and their closing braces start at column 0.
fn split_listings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        if line.starts_with("Compiled from ") {
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
        if line == "}" {
            out.push(std::mem::take(&mut cur));
        }
    }
    out
}

/// Reads `RegistryDataLoader.SYNCHRONIZED_REGISTRIES` and the `Registries` key constants
/// from the jar's bytecode so a new drop's registry list is picked up automatically.
pub fn synchronized_registries(jar: &Path) -> Result<Vec<String>> {
    let loader = javap(jar, "net.minecraft.resources.RegistryDataLoader")?;
    let start = loader.find("static {};").context("static init")?;
    let mut consts = Vec::new();
    let mut found = false;
    for line in loader[start..].lines() {
        if let Some(pos) = line.find("Registries.") {
            let rest = &line[pos + "Registries.".len()..];
            consts.push(rest.split(':').next().unwrap().to_string());
        }
        if line.contains("putstatic") {
            if line.contains("Field SYNCHRONIZED_REGISTRIES:") {
                found = true;
                break;
            }
            consts.clear();
        }
    }
    if !found {
        bail!("SYNCHRONIZED_REGISTRIES not found in bytecode");
    }

    // Registries.<CONST> = createRegistryKey("<path>"): an `ldc` of the path precedes the putstatic.
    let keys = javap(jar, "net.minecraft.core.registries.Registries")?;
    let mut map = HashMap::new();
    let mut last_str: Option<String> = None;
    for line in keys.lines() {
        if let Some(pos) = line.find("// String ") {
            last_str = Some(line[pos + "// String ".len()..].trim().to_string());
        } else if line.contains("putstatic")
            && let (Some(pos), Some(path)) = (line.find("// Field "), last_str.take())
        {
            let field = line[pos + "// Field ".len()..].split(':').next().unwrap().to_string();
            map.insert(field, path);
        }
    }
    consts
        .iter()
        .map(|c| map.get(c).map(|p| format!("minecraft:{p}")).with_context(|| format!("no key path for {c}")))
        .collect()
}

/// `(direction, packet name)` -> packet class, from the `*PacketTypes` holders, whose static
/// initializers call `createClientbound("<name>")` / `createServerbound("<name>")` and whose
/// fields are typed `PacketType<PacketClass>`.
pub fn packet_classes(jar: &Path, zip: &Zip) -> Result<HashMap<(String, String), String>> {
    let holders: Vec<String> = zip
        .names()
        .filter(|n| n.starts_with("net/minecraft/network/protocol/") && n.ends_with("PacketTypes.class"))
        .map(class_name)
        .collect();
    ensure!(!holders.is_empty(), "no *PacketTypes classes in {}", jar.display());
    let mut out = HashMap::new();
    for listing in javap_many(jar, &holders)? {
        out.extend(parse_packet_types(&listing));
    }
    Ok(out)
}

fn parse_packet_types(listing: &str) -> Vec<((String, String), String)> {
    const TYPE: &str = "net.minecraft.network.protocol.PacketType<";
    let mut field_class = HashMap::new();
    for line in listing.lines() {
        if let Some(rest) = line.split_once(TYPE).map(|(_, r)| r)
            && let Some((class, field)) = rest.split_once("> ")
            && let Some(field) = field.strip_suffix(';')
        {
            field_class.insert(field.to_string(), class.to_string());
        }
    }
    let mut out = Vec::new();
    let (mut name, mut dir) = (None, None);
    for line in listing.lines() {
        if let Some(pos) = line.find("// String ") {
            name = Some(line[pos + "// String ".len()..].trim().to_string());
        } else if line.contains("Method createClientbound:") {
            dir = Some("clientbound");
        } else if line.contains("Method createServerbound:") {
            dir = Some("serverbound");
        } else if line.contains("putstatic") {
            let field = line.split_once("// Field ").and_then(|(_, f)| f.split(':').next());
            if let (Some(n), Some(d), Some(class)) = (name.take(), dir.take(), field.and_then(|f| field_class.get(f))) {
                out.push(((d.to_string(), n), class.clone()));
            }
        }
    }
    out
}

fn class_name(entry: &str) -> String {
    entry.trim_end_matches(".class").replace('/', ".")
}

/// Normalized bytecode of each class together with its nested classes, for spotting changes
/// between versions. Classes missing from the jar are left out.
pub fn fingerprints(jar: &Path, zip: &Zip, classes: &[String]) -> Result<BTreeMap<String, Vec<String>>> {
    let entries: Vec<String> = zip.names().filter(|n| n.ends_with(".class")).map(class_name).collect();
    let mut owner = Vec::new();
    let mut listed = Vec::new();
    for class in classes {
        let nested = format!("{class}$");
        for e in &entries {
            if e == class || e.starts_with(&nested) {
                owner.push(class.clone());
                listed.push(e.clone());
            }
        }
    }
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (class, listing) in owner.into_iter().zip(javap_many(jar, &listed)?) {
        out.entry(class).or_default().extend(normalize(&listing));
    }
    Ok(out)
}

/// Drops what changes without a behavior change: constant pool indices and bytecode offsets.
fn normalize(listing: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in listing.lines() {
        let mut line = line.trim();
        if let Some((offset, rest)) = line.split_once(": ")
            && !offset.is_empty()
            && offset.bytes().all(|b| b.is_ascii_digit())
        {
            line = rest;
        }
        let mut s = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '#' {
                while chars.next_if(char::is_ascii_digit).is_some() {}
                s.push('#');
            } else if c.is_whitespace() {
                while chars.next_if(|c| c.is_whitespace()).is_some() {}
                s.push(' ');
            } else {
                s.push(c);
            }
        }
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLDER: &str = r#"public class net.minecraft.network.protocol.ping.PingPacketTypes {
  public static final net.minecraft.network.protocol.PacketType<net.minecraft.network.protocol.ping.ClientboundPongResponsePacket> CLIENTBOUND_PONG_RESPONSE;
  public static final net.minecraft.network.protocol.PacketType<net.minecraft.network.protocol.ping.ServerboundPingRequestPacket> SERVERBOUND_PING_REQUEST;
  static {};
    Code:
       0: ldc           #7                  // String pong_response
       2: invokestatic  #9                  // Method createClientbound:(Ljava/lang/String;)Lnet/minecraft/network/protocol/PacketType;
       5: putstatic     #15                 // Field CLIENTBOUND_PONG_RESPONSE:Lnet/minecraft/network/protocol/PacketType;
       8: ldc           #19                 // String ping_request
      10: invokestatic  #21                 // Method createServerbound:(Ljava/lang/String;)Lnet/minecraft/network/protocol/PacketType;
      13: putstatic     #24                 // Field SERVERBOUND_PING_REQUEST:Lnet/minecraft/network/protocol/PacketType;
      16: return
}
"#;

    #[test]
    fn maps_packet_names_to_classes() {
        let mut got = parse_packet_types(HOLDER);
        got.sort();
        let key = |d: &str, n: &str| (d.to_string(), n.to_string());
        assert_eq!(
            got,
            [
                (
                    key("clientbound", "pong_response"),
                    "net.minecraft.network.protocol.ping.ClientboundPongResponsePacket".into()
                ),
                (
                    key("serverbound", "ping_request"),
                    "net.minecraft.network.protocol.ping.ServerboundPingRequestPacket".into()
                ),
            ]
        );
    }

    #[test]
    fn splits_concatenated_listings() {
        let text = format!("Compiled from \"A.java\"\n{HOLDER}Compiled from \"B.java\"\nclass B {{\n  B();\n}}\n");
        let parts = split_listings(&text);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("public class net.minecraft.network.protocol.ping.PingPacketTypes {"));
        assert_eq!(parts[1], "class B {\n  B();\n}\n");
    }

    #[test]
    fn normalization_ignores_constant_pool_and_offsets() {
        let a = "  12: invokestatic  #7                  // Method foo:()V\n  15: invokeinterface #22,  1  // X";
        let b = "  14: invokestatic  #9 // Method foo:()V\n  17: invokeinterface #30, 1 // X";
        assert_eq!(normalize(a), normalize(b));
        assert_eq!(normalize(a)[0], "invokestatic # // Method foo:()V");
        assert_ne!(normalize(a), normalize("  12: invokestatic  #7  // Method bar:()V"));
    }
}
