//! Functions: `/function` (`FunctionCommand`), `/return` (`ReturnCommand`), `/schedule`
//! (`ScheduleCommand`), `/reload` and `/datapack` (`DataPackCommand`), and the call machinery
//! the host uses for `#minecraft:load`, `#minecraft:tick` and scheduled functions.
//!
//! Vanilla queues function bodies on an execution context; Kiln runs them depth first,
//! which gives the same order: a called function runs to its end (or its `return`) before the
//! caller's next line. Each call has a [`Frame`] that `return` reports to and discards, and
//! all calls of one top-level command share its command quota.

use super::LEVEL_GAMEMASTERS;
use super::LEVEL_OWNERS;
use crate::arguments::{ArgumentType, ArgumentValue};
use crate::dispatcher::{CommandContext, Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::functions::{CommandFunction, DataPacks, PackSource, TimerCallback};
use crate::host::{Frame, Host, SourceStack};
use crate::text::Text;
use crate::tr;
use crate::types::Identifier;
use kiln_proto::nbt::Tag;
use std::sync::Arc;
use std::sync::atomic::Ordering;

type Result<T> = std::result::Result<T, CommandError>;

/// Deepest function nesting Kiln follows (vanilla is bounded only by the command quota).
const MAX_DEPTH: u32 = 512;

/// Runs a function body (`CallFunction`): its lines in a new frame with `stack` as the source,
/// until the end, a `return` or the command quota runs out. Returns what the frame's
/// `return`s reported, in order.
pub fn run_function<S: Host>(
    dispatcher: &Dispatcher<S>,
    source: &mut S,
    function: &CommandFunction,
    arguments: Option<&Tag>,
    mut stack: SourceStack<S>,
) -> std::result::Result<Vec<(bool, i32)>, Text> {
    let lines = instantiate(dispatcher, source, function, arguments, &stack)?;
    let quota = stack.quota.clone();
    if quota.load(Ordering::Relaxed) <= 0 || stack.frame.depth >= MAX_DEPTH {
        return Ok(Vec::new());
    }
    quota.fetch_sub(1, Ordering::Relaxed);
    let frame = Frame::new(stack.frame.depth + 1);
    stack.frame = frame.clone();
    stack.returning = false;
    let original = std::mem::replace(source.stack_mut(), stack);
    for line in &lines {
        if frame.is_discarded() || quota.load(Ordering::Relaxed) <= 0 {
            break;
        }
        // Errors in function bodies go to the (silent) source.
        let _ = dispatcher.run_nested(line, source);
    }
    *source.stack_mut() = original;
    Ok(frame.take_outcomes())
}

/// `CommandFunction.instantiate`: the lines to run. Macro lines are substituted and parsed
/// (with `stack` as the source); a line that does not parse fails the call.
fn instantiate<S: Host>(
    dispatcher: &Dispatcher<S>,
    source: &mut S,
    function: &CommandFunction,
    arguments: Option<&Tag>,
    stack: &SourceStack<S>,
) -> std::result::Result<Vec<String>, Text> {
    let lines = function.instantiate(arguments)?;
    if function.is_macro() {
        let original = std::mem::replace(source.stack_mut(), stack.clone());
        let bad = lines.iter().find_map(|l| {
            let parse = dispatcher.parse(l, source);
            dispatcher.check_parse(&parse).err().map(|e| (l.clone(), e))
        });
        *source.stack_mut() = original;
        if let Some((line, e)) = bad {
            return Err(tr!("commands.function.error.parse", function.id.to_string(), line, exception_message(&e)));
        }
    }
    Ok(lines)
}

/// `CommandSyntaxException.getMessage`: the message, then for parse errors
/// ` at position <cursor>: ...<ten characters before it><--[HERE]`.
fn exception_message(e: &CommandError) -> Text {
    let mut out = Text::empty().append(e.message().clone());
    if let (Some(input), Some(cursor)) = (e.input(), e.cursor()) {
        let cursor = cursor.min(input.len());
        let before = &input[..cursor];
        let skip = before.chars().count().saturating_sub(10);
        let start = before.char_indices().nth(skip).map_or(before.len(), |(i, _)| i);
        let dots = if skip > 0 { "..." } else { "" };
        let context = format!("{dots}{}<--[HERE]", &before[start..]);
        let position = input[..cursor].encode_utf16().count();
        out = out.append(Text::literal(format!(" at position {position}: {context}")));
    }
    out
}

/// `ServerFunctionManager.execute` with the game loop sender: `stack` should be the server's
/// source (the host makes it silent, at permission level 2). Failures are logged by the
/// caller.
pub fn run_as_server<S: Host>(
    dispatcher: &Dispatcher<S>,
    source: &mut S,
    function: &CommandFunction,
    stack: SourceStack<S>,
) -> std::result::Result<(), Text> {
    let mut stack = stack;
    stack.silent = true;
    stack.max_permission = 2;
    stack.frame = Frame::new(0);
    stack.quota = Arc::new(std::sync::atomic::AtomicI32::new(source.command_limit().max(1)));
    stack.callbacks.clear();
    // The function is the top of its own execution (`queueInitialFunctionCall`).
    stack.frame = Frame::new(0);
    run_function(dispatcher, source, function, None, stack).map(|_| ())
}

fn library_missing() -> CommandError {
    CommandError::unsupported("Functions")
}

/// `FunctionArgument.getFunctionCollection`: the id and its functions (a tag may be empty).
fn function_collection<S: Host>(c: &CommandContext<S>, s: &S, name: &str) -> Result<(Identifier, Vec<Arc<CommandFunction>>)> {
    let Some(ArgumentValue::Function { tag, id }) = c.get(name) else { unreachable!("function argument") };
    let lib = s.functions().ok_or_else(library_missing)?;
    if *tag {
        Ok((id.clone(), lib.tag(id)))
    } else {
        match lib.get(id) {
            Some(f) => Ok((id.clone(), vec![f])),
            None => Err(CommandError::new(tr!("arguments.function.unknown", id.to_string()))),
        }
    }
}

pub fn function<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let sources = ["block", "entity", "storage"];
    let with = sources.into_iter().fold(literal("with"), |b, kind| {
        let source = match kind {
            "block" => argument("source", ArgumentType::BlockPos),
            "entity" => argument("source", ArgumentType::entity()),
            _ => argument("source", ArgumentType::ResourceLocation),
        };
        b.then(literal(kind).then(
            source
                .executes(move |c, s: &mut S| {
                    let data = data_source(c, s, kind)?;
                    call_functions(c, s, Some(data))
                })
                .custom()
                .then(
                    argument("path", ArgumentType::NbtPath)
                        .executes(move |c, s: &mut S| {
                            let data = data_source(c, s, kind)?;
                            let tag = single_tag(c, &data)?;
                            call_functions(c, s, Some(tag))
                        })
                        .custom(),
                ),
        ))
    });
    d.register(
        literal("function").requires(LEVEL_GAMEMASTERS).then(
            argument("name", ArgumentType::Function)
                .suggests_server(suggest_functions)
                .executes(|c, s: &mut S| call_functions(c, s, None))
                .custom()
                .then(
                    argument("arguments", ArgumentType::NbtCompound)
                        .executes(|c, s: &mut S| {
                            let args = c.nbt("arguments").clone();
                            call_functions(c, s, Some(args))
                        })
                        .custom(),
                )
                .then(with),
        ),
    );
}

pub(super) fn suggest_functions<S: Host>(_: &CommandContext<S>, s: &S, b: &mut crate::suggestion::SuggestionsBuilder) {
    let Some(lib) = s.functions() else { return };
    let tags: Vec<String> = lib.tag_names().map(Identifier::to_string).collect();
    b.suggest_resources(tags.iter().map(String::as_str), "#");
    let names: Vec<String> = lib.function_names().map(Identifier::to_string).collect();
    b.suggest_resources(names.iter().map(String::as_str), "");
}

/// The data a `with block|entity|storage` source gives (`DataAccessor.getData`).
fn data_source<S: Host>(c: &CommandContext<S>, s: &mut S, kind: &str) -> Result<Tag> {
    match kind {
        "block" => {
            let dimension = s.dimension().to_owned();
            let pos = super::blocks::loaded_block_pos(c, s, "source", &dimension)?;
            s.block_entity(&dimension, pos).ok_or_else(|| CommandError::new(tr!("commands.data.block.invalid")))
        }
        "entity" => {
            c.selector("source").entity(s)?;
            Err(CommandError::unsupported("Entity data"))
        }
        _ => {
            let id = c.identifier("source").to_string();
            Ok(s.storage_mut().map_or(Tag::Compound(Vec::new()), |st| st.get(&id)))
        }
    }
}

/// `FunctionCommand.getArgumentTag`: exactly one compound at the path.
fn single_tag<S: Host>(c: &CommandContext<S>, data: &Tag) -> Result<Tag> {
    let path = c.nbt_path("path");
    let found = path.get(data);
    let tag = match found.as_slice() {
        [] => return Err(CommandError::new(tr!("arguments.nbtpath.nothing_found", c.arg_text("path").unwrap_or("")))),
        [one] => (*one).clone(),
        _ => return Err(CommandError::new(tr!("commands.data.get.multiple"))),
    };
    match tag {
        Tag::Compound(_) => Ok(tag),
        other => Err(CommandError::new(tr!("commands.function.error.argument_not_compound", tag_type_name(&other)))),
    }
}

/// `TagType.getName`.
fn tag_type_name(tag: &Tag) -> &'static str {
    match tag {
        Tag::Byte(_) => "BYTE",
        Tag::Short(_) => "SHORT",
        Tag::Int(_) => "INT",
        Tag::Long(_) => "LONG",
        Tag::Float(_) => "FLOAT",
        Tag::Double(_) => "DOUBLE",
        Tag::ByteArray(_) => "BYTE[]",
        Tag::String(_) => "STRING",
        Tag::List(_) => "LIST",
        Tag::Compound(_) => "COMPOUND",
        Tag::IntArray(_) => "INT[]",
        Tag::LongArray(_) => "LONG[]",
    }
}

/// `FunctionCustomExecutor.runGuarded` (the first pass: report and instantiate) and
/// `queueFunctions` (the second: run the bodies).
fn call_functions<S: Host>(c: &CommandContext<S>, s: &mut S, arguments: Option<Tag>) -> Result<i32> {
    let (id, functions) = function_collection(c, s, "name")?;
    if functions.is_empty() {
        return Err(CommandError::new(tr!("commands.function.scheduled.no_functions", id.to_string())));
    }
    let stack = s.stack().clone();
    let body = stack.clone().without_callbacks().for_function_body();
    let dispatcher = c.dispatcher();
    let failure =
        |f: &CommandFunction, message: Text| CommandError::new(tr!("commands.function.instantiationFailure", f.id.to_string(), message));
    if stack.preparing {
        let text = match functions.as_slice() {
            [one] => tr!("commands.function.scheduled.single", one.id.to_string()),
            all => tr!(
                "commands.function.scheduled.multiple",
                Text::join(all.iter().map(|f| Text::literal(f.id.to_string())))
            ),
        };
        s.send_success(text, true);
        for f in &functions {
            instantiate(dispatcher, s, f, arguments.as_ref(), &body).map_err(|m| failure(f, m))?;
        }
        return Err(CommandError::deferred());
    }
    let run = |s: &mut S, f: &CommandFunction| -> Result<Vec<(bool, i32)>> {
        run_function(dispatcher, s, f, arguments.as_ref(), body.clone()).map_err(|m| failure(f, m))
    };
    // `decorateOutputIfNeeded`: each returned value is announced to a source that is not
    // silent (the host drops feedback for silent stacks).
    let signal = |s: &mut S, f: &CommandFunction, value: i32| {
        s.send_success(tr!("commands.function.result", f.id.to_string(), value), true);
    };
    let callbacks = stack.callbacks.clone();
    let notify = |s: &mut S, success: bool, value: i32| {
        for cb in &callbacks {
            cb(s, success, value);
        }
    };
    if stack.returning {
        // `queueFunctionsAsReturn`: the first function that returns ends the caller's frame.
        for f in &functions {
            let outcomes = run(s, f)?;
            if !outcomes.is_empty() {
                for (ok, v) in outcomes {
                    signal(s, f, v);
                    notify(s, ok, v);
                    stack.frame.report(ok, v);
                }
                return Err(CommandError::deferred());
            }
        }
        // `FallthroughTask`.
        stack.frame.report(false, 0);
        return Err(CommandError::deferred());
    }
    if callbacks.is_empty() || functions.len() == 1 {
        for f in &functions {
            for (ok, v) in run(s, f)? {
                signal(s, f, v);
                notify(s, ok, v);
            }
        }
    } else {
        // `Accumulator`: the sum of every returned value, if any function returned.
        let (mut any, mut sum) = (false, 0i32);
        for f in &functions {
            for (_, v) in run(s, f)? {
                signal(s, f, v);
                any = true;
                sum = sum.wrapping_add(v);
            }
        }
        if any {
            notify(s, true, sum);
        }
    }
    Err(CommandError::deferred())
}

/// `execute if|unless function`: the sources whose functions return a value passing the
/// check (non-zero for `if`, zero for `unless`); a function that does not return never
/// passes.
pub(super) fn function_condition<S: Host>(c: &CommandContext<S>, s: &mut S, positive: bool) -> Result<Vec<SourceStack<S>>> {
    let Some(ArgumentValue::Function { tag, id }) = c.get("name") else { unreachable!("function argument") };
    let functions = {
        let lib = s.functions().ok_or_else(library_missing)?;
        if *tag {
            if !lib.has_tag(id) {
                return Err(CommandError::new(tr!("arguments.function.tag.unknown", id.to_string())));
            }
            lib.tag(id)
        } else {
            vec![lib.get(id).ok_or_else(|| CommandError::new(tr!("arguments.function.unknown", id.to_string())))?]
        }
    };
    let stack = s.stack().clone();
    let body = stack.clone().without_callbacks();
    // As `return run function`: the first function that returns decides; none returning is
    // a failure (0).
    let mut values = vec![0];
    for f in &functions {
        let outcomes = run_function(c.dispatcher(), s, f, None, body.clone()).map_err(|message| {
            CommandError::new(tr!("commands.execute.function.instantiationFailure", f.id.to_string(), message))
        })?;
        if !outcomes.is_empty() {
            values = outcomes.into_iter().map(|(_, v)| v).collect();
            break;
        }
    }
    Ok(values.into_iter().filter(|v| (*v != 0) == positive).map(|_| stack.clone()).collect())
}

pub fn return_<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let root = d.root();
    d.register(
        literal("return")
            .requires(LEVEL_GAMEMASTERS)
            .then(argument("value", ArgumentType::integer()).executes(|c, s: &mut S| {
                let value = c.integer("value");
                let frame = s.stack().frame.clone();
                frame.report(true, value);
                frame.discard();
                Ok(value)
            }))
            .then(literal("fail").executes(|_, s: &mut S| {
                let frame = s.stack().frame.clone();
                frame.report(false, 0);
                frame.discard();
                Err(CommandError::quiet())
            }))
            .then(literal("run").redirect_with(root, |_, s: &mut S| {
                // `ReturnFromCommandCustomModifier`: the rest of the function is dropped and
                // the command's result becomes the return value.
                s.stack().frame.discard();
                let mut stack = s.stack().clone();
                stack.returning = true;
                Ok(stack)
            })),
    );
}

pub fn schedule<S: Host + 'static>(d: &mut Dispatcher<S>) {
    let time = argument("time", ArgumentType::time())
        .executes(|c, s: &mut S| schedule_function(c, s, true))
        .then(literal("append").executes(|c, s: &mut S| schedule_function(c, s, false)))
        .then(literal("replace").executes(|c, s: &mut S| schedule_function(c, s, true)));
    d.register(
        literal("schedule")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("function").then(argument("function", ArgumentType::Function).suggests_server(suggest_functions).then(time)))
            .then(literal("clear").then(
                argument("function", ArgumentType::ResourceLocation)
                    .suggests_server(|_, s: &S, b| {
                        let ids: Vec<String> =
                            s.timers().map(|t| t.ids().into_iter().map(str::to_owned).collect()).unwrap_or_default();
                        b.suggest_resources(ids.iter().map(String::as_str), "");
                    })
                    .executes(|c, s: &mut S| {
                        let id = c.identifier("function").to_string();
                        let removed = s.timers_mut().map_or(0, |t| t.remove(&id));
                        if removed == 0 {
                            return Err(CommandError::new(tr!("commands.schedule.cleared.failure", id)));
                        }
                        s.send_success(tr!("commands.schedule.cleared.success", removed as i32, id), true);
                        Ok(removed as i32)
                    }),
            )),
    );
}

/// `ScheduleCommand.schedule`.
fn schedule_function<S: Host>(c: &CommandContext<S>, s: &mut S, replace: bool) -> Result<i32> {
    let time = c.time("time");
    let Some(ArgumentValue::Function { tag, id }) = c.get("function") else { unreachable!("function argument") };
    let (key, callback) = {
        let lib = s.functions().ok_or_else(library_missing)?;
        if *tag {
            (format!("#{id}"), TimerCallback::Tag(id.clone()))
        } else {
            let f = lib.get(id).ok_or_else(|| CommandError::new(tr!("arguments.function.unknown", id.to_string())))?;
            if time == 0 {
                return Err(CommandError::new(tr!("commands.schedule.same_tick")));
            }
            if f.is_macro() {
                return Err(CommandError::new(tr!("commands.schedule.macro")));
            }
            (id.to_string(), TimerCallback::Function(id.clone()))
        }
    };
    if time == 0 {
        return Err(CommandError::new(tr!("commands.schedule.same_tick")));
    }
    let trigger = s.game_time() + i64::from(time);
    let timers = s.timers_mut().ok_or_else(|| CommandError::unsupported("Scheduled functions"))?;
    if replace {
        timers.remove(&key);
    }
    timers.schedule(&key, trigger, callback);
    let kind = if *tag { "commands.schedule.created.tag" } else { "commands.schedule.created.function" };
    s.send_success(Text::translate(kind, vec![id.to_string().into(), time.into(), trigger.into()]), true);
    Ok(trigger.rem_euclid(i64::from(i32::MAX)) as i32)
}

pub fn reload<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(literal("reload").requires(LEVEL_GAMEMASTERS).executes(|_, s: &mut S| {
        s.send_success(tr!("commands.reload.success"), true);
        s.reload_packs(None);
        Ok(0)
    }));
}

pub fn datapack<S: Host + 'static>(d: &mut Dispatcher<S>) {
    
    let enable = argument("name", ArgumentType::string())
        .suggests_server(|_, s: &S, b| suggest_packs(s, b, false))
        .executes(|c, s: &mut S| enable_pack(c, s, Insert::Default))
        .then(literal("after").then(
            argument("existing", ArgumentType::string())
                .suggests_server(|_, s: &S, b| suggest_packs(s, b, true))
                .executes(|c, s: &mut S| enable_pack(c, s, Insert::After)),
        ))
        .then(literal("before").then(
            argument("existing", ArgumentType::string())
                .suggests_server(|_, s: &S, b| suggest_packs(s, b, true))
                .executes(|c, s: &mut S| enable_pack(c, s, Insert::Before)),
        ))
        .then(literal("last").executes(|c, s: &mut S| enable_pack(c, s, Insert::Last)))
        .then(literal("first").executes(|c, s: &mut S| enable_pack(c, s, Insert::First)));
    d.register(
        literal("datapack")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("enable").then(enable))
            .then(literal("disable").then(
                argument("name", ArgumentType::string())
                    .suggests_server(|_, s: &S, b| suggest_packs(s, b, true))
                    .executes(|c, s: &mut S| disable_pack(c, s)),
            ))
            .then(
                literal("list")
                    .executes(|_, s: &mut S| Ok(list_enabled(s)? + list_available(s)?))
                    .then(literal("available").executes(|_, s: &mut S| list_available(s)))
                    .then(literal("enabled").executes(|_, s: &mut S| list_enabled(s))),
            )
            .then(literal("create").requires(LEVEL_OWNERS).then(
                argument("id", ArgumentType::string()).then(argument("description", ArgumentType::Component).executes(
                    |c, s: &mut S| {
                        let id = c.string("id").to_owned();
                        let description = super::scoreboard::resolved(c, s, "description")?;
                        s.create_pack(&id, &description)?;
                        s.send_success(tr!("commands.datapack.create.success", id), true);
                        Ok(1)
                    },
                )),
            )),
    );
}

fn suggest_packs<S: Host>(s: &S, b: &mut crate::suggestion::SuggestionsBuilder, selected: bool) {
    let Some(packs) = s.data_packs() else { return };
    let ids: Vec<String> = packs
        .available
        .iter()
        .filter(|p| packs.is_selected(&p.id) == selected)
        .map(|p| format!("\"{}\"", p.id.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    b.suggest_matching(ids.iter().map(String::as_str));
}

fn packs<S: Host>(s: &mut S) -> Result<DataPacks> {
    s.refresh_packs();
    s.data_packs().ok_or_else(|| CommandError::unsupported("Data packs"))
}

/// `DataPackCommand.getPack`.
fn get_pack(packs: &DataPacks, name: &str, enable: bool) -> Result<()> {
    let Some(pack) = packs.get(name) else {
        return Err(CommandError::new(tr!("commands.datapack.unknown", name)));
    };
    let selected = packs.is_selected(name);
    if enable && selected {
        return Err(CommandError::new(tr!("commands.datapack.enable.failed", name)));
    }
    if !enable && !selected {
        return Err(CommandError::new(tr!("commands.datapack.disable.failed", name)));
    }
    if !enable && !pack.required_features.is_empty() && pack.source == PackSource::Feature {
        return Err(CommandError::new(tr!("commands.datapack.disable.failed.feature", name)));
    }
    let missing: Vec<&str> =
        pack.required_features.iter().filter(|f| !packs.features.contains(f)).map(String::as_str).collect();
    if !missing.is_empty() {
        return Err(CommandError::new(tr!("commands.datapack.enable.failed.no_flags", name, missing.join(", "))));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Insert {
    Default,
    First,
    Last,
    Before,
    After,
}

fn enable_pack<S: Host>(c: &CommandContext<S>, s: &mut S, insert: Insert) -> Result<i32> {
    let name = c.string("name");
    let packs = packs(s)?;
    get_pack(&packs, name, true)?;
    let mut list = packs.selected.clone();
    match insert {
        Insert::Default | Insert::Last => list.push(name.to_owned()),
        Insert::First => list.insert(0, name.to_owned()),
        Insert::Before | Insert::After => {
            let existing = c.string("existing");
            get_pack(&packs, existing, false)?;
            let i = list.iter().position(|p| p == existing).expect("selected");
            list.insert(if matches!(insert, Insert::After) { i + 1 } else { i }, name.to_owned());
        }
    }
    let link = packs.get(name).expect("known").chat_link(true);
    s.send_success(tr!("commands.datapack.modify.enable", link), true);
    let n = list.len() as i32;
    s.reload_packs(Some(list));
    Ok(n)
}

fn disable_pack<S: Host>(c: &CommandContext<S>, s: &mut S) -> Result<i32> {
    let name = c.string("name");
    let packs = packs(s)?;
    get_pack(&packs, name, false)?;
    let list: Vec<String> = packs.selected.iter().filter(|p| *p != name).cloned().collect();
    let link = packs.get(name).expect("known").chat_link(true);
    s.send_success(tr!("commands.datapack.modify.disable", link), true);
    let n = list.len() as i32;
    s.reload_packs(Some(list));
    Ok(n)
}

fn list_enabled<S: Host>(s: &mut S) -> Result<i32> {
    let packs = packs(s)?;
    let enabled = packs.selected_packs();
    let n = enabled.len() as i32;
    let text = if enabled.is_empty() {
        tr!("commands.datapack.list.enabled.none")
    } else {
        tr!("commands.datapack.list.enabled.success", n, Text::join(enabled.iter().map(|p| p.chat_link(true))))
    };
    s.send_success(text, false);
    Ok(n)
}

fn list_available<S: Host>(s: &mut S) -> Result<i32> {
    let packs = packs(s)?;
    let available: Vec<_> = packs
        .available
        .iter()
        .filter(|p| !packs.is_selected(&p.id) && p.required_features.iter().all(|f| packs.features.contains(f)))
        .collect();
    let n = available.len() as i32;
    let text = if available.is_empty() {
        tr!("commands.datapack.list.available.none")
    } else {
        tr!("commands.datapack.list.available.success", n, Text::join(available.iter().map(|p| p.chat_link(false))))
    };
    s.send_success(text, false);
    Ok(n)
}
