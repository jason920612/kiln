//! `data` (vanilla `DataCommands`): get, merge, modify and remove the NBT of block entities,
//! entities and command storage.

use super::LEVEL_GAMEMASTERS;
use super::blocks::loaded_block_pos;
use crate::arguments::ArgumentType;
use crate::dispatcher::{Builder, CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::Host;
use crate::nbt_path::{NbtPath, is_too_deep, merge_compound, nbt_eq};
use crate::nbt_text::{pretty, snbt};
use crate::selector::SelectorTarget;
use crate::text::Text;
use crate::tr;
use kiln_proto::nbt::Tag;

type Result<T> = std::result::Result<T, CommandError>;

/// What `/data` reads and writes (`DataAccessor`).
pub(super) enum Accessor<E> {
    Block { dimension: String, pos: [i32; 3] },
    Entity(E),
    Storage(String),
}

/// The target and source kinds (`ArgProvider`s), in vanilla's order.
#[derive(Clone, Copy)]
enum Kind {
    Block,
    Entity,
    Storage,
}

const KINDS: [Kind; 3] = [Kind::Block, Kind::Entity, Kind::Storage];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Block => "block",
            Kind::Entity => "entity",
            Kind::Storage => "storage",
        }
    }

    fn argument<S: Host + 'static>(self, name: &str) -> Builder<S> {
        match self {
            Kind::Block => argument(name, ArgumentType::BlockPos),
            Kind::Entity => argument(name, ArgumentType::entity()),
            Kind::Storage => argument(name, ArgumentType::ResourceLocation).suggests_server(|_, s: &S, b| {
                let ids: Vec<String> = s.storage_ids();
                b.suggest_resources(ids.iter().map(String::as_str), "");
            }),
        }
    }

    /// `ArgProvider.access`.
    fn access<S: Host>(self, c: &CommandContext<S>, s: &mut S, name: &str) -> Result<Accessor<S::Entity>> {
        Ok(match self {
            Kind::Block => {
                let dimension = s.dimension().to_owned();
                let pos = loaded_block_pos(c, s, name, &dimension)?;
                if s.block_entity(&dimension, pos).is_none() {
                    return Err(CommandError::new(tr!("commands.data.block.invalid")));
                }
                Accessor::Block { dimension, pos }
            }
            Kind::Entity => Accessor::Entity(c.selector(name).entity(s)?),
            Kind::Storage => Accessor::Storage(c.identifier(name).to_string()),
        })
    }
}

impl<E: SelectorTarget> Accessor<E> {
    /// `getData`.
    pub(super) fn get<S: Host<Entity = E>>(&self, s: &mut S) -> Result<Tag> {
        match self {
            Accessor::Block { dimension, pos } => {
                s.block_entity(dimension, *pos).ok_or_else(|| CommandError::new(tr!("commands.data.block.invalid")))
            }
            Accessor::Entity(e) => s.entity_data(e).ok_or_else(|| CommandError::unsupported("Entity data")),
            Accessor::Storage(id) => Ok(s.storage_mut().map_or(Tag::Compound(Vec::new()), |st| st.get(id))),
        }
    }

    /// `setData`.
    pub(super) fn set<S: Host<Entity = E>>(&self, s: &mut S, data: Tag) -> Result<()> {
        match self {
            Accessor::Block { dimension, pos } => s.set_block_entity_data(dimension, *pos, &data),
            Accessor::Entity(e) if e.is_player() => Err(CommandError::new(tr!("commands.data.entity.invalid"))),
            Accessor::Entity(e) => s.set_entity_data(e, &data),
            Accessor::Storage(id) => match s.storage_mut() {
                Some(st) => {
                    st.set(id, data);
                    Ok(())
                }
                None => Err(CommandError::unsupported("Command storage")),
            },
        }
    }

    fn modified(&self) -> Text {
        match self {
            Accessor::Block { pos: [x, y, z], .. } => tr!("commands.data.block.modified", *x, *y, *z),
            Accessor::Entity(e) => tr!("commands.data.entity.modified", e.display_name()),
            Accessor::Storage(id) => tr!("commands.data.storage.modified", id.as_str()),
        }
    }

    fn print(&self, tag: &Tag) -> Text {
        let p = pretty(tag);
        match self {
            Accessor::Block { pos: [x, y, z], .. } => tr!("commands.data.block.query", *x, *y, *z, p),
            Accessor::Entity(e) => tr!("commands.data.entity.query", e.display_name(), p),
            Accessor::Storage(id) => tr!("commands.data.storage.query", id.as_str(), p),
        }
    }

    fn print_numeric(&self, path: &NbtPath, scale: f64, value: i32) -> Text {
        let scale = format_2f(scale);
        let path = path.text.as_str();
        match self {
            Accessor::Block { pos: [x, y, z], .. } => tr!("commands.data.block.get", path, *x, *y, *z, scale, value),
            Accessor::Entity(e) => tr!("commands.data.entity.get", path, e.display_name(), scale, value),
            Accessor::Storage(id) => tr!("commands.data.storage.get", path, id.as_str(), scale, value),
        }
    }
}

/// `String.format(Locale.ROOT, "%.2f", v)` (half-up rounding of the exact value).
pub(super) fn format_2f(v: f64) -> String {
    if !v.is_finite() {
        return if v.is_nan() { "NaN".into() } else if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    // Rust rounds exact ties to even; Java's `Formatter` rounds them up.
    let a = v.abs();
    let exact = format!("{a:.60}");
    let exact = exact.trim_end_matches('0');
    let tie = exact.split_once('.').is_some_and(|(_, frac)| frac.len() == 3 && frac.ends_with('5'));
    let s = if tie { format!("{:.2}", a + 0.001) } else { format!("{a:.2}") };
    if v.is_sign_negative() { format!("-{s}") } else { s }
}

/// `getSingleTag`.
fn single_tag(path: &NbtPath, data: &Tag) -> Result<Tag> {
    let found = path.get_checked(data)?;
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        _ => Err(CommandError::new(tr!("commands.data.get.multiple"))),
    }
}

/// `Mth.floor`.
fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 { i - 1 } else { i }
}

fn numeric(tag: &Tag) -> Option<f64> {
    Some(match tag {
        Tag::Byte(v) => *v as f64,
        Tag::Short(v) => *v as f64,
        Tag::Int(v) => *v as f64,
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => *v as f64,
        Tag::Double(v) => *v,
        _ => return None,
    })
}

fn get_all<S: Host>(s: &mut S, acc: &Accessor<S::Entity>) -> Result<i32> {
    let data = acc.get(s)?;
    s.send_success(acc.print(&data), false);
    Ok(1)
}

fn get_path<S: Host>(s: &mut S, acc: &Accessor<S::Entity>, path: &NbtPath) -> Result<i32> {
    let data = acc.get(s)?;
    let tag = single_tag(path, &data)?;
    let value = match &tag {
        Tag::List(l) => l.len() as i32,
        Tag::ByteArray(a) => a.len() as i32,
        Tag::IntArray(a) => a.len() as i32,
        Tag::LongArray(a) => a.len() as i32,
        Tag::Compound(f) => f.len() as i32,
        Tag::String(st) => st.encode_utf16().count() as i32,
        other => floor(numeric(other).expect("numeric")),
    };
    s.send_success(acc.print(&tag), false);
    Ok(value)
}

fn get_numeric<S: Host>(s: &mut S, acc: &Accessor<S::Entity>, path: &NbtPath, scale: f64) -> Result<i32> {
    let data = acc.get(s)?;
    let tag = single_tag(path, &data)?;
    let Some(v) = numeric(&tag) else {
        return Err(CommandError::new(tr!("commands.data.get.invalid", path.text.as_str())));
    };
    let value = floor(v * scale);
    s.send_success(acc.print_numeric(path, scale, value), false);
    Ok(value)
}

fn merge<S: Host>(s: &mut S, acc: &Accessor<S::Entity>, nbt: &Tag) -> Result<i32> {
    let data = acc.get(s)?;
    if is_too_deep(nbt, 0) {
        return Err(CommandError::new(tr!("arguments.nbtpath.too_deep")));
    }
    let mut merged = data.clone();
    if let (Tag::Compound(into), Tag::Compound(from)) = (&mut merged, nbt) {
        merge_compound(into, from);
    }
    if nbt_eq(&data, &merged) {
        return Err(CommandError::new(tr!("commands.data.merge.failed")));
    }
    acc.set(s, merged)?;
    s.send_success(acc.modified(), true);
    Ok(1)
}

fn remove<S: Host>(s: &mut S, acc: &Accessor<S::Entity>, path: &NbtPath) -> Result<i32> {
    let mut data = acc.get(s)?;
    let removed = path.remove(&mut data);
    if removed == 0 {
        return Err(CommandError::new(tr!("commands.data.merge.failed")));
    }
    acc.set(s, data)?;
    s.send_success(acc.modified(), true);
    Ok(removed)
}

/// `DataManipulator`s.
#[derive(Clone, Copy)]
enum Op {
    Insert,
    Prepend,
    Append,
    Set,
    Merge,
}

impl Op {
    fn apply(self, c: &CommandContext<impl Host>, target: &mut Tag, path: &NbtPath, sources: &[Tag]) -> Result<i32> {
        match self {
            Op::Set => {
                let last = sources.last().expect("a source path finds something");
                path.set(target, last)
            }
            Op::Append => path.insert(-1, target, sources),
            Op::Prepend => path.insert(0, target, sources),
            Op::Insert => path.insert(c.integer("index"), target, sources),
            Op::Merge => {
                let mut merged = Vec::new();
                for src in sources {
                    if is_too_deep(src, 0) {
                        return Err(CommandError::new(tr!("arguments.nbtpath.too_deep")));
                    }
                    match src {
                        Tag::Compound(f) => merge_compound(&mut merged, f),
                        other => return Err(CommandError::new(tr!("commands.data.modify.expected_object", snbt(other)))),
                    }
                }
                let targets = path.get_or_create(target, || Tag::Compound(Vec::new()))?;
                let mut changed = 0;
                for t in targets {
                    let Tag::Compound(fields) = t else {
                        return Err(CommandError::new(tr!("commands.data.modify.expected_object", snbt(t))));
                    };
                    let before = Tag::Compound(fields.clone());
                    merge_compound(fields, &merged);
                    changed += !nbt_eq(&before, &Tag::Compound(fields.clone())) as i32;
                }
                Ok(changed)
            }
        }
    }
}

/// `manipulateData`.
fn manipulate<S: Host>(c: &CommandContext<S>, s: &mut S, kind: Kind, op: Op, sources: Vec<Tag>) -> Result<i32> {
    let acc = kind.access(c, s, "target")?;
    let path = c.nbt_path("targetPath");
    let mut data = acc.get(s)?;
    let changed = op.apply(c, &mut data, path, &sources)?;
    if changed == 0 {
        return Err(CommandError::new(tr!("commands.data.merge.failed")));
    }
    acc.set(s, data)?;
    s.send_success(acc.modified(), true);
    Ok(changed)
}

/// `getAsText`: strings as themselves, numbers as SNBT.
fn as_text(tag: &Tag) -> Result<String> {
    match tag {
        Tag::String(s) => Ok(s.clone()),
        Tag::Byte(_) | Tag::Short(_) | Tag::Int(_) | Tag::Long(_) | Tag::Float(_) | Tag::Double(_) => Ok(snbt(tag)),
        other => Err(CommandError::new(tr!("commands.data.modify.expected_value", snbt(other)))),
    }
}

/// `substring` with `getOffset` and `validatedSubstring`, in UTF-16 units like Java.
fn substring(s: &str, start: i32, end: Option<i32>) -> Result<String> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let len = units.len() as i32;
    let offset = |i: i32| if i < 0 { len + i } else { i };
    let (a, b) = (offset(start), end.map_or(len, offset));
    if a < 0 || b > len || a > b {
        return Err(CommandError::new(tr!("commands.data.modify.invalid_substring", a, b)));
    }
    Ok(String::from_utf16_lossy(&units[a as usize..b as usize]))
}

/// The source subtree of one modification (`decorateModification`'s `from`, `string`,
/// `value` and `compute`).
fn sources<S: Host + 'static>(b: Builder<S>, kind: Kind, op: Op) -> Builder<S> {
    let mut from = literal("from");
    let mut string = literal("string");
    for src in KINDS {
        from = from.then(literal(src.name()).then(
            src.argument("source")
                .executes(move |c, s: &mut S| {
                    let data = src.access(c, s, "source")?.get(s)?;
                    manipulate(c, s, kind, op, vec![data])
                })
                .then(argument("sourcePath", ArgumentType::NbtPath).executes(move |c, s: &mut S| {
                    let data = src.access(c, s, "source")?.get(s)?;
                    let found: Vec<Tag> = c.nbt_path("sourcePath").get_checked(&data)?.into_iter().cloned().collect();
                    manipulate(c, s, kind, op, found)
                })),
        ));
        let strings = move |c: &CommandContext<S>, s: &mut S, path: bool, range: fn(&CommandContext<S>) -> (Option<i32>, Option<i32>)| {
            let data = src.access(c, s, "source")?.get(s)?;
            let found: Vec<Tag> =
                if path { c.nbt_path("sourcePath").get_checked(&data)?.into_iter().cloned().collect() } else { vec![data] };
            let (start, end) = range(c);
            let mut out = Vec::with_capacity(found.len());
            for t in &found {
                let text = as_text(t)?;
                let text = match start {
                    Some(start) => substring(&text, start, end)?,
                    None => text,
                };
                out.push(Tag::String(text));
            }
            manipulate(c, s, kind, op, out)
        };
        string = string.then(literal(src.name()).then(
            src.argument("source").executes(move |c, s: &mut S| strings(c, s, false, |_| (None, None))).then(
                argument("sourcePath", ArgumentType::NbtPath).executes(move |c, s: &mut S| strings(c, s, true, |_| (None, None))).then(
                    argument("start", ArgumentType::integer())
                        .executes(move |c, s: &mut S| strings(c, s, true, |c| (Some(c.integer("start")), None)))
                        .then(argument("end", ArgumentType::integer()).executes(move |c, s: &mut S| {
                            strings(c, s, true, |c| (Some(c.integer("start")), Some(c.integer("end"))))
                        })),
                ),
            ),
        ));
    }
    let value = literal("value").then(
        argument("value", ArgumentType::NbtTag).executes(move |c, s: &mut S| manipulate(c, s, kind, op, vec![c.nbt("value").clone()])),
    );
    b.then(compute_node(kind, op)).then(from).then(string).then(value)
}

/// `compute default|block|entity (float <provider> [scale] | integer <provider>)`.
fn compute_node<S: Host + 'static>(kind: Kind, op: Op) -> Builder<S> {
    use super::compute::{evaluate, provider_arg, Branch};
    let numbers = move |branch: Branch| {
        let run = move |c: &CommandContext<S>, s: &mut S, float: bool| {
            let target = branch.target(c, s)?;
            let v = evaluate(s, &provider_arg(c), float, &target)?;
            let tag = if float {
                let scale = c.get("scale").map_or(1.0, |_| c.float("scale"));
                Tag::Float(v as f32 * scale)
            } else {
                Tag::Int(v as i32)
            };
            manipulate(c, s, kind, op, vec![tag])
        };
        [
            literal("float").then(argument("provider", ArgumentType::ContextProvider { float: true }).executes(move |c, s: &mut S| run(c, s, true))),
            literal("integer").then(
                argument("provider", ArgumentType::ContextProvider { float: false }).executes(move |c, s: &mut S| run(c, s, false)),
            ),
        ]
    };
    let with = |b: Builder<S>, branch: Branch| {
        let [f, i] = numbers(branch);
        b.then(f).then(i)
    };
    literal("compute")
        .then(with(literal("default"), Branch::Default))
        .then(literal("block").then(with(argument("computePos", ArgumentType::BlockPos), Branch::Block)))
        .then(literal("entity").then(with(argument("computeTarget", ArgumentType::entity()), Branch::Entity)))
}

pub fn data<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let mut merge_node = literal("merge");
    let mut get_node = literal("get");
    let mut remove_node = literal("remove");
    let mut modify_node = literal("modify");
    for kind in KINDS {
        merge_node = merge_node.then(literal(kind.name()).then(kind.argument("target").then(
            argument("nbt", ArgumentType::NbtCompound).executes(move |c, s: &mut S| {
                let acc = kind.access(c, s, "target")?;
                merge(s, &acc, c.nbt("nbt"))
            }),
        )));
        get_node = get_node.then(
            literal(kind.name()).then(
                kind.argument("target")
                    .executes(move |c, s: &mut S| {
                        let acc = kind.access(c, s, "target")?;
                        get_all(s, &acc)
                    })
                    .then(
                        argument("path", ArgumentType::NbtPath)
                            .executes(move |c, s: &mut S| {
                                let acc = kind.access(c, s, "target")?;
                                get_path(s, &acc, c.nbt_path("path"))
                            })
                            .then(argument("scale", ArgumentType::double()).executes(move |c, s: &mut S| {
                                let acc = kind.access(c, s, "target")?;
                                get_numeric(s, &acc, c.nbt_path("path"), c.double("scale"))
                            })),
                    ),
            ),
        );
        remove_node = remove_node.then(literal(kind.name()).then(kind.argument("target").then(
            argument("path", ArgumentType::NbtPath).executes(move |c, s: &mut S| {
                let acc = kind.access(c, s, "target")?;
                remove(s, &acc, c.nbt_path("path"))
            }),
        )));
        let path = argument("targetPath", ArgumentType::NbtPath)
            .then(literal("insert").then(sources(argument("index", ArgumentType::integer()), kind, Op::Insert)))
            .then(sources(literal("prepend"), kind, Op::Prepend))
            .then(sources(literal("append"), kind, Op::Append))
            .then(sources(literal("set"), kind, Op::Set))
            .then(sources(literal("merge"), kind, Op::Merge));
        modify_node = modify_node.then(literal(kind.name()).then(kind.argument("target").then(path)));
    }
    d.register(literal("data").requires(LEVEL_GAMEMASTERS).then(merge_node).then(get_node).then(remove_node).then(modify_node));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_decimals() {
        assert_eq!(format_2f(1.0), "1.00");
        assert_eq!(format_2f(0.125), "0.13");
        assert_eq!(format_2f(2.675), "2.67"); // not exactly a tie in binary
        assert_eq!(format_2f(-0.5), "-0.50");
    }

    #[test]
    fn substrings() {
        assert_eq!(substring("hello", 1, Some(3)).unwrap(), "el");
        assert_eq!(substring("hello", -3, None).unwrap(), "llo");
        assert_eq!(substring("hello", 3, Some(1)).unwrap_err().key(), Some("commands.data.modify.invalid_substring"));
    }
}
