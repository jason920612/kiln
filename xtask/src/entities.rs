//! Entity codegen from the unobfuscated jar (`javap`): entity types with dimensions and
//! tracking, data serializer ids, poses and per-class synched data fields.

use crate::bytecode::javap;
use crate::codegen::{Input, const_name};
use crate::read_json;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

const ENTITY_CLASS: &str = "net.minecraft.world.entity.Entity";

/// One `javap -c` instruction line: opcode, first operand and the constant-pool comment.
struct Insn<'a> {
    op: &'a str,
    arg: &'a str,
    comment: &'a str,
}

fn parse_insn(line: &str) -> Option<Insn<'_>> {
    let (offset, rest) = line.trim_start().split_once(": ")?;
    if offset.is_empty() || !offset.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (code, comment) = rest.split_once("// ").map_or((rest, ""), |(c, m)| (c, m.trim()));
    let mut parts = code.split_whitespace();
    Some(Insn { op: parts.next()?, arg: parts.next().unwrap_or(""), comment })
}

#[derive(Clone, Copy)]
enum Num {
    Int(i64),
    Float(f32),
}

/// Numeric constant pushed by `insn`, if it is a constant load.
fn constant(insn: &Insn) -> Option<Num> {
    let op = insn.op;
    if op == "iconst_m1" {
        return Some(Num::Int(-1));
    }
    if let Some(v) = op.strip_prefix("iconst_") {
        return v.parse().ok().map(Num::Int);
    }
    if let Some(v) = op.strip_prefix("fconst_") {
        return v.parse().ok().map(Num::Float);
    }
    match op {
        "bipush" | "sipush" => insn.arg.parse().ok().map(Num::Int),
        "ldc" | "ldc_w" => {
            if let Some(v) = insn.comment.strip_prefix("int ") {
                v.parse().ok().map(Num::Int)
            } else if let Some(v) = insn.comment.strip_prefix("float ") {
                v.trim_end_matches('f').parse().ok().map(Num::Float)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The `static {}` method body of one class in javap output.
fn static_init(class_text: &str) -> Result<&str> {
    let start = class_text.find("  static {};").context("no static initializer")?;
    let body = &class_text[start..];
    Ok(&body[..body.find("\n\n").unwrap_or(body.len())])
}

/// `javap -c -p` for several classes in one JVM start.
fn javap_classes(jar: &Path, classes: &[String]) -> Result<String> {
    let out = Command::new("javap")
        .arg("-cp")
        .arg(jar)
        .args(["-c", "-p"])
        .args(classes)
        .output()
        .context("running javap (JDK 25 must be on PATH)")?;
    if !out.status.success() {
        bail!("javap failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// One `EntityType.Builder` chain, starting from the builder's defaults.
struct TypeSpec {
    key: String,
    class: String,
    width: f32,
    height: f32,
    eye_height: f32,
    tracking_range: i64,
    update_interval: i64,
    track_deltas: bool,
}

impl TypeSpec {
    fn new() -> Self {
        // EntityType.Builder.<init>: EntityDimensions.scalable(0.6f, 1.8f), range 5, interval 3;
        // EntityDimensions defaults the eye height to height * 0.85f.
        let height = 1.8f32;
        Self {
            key: String::new(),
            class: String::new(),
            width: 0.6,
            height,
            eye_height: height * 0.85,
            tracking_range: 5,
            update_interval: 3,
            track_deltas: true,
        }
    }
}

fn last_floats<const N: usize>(nums: &[Num], call: &str) -> Result<[f32; N]> {
    let tail = nums.len().checked_sub(N).map(|s| &nums[s..]).with_context(|| format!("{call}: missing arguments"))?;
    let mut out = [0.0; N];
    for (o, n) in out.iter_mut().zip(tail) {
        let Num::Float(f) = n else { bail!("{call}: non-float argument") };
        *o = *f;
    }
    Ok(out)
}

fn last_int(nums: &[Num], call: &str) -> Result<i64> {
    match nums.last() {
        Some(Num::Int(v)) => Ok(*v),
        _ => bail!("{call}: missing int argument"),
    }
}

/// Entity types from the `EntityTypes` static initializer: builder calls per registration,
/// the registry key from `EntityTypeIds` and the entity class from the field's type argument.
fn entity_types(jar: &Path) -> Result<Vec<TypeSpec>> {
    let ids = javap(jar, "net.minecraft.world.entity.EntityTypeIds")?;
    let mut keys = HashMap::new();
    let mut last_str = None;
    for insn in static_init(&ids)?.lines().filter_map(parse_insn) {
        if let Some(s) = insn.comment.strip_prefix("String ") {
            last_str = Some(s.to_string());
        } else if insn.op == "putstatic" {
            let field = insn.comment.trim_start_matches("Field ").split(':').next().unwrap();
            keys.insert(field.to_string(), format!("minecraft:{}", last_str.take().context("key string")?));
        }
    }

    let types = javap(jar, "net.minecraft.world.entity.EntityTypes")?;
    let mut classes = HashMap::new();
    for line in types.lines() {
        if let Some(rest) = line.trim().strip_prefix("public static final net.minecraft.world.entity.EntityType<")
            && let Some((class, field)) = rest.rsplit_once("> ")
        {
            classes.insert(field.trim_end_matches(';').to_string(), class.to_string());
        }
    }

    let mut out = Vec::new();
    let mut spec: Option<TypeSpec> = None;
    let mut key_const: Option<String> = None;
    let mut nums: Vec<Num> = Vec::new();
    for insn in static_init(&types)?.lines().filter_map(parse_insn) {
        if let Some(n) = constant(&insn) {
            nums.push(n);
            continue;
        }
        let c = insn.comment;
        match insn.op {
            "getstatic" => {
                if let Some(f) = c.strip_prefix("Field net/minecraft/world/entity/EntityTypeIds.") {
                    key_const = Some(f.split(':').next().unwrap().to_string());
                }
            }
            "invokestatic"
                if c.contains("EntityType$Builder.of:") || c.contains("EntityType$Builder.createNothing:") =>
            {
                spec = Some(TypeSpec::new());
                nums.clear();
            }
            "invokevirtual" if c.contains("EntityType$Builder.") => {
                let method = c.split("EntityType$Builder.").nth(1).unwrap().split(':').next().unwrap();
                let s = spec.as_mut().with_context(|| format!("builder call {method} outside a chain"))?;
                match method {
                    "sized" => {
                        let [w, h] = last_floats::<2>(&nums, method)?;
                        (s.width, s.height, s.eye_height) = (w, h, h * 0.85);
                    }
                    "eyeHeight" => s.eye_height = last_floats::<1>(&nums, method)?[0],
                    "clientTrackingRange" => s.tracking_range = last_int(&nums, method)?,
                    "updateInterval" => s.update_interval = last_int(&nums, method)?,
                    "noUpdateInterval" => s.update_interval = i32::MAX as i64,
                    "dontTrackDeltas" => s.track_deltas = false,
                    _ => {}
                }
                nums.clear();
            }
            "invokestatic" if c.starts_with("Method register:(Lnet/minecraft/resources/ResourceKey;") => {
                let mut s = spec.take().context("register without a builder")?;
                let k = key_const.take().context("register without a key")?;
                s.key = keys.get(&k).with_context(|| format!("no EntityTypeIds.{k}"))?.clone();
                s.class = classes.get(&k).with_context(|| format!("no EntityTypes.{k} field"))?.clone();
                out.push(s);
            }
            _ => {}
        }
    }
    Ok(out)
}

struct EntityClassInfo {
    name: String,
    parent: Option<String>,
    /// (Java field name, serializer constant name) in definition order.
    fields: Vec<(String, String)>,
}

fn parse_entity_class(block: &str) -> Result<EntityClassInfo> {
    let header = block
        .lines()
        .find(|l| !l.starts_with(' ') && l.contains("class ") && l.ends_with('{'))
        .context("class header")?;
    let decl = header.split_once("class ").unwrap().1;
    let name = decl.split([' ', '<']).next().unwrap().to_string();
    let mut rest = &decl[name.len()..];
    if rest.starts_with('<') {
        // Skip the class's own type parameters, whose bounds may say "extends".
        let mut depth = 0;
        let (end, _) = rest
            .char_indices()
            .find(|&(_, ch)| {
                depth += match ch {
                    '<' => 1,
                    '>' => -1,
                    _ => 0,
                };
                depth == 0
            })
            .context("unbalanced type parameters")?;
        rest = &rest[end + 1..];
    }
    let parent = rest.split_once(" extends ").map(|(_, r)| r.split([' ', '<']).next().unwrap().to_string());

    let mut fields = Vec::new();
    if block.contains("SynchedEntityData.defineId") {
        let (mut class_arg, mut serializer, mut pending) = (None, None, None);
        for insn in static_init(block)?.lines().filter_map(parse_insn) {
            let c = insn.comment;
            match insn.op {
                "ldc" | "ldc_w" => {
                    if let Some(cls) = c.strip_prefix("class ") {
                        class_arg = Some(cls.replace('/', "."));
                    }
                }
                "getstatic" => {
                    if let Some(f) = c.strip_prefix("Field net/minecraft/network/syncher/EntityDataSerializers.") {
                        serializer = Some(f.split(':').next().unwrap().to_string());
                    }
                }
                "invokestatic" if c.contains("SynchedEntityData.defineId:") => {
                    let owner = class_arg.take().context("defineId without a class argument")?;
                    if owner != name {
                        bail!("{name} defines data for {owner}");
                    }
                    pending = Some(serializer.take().context("defineId without a serializer")?);
                }
                "putstatic" => {
                    if let Some(ser) = pending.take() {
                        let field = c.trim_start_matches("Field ");
                        if !field.ends_with(":Lnet/minecraft/network/syncher/EntityDataAccessor;") {
                            bail!("{name}: defineId result stored in {field}");
                        }
                        fields.push((field.split(':').next().unwrap().to_string(), ser));
                    }
                }
                _ => {}
            }
        }
        if pending.is_some() {
            bail!("{name}: defineId result not stored");
        }
    }
    Ok(EntityClassInfo { name, parent, fields })
}

/// Entity classes from `roots` up to `Entity`, with the data fields each class defines.
/// Class initialization runs superclasses first, so a class's indices follow its parent's.
fn entity_classes(jar: &Path, roots: impl IntoIterator<Item = String>) -> Result<BTreeMap<String, EntityClassInfo>> {
    let mut classes = BTreeMap::new();
    let mut todo: BTreeSet<String> = roots.into_iter().collect();
    while !todo.is_empty() {
        let batch: Vec<String> = std::mem::take(&mut todo).into_iter().collect();
        let text = javap_classes(jar, &batch)?;
        for block in text.split("Compiled from ").skip(1) {
            let info = parse_entity_class(block)?;
            if info.name != ENTITY_CLASS {
                let parent = info.parent.as_deref().with_context(|| format!("{} is not an entity", info.name))?;
                if !classes.contains_key(parent) && !batch.iter().any(|b| b == parent) {
                    todo.insert(parent.to_string());
                }
            }
            classes.insert(info.name.clone(), info);
        }
        if let Some(missing) = batch.iter().find(|b| !classes.contains_key(*b)) {
            bail!("javap printed no class {missing}");
        }
    }
    Ok(classes)
}

/// `EntityDataSerializers` registration order, which is the serializer's network id.
fn entity_data_serializers(jar: &Path) -> Result<Vec<String>> {
    let text = javap(jar, "net.minecraft.network.syncher.EntityDataSerializers")?;
    let mut out = Vec::new();
    let mut last = None;
    for insn in static_init(&text)?.lines().filter_map(parse_insn) {
        if insn.op == "getstatic" {
            last = insn
                .comment
                .strip_prefix("Field ")
                .filter(|f| f.ends_with(":Lnet/minecraft/network/syncher/EntityDataSerializer;"))
                .map(|f| f.split(':').next().unwrap().to_string());
        } else if insn.op == "invokestatic" && insn.comment.contains("registerSerializer:") {
            out.push(last.take().context("registerSerializer without a serializer")?);
        }
    }
    if out.is_empty() {
        bail!("no registerSerializer calls found");
    }
    Ok(out)
}

/// `Pose` constants and their ids: `Pose(String name, int ordinal, int id, String serializedName)`.
fn poses(jar: &Path) -> Result<Vec<(String, i64)>> {
    let text = javap(jar, "net.minecraft.world.entity.Pose")?;
    let (mut out, mut id, mut nums) = (Vec::new(), None, Vec::new());
    for insn in static_init(&text)?.lines().filter_map(parse_insn) {
        if let Some(n) = constant(&insn) {
            nums.push(n);
        } else if insn.op == "invokespecial" && insn.comment.contains("\"<init>\":(Ljava/lang/String;II") {
            id = Some(last_int(&nums, "Pose.<init>")?);
            nums.clear();
        } else if insn.op == "putstatic"
            && insn.comment.ends_with(":Lnet/minecraft/world/entity/Pose;")
            && let Some(id) = id.take()
        {
            let name = insn.comment.trim_start_matches("Field ").split(':').next().unwrap();
            out.push((name.to_string(), id));
        }
    }
    if out.is_empty() {
        bail!("no Pose constants found");
    }
    Ok(out)
}

/// `LivingEntity` -> `living_entity`, `Display$BlockDisplay` -> `display_block_display`.
fn snake_case(class: &str) -> String {
    let simple: Vec<char> = class.rsplit('.').next().unwrap().replace('$', "_").chars().collect();
    let mut s = String::new();
    for (i, &ch) in simple.iter().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 && simple[i - 1] != '_' {
            let prev = simple[i - 1];
            let next_lower = simple.get(i + 1).is_some_and(|n| n.is_ascii_lowercase());
            if prev.is_ascii_lowercase() || prev.is_ascii_digit() || (prev.is_ascii_uppercase() && next_lower) {
                s.push('_');
            }
        }
        s.push(ch.to_ascii_lowercase());
    }
    s
}

/// `DATA_SHARED_FLAGS_ID` -> `SHARED_FLAGS`.
fn data_const_name(field: &str) -> String {
    let s = field.strip_prefix("DATA_").unwrap_or(field);
    s.strip_suffix("_ID").unwrap_or(s).to_string()
}

const ENTITIES_PRELUDE: &str = r#"/// A synched data field: index in `set_entity_data` and serializer id (`serializer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataField {
    pub index: u8,
    pub serializer: i32,
}

/// An entity class and the data fields it defines with `SynchedEntityData.defineId`.
#[derive(Debug)]
pub struct EntityClass {
    /// Fully qualified Java class name.
    pub name: &'static str,
    /// Index of the superclass in `CLASSES` (`None` for `Entity`).
    pub parent: Option<u16>,
    /// (Java field name, field) for the fields this class itself defines.
    pub fields: &'static [(&'static str, DataField)],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityType {
    pub name: &'static str,
    /// Protocol id in `minecraft:entity_type`.
    pub id: i32,
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
    /// Client tracking range in chunks.
    pub tracking_range: i32,
    /// Ticks between position updates (`i32::MAX`: never).
    pub update_interval: i32,
    /// Whether velocity changes are broadcast (cleared by `dontTrackDeltas`).
    pub track_deltas: bool,
    /// Index of the entity class in `CLASSES`.
    pub class: u16,
}

impl EntityType {
    /// Every data field of this type, base class first (ascending index).
    pub fn fields(&self) -> Vec<(&'static str, DataField)> {
        let mut chain = Vec::new();
        let mut class = Some(self.class);
        while let Some(c) = class {
            let c = &CLASSES[c as usize];
            chain.push(c.fields);
            class = c.parent;
        }
        chain.into_iter().rev().flatten().copied().collect()
    }
}

pub fn by_id(id: i32) -> Option<&'static EntityType> {
    usize::try_from(id).ok().and_then(|i| TYPES.get(i))
}

pub fn by_name(name: &str) -> Option<&'static EntityType> {
    TYPES.iter().find(|t| t.name == name)
}
"#;

const RUST_KEYWORDS: &[&str] = &[
    "as", "box", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if",
    "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct",
    "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "gen", "try", "yield",
];

pub(crate) fn gen_entities(input: &Input) -> Result<String> {
    let jar = &input.server_jar;
    let serializers = entity_data_serializers(jar)?;

    let registries = read_json(&input.generated.join("reports/registries.json"))?;
    let protocol_ids = registries["minecraft:entity_type"]["entries"].as_object().context("entity_type registry")?;
    let mut types = entity_types(jar)?;
    if types.len() != protocol_ids.len() {
        bail!("{} registered entity types, registry report has {}", types.len(), protocol_ids.len());
    }
    let id_of = |t: &TypeSpec| protocol_ids.get(&t.key).and_then(|e| e["protocol_id"].as_i64());
    types.sort_by_key(|t| id_of(t).unwrap_or(i64::MAX));
    for (i, t) in types.iter().enumerate() {
        if id_of(t) != Some(i as i64) {
            bail!("entity type ids are not dense at {}", t.key);
        }
    }

    let classes = entity_classes(jar, types.iter().map(|t| t.class.clone()))?;
    fn parent<'a>(classes: &'a BTreeMap<String, EntityClassInfo>, name: &str) -> Option<&'a str> {
        if name == ENTITY_CLASS { None } else { classes[name].parent.as_deref() }
    }
    let parent_of = |name: &str| parent(&classes, name);
    let depth = |name: &str| {
        let (mut d, mut cur) = (0, parent_of(name));
        while let Some(p) = cur {
            (d, cur) = (d + 1, parent_of(p));
        }
        d
    };
    let mut order: Vec<&str> = classes.keys().map(String::as_str).collect();
    order.sort_by_key(|n| (depth(n), *n));
    let index_of: HashMap<&str, usize> = order.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    // Data indices continue from the superclass's last one (ClassTreeIdRegistry).
    let mut first_index: HashMap<&str, usize> = HashMap::new();
    for name in &order {
        let base = parent_of(name).map_or(0, |p| first_index[p] + classes[p].fields.len());
        if base + classes[*name].fields.len() > 255 {
            bail!("{name}: more than 255 data fields");
        }
        first_index.insert(name, base);
    }
    let mut modules = HashSet::new();
    for name in &order {
        let m = snake_case(name);
        if RUST_KEYWORDS.contains(&m.as_str()) || !modules.insert(m.clone()) {
            bail!("module name {m} for {name} collides");
        }
    }

    let mut s =
        String::from("// @generated by `cargo xtask codegen` from the vanilla server jar (javap). Do not edit.\n\n");
    s.push_str(ENTITIES_PRELUDE);

    writeln!(s, "\n/// `EntityDataSerializers` network ids (registration order).")?;
    writeln!(s, "pub mod serializer {{")?;
    for (i, name) in serializers.iter().enumerate() {
        writeln!(s, "    pub const {name}: i32 = {i};")?;
    }
    writeln!(s, "\n    pub const NAMES: &[&str] = &[")?;
    for name in &serializers {
        writeln!(s, "        {:?},", name.to_ascii_lowercase())?;
    }
    writeln!(s, "    ];\n}}")?;

    writeln!(s, "\n/// `Pose` ids (serializer `POSE`).")?;
    writeln!(s, "pub mod pose {{")?;
    for (name, id) in poses(jar)? {
        writeln!(s, "    pub const {name}: i32 = {id};")?;
    }
    writeln!(s, "}}")?;

    writeln!(s, "\n/// Entity types by name.")?;
    writeln!(s, "pub mod types {{")?;
    writeln!(s, "    use super::EntityType;\n")?;
    for (i, t) in types.iter().enumerate() {
        writeln!(
            s,
            "    pub const {}: EntityType = EntityType {{ name: {:?}, id: {i}, width: {:?}, height: {:?}, \
             eye_height: {:?}, tracking_range: {}, update_interval: {}, track_deltas: {}, class: {} }};",
            const_name(&t.key),
            t.key,
            t.width,
            t.height,
            t.eye_height,
            t.tracking_range,
            t.update_interval,
            t.track_deltas,
            index_of[t.class.as_str()],
        )?;
    }
    writeln!(s, "}}")?;
    writeln!(s, "\n/// Entity types indexed by protocol id.")?;
    writeln!(s, "pub const TYPES: &[EntityType] = &[")?;
    for t in &types {
        writeln!(s, "    types::{},", const_name(&t.key))?;
    }
    writeln!(s, "];")?;

    writeln!(s, "\n/// Entity classes, superclasses first.")?;
    writeln!(s, "pub const CLASSES: &[EntityClass] = &[")?;
    for name in &order {
        let parent = parent_of(name).map_or("None".to_string(), |p| format!("Some({})", index_of[p]));
        let m = snake_case(name);
        let fields: Vec<String> =
            classes[*name].fields.iter().map(|(f, _)| format!("({f:?}, data::{m}::{})", data_const_name(f))).collect();
        writeln!(s, "    EntityClass {{ name: {name:?}, parent: {parent}, fields: &[{}] }},", fields.join(", "))?;
    }
    writeln!(s, "];")?;

    writeln!(s, "\n/// Data fields by defining class.")?;
    writeln!(s, "pub mod data {{")?;
    let mut field_count = 0;
    for name in &order {
        let c = &classes[*name];
        if c.fields.is_empty() {
            continue;
        }
        writeln!(s, "    /// `{name}`")?;
        writeln!(s, "    pub mod {} {{", snake_case(name))?;
        writeln!(s, "        use super::super::{{DataField, serializer as s}};\n")?;
        let mut seen = HashSet::new();
        for (i, (field, ser)) in c.fields.iter().enumerate() {
            let cn = data_const_name(field);
            if !seen.insert(cn.clone()) {
                bail!("{name}: duplicate data constant {cn}");
            }
            if !serializers.contains(ser) {
                bail!("{name}.{field}: unknown serializer {ser}");
            }
            writeln!(
                s,
                "        pub const {cn}: DataField = DataField {{ index: {}, serializer: s::{ser} }};",
                first_index[*name] + i
            )?;
            field_count += 1;
        }
        writeln!(s, "    }}")?;
    }
    writeln!(s, "}}")?;
    println!(
        "codegen: {} entity types, {} entity classes, {field_count} data fields, {} serializers",
        types.len(),
        order.len(),
        serializers.len()
    );
    Ok(s)
}
