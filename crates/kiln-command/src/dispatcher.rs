//! The command tree and Brigadier's algorithms: parsing with backtracking over literal and
//! argument children, execution through redirects (with redirect modifiers and forks, run the
//! way 26.3's queued execution runs them), completion and usage strings, plus the
//! per-permission-level commands packet.

use crate::arguments::{ArgumentType, ArgumentValue, GameProfileArg, MessageArg};
use crate::coords::Coordinates;
use crate::error::CommandError;
use crate::host::{Frame, Source, SourceStack};
use crate::reader::StringReader;
use crate::selector::{EntitySelector, SELECTOR_PERMISSION};
use crate::suggestion::{Suggestions, SuggestionsBuilder};
use crate::types::{Anchor, GameMode, Identifier, ItemInput};
use bytes::Bytes;
use kiln_proto::packets::commands as wire;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

pub type Handler<S> = Arc<dyn Fn(&CommandContext<S>, &mut S) -> Result<i32, CommandError> + Send + Sync>;
pub type SuggestFn<S> = Arc<dyn Fn(&CommandContext<S>, &S, &mut SuggestionsBuilder) + Send + Sync>;
/// Brigadier's `RedirectModifier`: the sources a redirect continues with, derived from the
/// current one (set on `S` while it runs); may be empty.
pub type Modifier<S> =
    Arc<dyn Fn(&CommandContext<S>, &mut S) -> Result<Vec<SourceStack<S>>, CommandError> + Send + Sync>;

/// Custom suggestions for an argument node.
pub enum SuggestionProvider<S: Source> {
    /// A provider the client implements, e.g. `minecraft:summonable_entities`.
    Named(&'static str),
    /// `minecraft:ask_server`: the client sends a `command_suggestion` request and the server
    /// answers with this function.
    Server(SuggestFn<S>),
}

impl<S: Source> Clone for SuggestionProvider<S> {
    fn clone(&self) -> Self {
        match self {
            SuggestionProvider::Named(n) => SuggestionProvider::Named(n),
            SuggestionProvider::Server(f) => SuggestionProvider::Server(f.clone()),
        }
    }
}

impl<S: Source> SuggestionProvider<S> {
    fn id(&self) -> &'static str {
        match self {
            SuggestionProvider::Named(n) => n,
            SuggestionProvider::Server(_) => "minecraft:ask_server",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Root,
    Literal(String),
    Argument { name: String, ty: ArgumentType },
}

impl NodeKind {
    fn name(&self) -> &str {
        match self {
            NodeKind::Root => "",
            NodeKind::Literal(l) => l,
            NodeKind::Argument { name, .. } => name,
        }
    }
}

struct Node<S: Source> {
    kind: NodeKind,
    children: Vec<NodeId>,
    command: Option<Handler<S>>,
    redirect: Option<NodeId>,
    modifier: Option<Modifier<S>>,
    forks: bool,
    /// A custom executor (`function`): runs twice per execution, see [`Builder::custom`].
    custom: bool,
    permission: u8,
    suggestions: Option<SuggestionProvider<S>>,
}

/// A node under construction, as with Brigadier's `literal(...)` and `argument(...)`.
pub struct Builder<S: Source> {
    kind: NodeKind,
    children: Vec<Builder<S>>,
    command: Option<Handler<S>>,
    redirect: Option<NodeId>,
    modifier: Option<Modifier<S>>,
    forks: bool,
    /// A custom executor (`function`): runs twice per execution, see [`Builder::custom`].
    custom: bool,
    permission: u8,
    suggestions: Option<SuggestionProvider<S>>,
}

pub fn literal<S: Source>(name: &str) -> Builder<S> {
    Builder::new(NodeKind::Literal(name.to_owned()))
}

pub fn argument<S: Source>(name: &str, ty: ArgumentType) -> Builder<S> {
    Builder::new(NodeKind::Argument { name: name.to_owned(), ty })
}

impl<S: Source> Builder<S> {
    fn new(kind: NodeKind) -> Self {
        Builder {
            kind,
            children: Vec::new(),
            command: None,
            redirect: None,
            modifier: None,
            forks: false,
            custom: false,
            permission: 0,
            suggestions: None,
        }
    }

    pub fn then(mut self, child: Builder<S>) -> Self {
        assert!(self.redirect.is_none(), "cannot add children to a redirected node");
        self.children.push(child);
        self
    }

    pub fn executes(
        mut self,
        f: impl Fn(&CommandContext<S>, &mut S) -> Result<i32, CommandError> + Send + Sync + 'static,
    ) -> Self {
        self.command = Some(Arc::new(f));
        self
    }

    /// Makes the handler a custom executor (`CustomCommandExecutor`), as `function` is:
    /// it runs for every source with [`SourceStack::preparing`] set (to report and check,
    /// as vanilla does inline), then again for every source that succeeded to do the work
    /// (what vanilla queues). It tells result callbacks itself, and costs no command quota.
    pub fn custom(mut self) -> Self {
        self.custom = true;
        self
    }

    /// Minimum permission level (0-4) to use this node and everything below it.
    pub fn requires(mut self, level: u8) -> Self {
        self.permission = level;
        self
    }

    /// Continues parsing at `target` (e.g. `tp` -> `teleport`, `execute run` -> root).
    pub fn redirect(mut self, target: NodeId) -> Self {
        assert!(self.children.is_empty(), "cannot redirect a node with children");
        self.redirect = Some(target);
        self
    }

    /// `redirect(target, SingleRedirectModifier)`: continues at `target` with the one source
    /// `f` derives (e.g. `execute positioned`).
    pub fn redirect_with(
        self,
        target: NodeId,
        f: impl Fn(&CommandContext<S>, &mut S) -> Result<SourceStack<S>, CommandError> + Send + Sync + 'static,
    ) -> Self {
        self.forward(target, Arc::new(move |c, s| f(c, s).map(|stack| vec![stack])), false)
    }

    /// `fork(target, RedirectModifier)`: continues at `target` once per source `f` returns
    /// (e.g. `execute as`); failures after a fork are not reported.
    pub fn fork(
        self,
        target: NodeId,
        f: impl Fn(&CommandContext<S>, &mut S) -> Result<Vec<SourceStack<S>>, CommandError> + Send + Sync + 'static,
    ) -> Self {
        self.forward(target, Arc::new(f), true)
    }

    /// `forward(target, modifier, fork)`.
    pub fn forward(mut self, target: NodeId, modifier: Modifier<S>, fork: bool) -> Self {
        assert!(self.children.is_empty(), "cannot redirect a node with children");
        self.redirect = Some(target);
        self.modifier = Some(modifier);
        self.forks = fork;
        self
    }

    pub fn suggests(mut self, provider: SuggestionProvider<S>) -> Self {
        debug_assert!(matches!(self.kind, NodeKind::Argument { .. }), "only arguments take suggestions");
        self.suggestions = Some(provider);
        self
    }

    /// Server-side suggestions (`minecraft:ask_server`).
    pub fn suggests_server(
        self,
        f: impl Fn(&CommandContext<S>, &S, &mut SuggestionsBuilder) + Send + Sync + 'static,
    ) -> Self {
        self.suggests(SuggestionProvider::Server(Arc::new(f)))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ParsedArgument {
    name: String,
    start: usize,
    end: usize,
    value: ArgumentValue,
}

/// Brigadier's `CommandContextBuilder`.
#[derive(Debug, Clone)]
struct ContextBuilder {
    root: NodeId,
    range: (usize, usize),
    nodes: Vec<(NodeId, usize, usize)>,
    args: Vec<ParsedArgument>,
    command: Option<NodeId>,
    child: Option<Box<ContextBuilder>>,
}

impl ContextBuilder {
    fn new(root: NodeId, start: usize) -> Self {
        ContextBuilder { root, range: (start, start), nodes: Vec::new(), args: Vec::new(), command: None, child: None }
    }

    fn with_node(&mut self, node: NodeId, start: usize, end: usize) {
        self.nodes.push((node, start, end));
        self.range = (self.range.0.min(start), self.range.1.max(end));
    }

    /// The node whose children complete the text at `cursor`, and where completion starts.
    fn find_suggestion_context(&self, cursor: usize) -> (NodeId, usize) {
        if self.range.0 > cursor {
            return (self.root, self.range.0);
        }
        if self.range.1 < cursor {
            if let Some(child) = &self.child {
                return child.find_suggestion_context(cursor);
            }
            return match self.nodes.last() {
                Some(&(node, _, end)) => (node, end + 1),
                None => (self.root, self.range.0),
            };
        }
        let mut prev = self.root;
        for &(node, start, end) in &self.nodes {
            if start <= cursor && cursor <= end {
                return (prev, start);
            }
            prev = node;
        }
        (prev, self.range.0)
    }
}

/// A parse outcome: context, reader cursor and the errors of the alternatives that failed.
type Parsed = (ContextBuilder, usize, Vec<(NodeId, CommandError)>);

/// The outcome of parsing: the deepest context reached, where the reader stopped and the
/// errors of the alternatives that failed there.
pub struct ParseResults<'a> {
    input: &'a str,
    context: ContextBuilder,
    cursor: usize,
    errors: Vec<(NodeId, CommandError)>,
}

impl ParseResults<'_> {
    pub fn input(&self) -> &str {
        self.input
    }

    /// Byte offset where parsing stopped.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn errors(&self) -> impl Iterator<Item = &CommandError> {
        self.errors.iter().map(|(_, e)| e)
    }

    /// Nodes matched in the top-level context.
    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.context.nodes.iter().map(|n| n.0)
    }
}

/// Arguments and nodes of the context a command runs in (the last one after redirects).
pub struct CommandContext<'a, S: Source> {
    dispatcher: &'a Dispatcher<S>,
    input: &'a str,
    context: &'a ContextBuilder,
}

impl<'a, S: Source> CommandContext<'a, S> {
    pub fn input(&self) -> &'a str {
        self.input
    }

    pub fn dispatcher(&self) -> &'a Dispatcher<S> {
        self.dispatcher
    }

    pub fn nodes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.context.nodes.iter().map(|n| n.0)
    }

    pub fn get(&self, name: &str) -> Option<&'a ArgumentValue> {
        self.context.args.iter().find(|a| a.name == name).map(|a| &a.value)
    }

    pub fn has(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The input text an argument was parsed from.
    pub fn arg_text(&self, name: &str) -> Option<&'a str> {
        self.context.args.iter().find(|a| a.name == name).map(|a| &self.input[a.start..a.end])
    }

    fn expect(&self, name: &str) -> &'a ArgumentValue {
        self.get(name).unwrap_or_else(|| panic!("no argument named {name}"))
    }
}

macro_rules! getters {
    ($($fn:ident: $variant:ident => $ty:ty $(, $deref:tt)?;)*) => {
        impl<'a, S: Source> CommandContext<'a, S> {
            $(
                #[doc = concat!("The `", stringify!($variant), "` argument `name`; panics if missing or of another type.")]
                pub fn $fn(&self, name: &str) -> $ty {
                    match self.expect(name) {
                        ArgumentValue::$variant(v) => $($deref)? v,
                        other => panic!("argument {name} is {other:?}"),
                    }
                }
            )*
        }
    };
}

getters! {
    bool: Bool => bool, *;
    integer: Integer => i32, *;
    long: Long => i64, *;
    float: Float => f32, *;
    double: Double => f64, *;
    string: String => &'a str;
    selector: Entity => &'a EntitySelector;
    game_profile: GameProfile => &'a GameProfileArg;
    coordinates: Coordinates => &'a Coordinates;
    item: Item => &'a ItemInput;
    message: Message => &'a MessageArg;
    identifier: Identifier => &'a Identifier;
    anchor: Anchor => Anchor, *;
    game_mode: GameMode => GameMode, *;
    time: Time => i32, *;
    block_state: BlockState => &'a crate::blocks::BlockInput;
    block_predicate: BlockPredicate => &'a crate::blocks::BlockPredicate;
    swizzle: Swizzle => [bool; 3], *;
    heightmap: Heightmap => crate::types::Heightmap, *;
    score_holder: ScoreHolder => &'a crate::arguments::ScoreHolderArg;
    operation: Operation => crate::arguments::Operation, *;
    int_range: IntRange => crate::range::IntRange, *;
    double_range: DoubleRange => crate::range::DoubleRange, *;
    nbt_path: NbtPath => &'a crate::nbt_path::NbtPath;
    resource_or_tag: ResourceOrTag => &'a crate::arguments::ResourceOrTag;
    component: Component => &'a crate::component::Component;
    nbt: Nbt => &'a kiln_proto::nbt::Tag;
    particle: Particle => &'a crate::arguments::ParticleArg;
}

/// A command tree over sources of type `S`.
pub struct Dispatcher<S: Source> {
    nodes: Vec<Node<S>>,
    parents: Vec<Option<NodeId>>,
}

impl<S: Source> Default for Dispatcher<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Source> Dispatcher<S> {
    pub fn new() -> Self {
        let root = Node {
            kind: NodeKind::Root,
            children: Vec::new(),
            command: None,
            redirect: None,
            modifier: None,
            forks: false,
            custom: false,
            permission: 0,
            suggestions: None,
        };
        Dispatcher { nodes: vec![root], parents: vec![None] }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Adds `builder` under the root, merging with an existing literal of the same name.
    pub fn register(&mut self, builder: Builder<S>) -> NodeId {
        self.insert(self.root(), builder)
    }

    fn insert(&mut self, parent: NodeId, b: Builder<S>) -> NodeId {
        let existing = self.node(parent).children.iter().copied().find(|&c| self.node(c).kind == b.kind);
        let id = match existing {
            Some(id) => {
                if b.command.is_some() {
                    self.nodes[id.0 as usize].command = b.command;
                }
                id
            }
            None => {
                let id = NodeId(self.nodes.len() as u32);
                self.nodes.push(Node {
                    kind: b.kind,
                    children: Vec::new(),
                    command: b.command,
                    redirect: b.redirect,
                    modifier: b.modifier,
                    forks: b.forks,
                    custom: b.custom,
                    permission: b.permission,
                    suggestions: b.suggestions,
                });
                self.parents.push(Some(parent));
                self.nodes[parent.0 as usize].children.push(id);
                id
            }
        };
        for child in b.children {
            self.insert(id, child);
        }
        id
    }

    fn node(&self, id: NodeId) -> &Node<S> {
        &self.nodes[id.0 as usize]
    }

    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.node(id).kind
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.node(id).children
    }

    pub fn redirect_of(&self, id: NodeId) -> Option<NodeId> {
        self.node(id).redirect
    }

    pub fn permission(&self, id: NodeId) -> u8 {
        self.node(id).permission
    }

    pub fn is_executable(&self, id: NodeId) -> bool {
        self.node(id).command.is_some()
    }

    pub fn has_custom_suggestions(&self, id: NodeId) -> Option<&'static str> {
        self.node(id).suggestions.as_ref().map(SuggestionProvider::id)
    }

    /// The node at `path` of literal/argument names below the root.
    pub fn find(&self, path: &[&str]) -> Option<NodeId> {
        let mut id = self.root();
        for name in path {
            id = self.node(id).children.iter().copied().find(|&c| self.node(c).kind.name() == *name)?;
        }
        Some(id)
    }

    /// Names from the root to `id` (`CommandDispatcher.getPath`).
    pub fn path(&self, id: NodeId) -> Vec<&str> {
        let mut path = Vec::new();
        let mut cur = id;
        while let Some(parent) = self.parents[cur.0 as usize] {
            path.push(self.node(cur).kind.name());
            cur = parent;
        }
        path.reverse();
        path
    }

    fn usage_text(&self, id: NodeId) -> String {
        match &self.node(id).kind {
            NodeKind::Root => String::new(),
            NodeKind::Literal(l) => l.clone(),
            NodeKind::Argument { name, .. } => format!("<{name}>"),
        }
    }
}

impl<S: Source> Dispatcher<S> {
    fn can_use(&self, id: NodeId, source: &S) -> bool {
        source.permission() >= self.node(id).permission
    }

    pub fn parse<'a>(&self, input: &'a str, source: &S) -> ParseResults<'a> {
        self.parse_from(input, 0, source)
    }

    /// Parses `input` starting at byte `start` (e.g. 1 to skip a leading `/`).
    pub fn parse_from<'a>(&self, input: &'a str, start: usize, source: &S) -> ParseResults<'a> {
        let mut reader = StringReader::new(input);
        reader.set_cursor(start);
        let ctx = ContextBuilder::new(self.root(), start);
        let (context, cursor, errors) = self.parse_nodes(self.root(), &reader, ctx, source);
        ParseResults { input, context, cursor, errors }
    }

    fn relevant_nodes(&self, node: NodeId, reader: &StringReader) -> Vec<NodeId> {
        let children = &self.node(node).children;
        let word = reader.remaining().split(' ').next().unwrap_or("");
        let literal =
            children.iter().copied().find(|&c| matches!(&self.node(c).kind, NodeKind::Literal(l) if l == word));
        match literal {
            Some(l) => vec![l],
            None => {
                children.iter().copied().filter(|&c| matches!(self.node(c).kind, NodeKind::Argument { .. })).collect()
            }
        }
    }

    fn parse_node(
        &self,
        id: NodeId,
        reader: &mut StringReader,
        ctx: &mut ContextBuilder,
        source: &S,
    ) -> Result<(), CommandError> {
        let start = reader.cursor();
        match &self.node(id).kind {
            NodeKind::Root => unreachable!("the root is never a child"),
            NodeKind::Literal(l) => {
                let rest = reader.remaining();
                if rest.starts_with(l.as_str()) && rest[l.len()..].chars().next().is_none_or(|c| c == ' ') {
                    reader.set_cursor(start + l.len());
                    ctx.with_node(id, start, reader.cursor());
                    Ok(())
                } else {
                    Err(CommandError::literal_incorrect(l).at(reader))
                }
            }
            NodeKind::Argument { name, ty } => {
                let value = ty.parse(reader, source.permission() >= SELECTOR_PERMISSION)?;
                // `ResourceSelectorArgument.parse`: a pattern must select something.
                if let (ArgumentType::ResourceSelector { registry }, ArgumentValue::String(pattern)) = (ty, &value)
                    && !source.registry_ids(registry).iter().any(|id| crate::types::wildcard_match(id, pattern))
                {
                    return Err(CommandError::new(crate::tr!("argument.resource_selector.not_found", pattern.as_str(), *registry)).at(reader));
                }
                // Inline definitions are decoded while parsing, as `ResourceOrIdArgument` does.
                let inline_registry = match ty {
                    ArgumentType::LootResource { registry } => Some(*registry),
                    ArgumentType::ContextProvider { float: true } => Some("minecraft:context_float_provider"),
                    ArgumentType::ContextProvider { float: false } => Some("minecraft:context_int_provider"),
                    ArgumentType::SlotSource => Some("minecraft:slot_source"),
                    _ => None,
                };
                if let Some(registry) = inline_registry {
                    let definition = match &value {
                        ArgumentValue::Nbt(tag) => Some(tag.clone()),
                        ArgumentValue::String(text) if text.starts_with(['{', '[', '"', '\'']) => {
                            crate::snbt::parse_tag(&mut StringReader::new(text)).ok()
                        }
                        _ => None,
                    };
                    match definition {
                        // A quoted string names an entry of a registry that takes references.
                        Some(kiln_proto::nbt::Tag::String(id)) if matches!(registry, "minecraft:item_modifier" | "minecraft:slot_source") => {
                            let id = Identifier::parse(&id).map_or(id, |i| i.to_string());
                            let ids = source.registry_ids(registry);
                            if !ids.is_empty() && !ids.iter().any(|r| *r == id) {
                                return Err(CommandError::new(crate::tr!("argument.resource_or_id.no_such_element", id, registry)).at(reader));
                            }
                        }
                        Some(d) => {
                            if let Some(message) = source.definition_error(registry, &d) {
                                return Err(CommandError::new(crate::tr!("argument.resource_or_id.failed_to_parse", message)).at(reader));
                            }
                        }
                        None => {}
                    }
                }
                // A loot predicate is an id of `predicate/` or an inline condition.
                if let ArgumentType::LootPredicate = ty {
                    match &value {
                        ArgumentValue::Identifier(id) => {
                            let ids = source.registry_ids("minecraft:predicate");
                            if !ids.iter().any(|r| r == id.as_str()) {
                                let e = crate::tr!("argument.resource_or_id.no_such_element", id.to_string(), "minecraft:predicate");
                                return Err(CommandError::new(e).at(reader));
                            }
                        }
                        ArgumentValue::Nbt(d) => {
                            if let Some(message) = source.definition_error("minecraft:predicate", d) {
                                return Err(CommandError::new(crate::tr!("argument.resource_or_id.failed_to_parse", message)).at(reader));
                            }
                        }
                        _ => {}
                    }
                }
                // Loot registry ids are looked up while parsing, as `ResourceOrIdArgument` does.
                if let (ArgumentType::LootResource { registry }, ArgumentValue::Identifier(id)) = (ty, &value)
                    && let ids = source.registry_ids(registry)
                    && !ids.is_empty()
                    && !ids.iter().any(|r| r == id.as_str())
                {
                    return Err(CommandError::new(crate::tr!("argument.resource_or_id.no_such_element", id.to_string(), *registry)).at(reader));
                }
                if let (ArgumentType::ContextProvider { float }, ArgumentValue::Identifier(id)) = (ty, &value) {
                    let registry = if *float { "minecraft:context_float_provider" } else { "minecraft:context_int_provider" };
                    if !source.registry_ids(registry).iter().any(|r| r == id.as_str()) {
                        return Err(CommandError::new(crate::tr!("argument.resource_or_id.no_such_element", id.to_string(), registry)).at(reader));
                    }
                }
                if let (ArgumentType::SlotSource, ArgumentValue::String(text)) = (ty, &value)
                    && crate::slots::by_name(text).is_none()
                    && !text.starts_with(['{', '['])
                    && let ids = source.registry_ids("minecraft:slot_source")
                    && !ids.is_empty()
                {
                    let id = Identifier::parse(text).map_or_else(|| text.clone(), |i| i.to_string());
                    if !ids.iter().any(|r| *r == id) {
                        return Err(CommandError::new(crate::tr!("argument.resource_or_id.no_such_element", id, "minecraft:slot_source")).at(reader));
                    }
                }
                let end = reader.cursor();
                ctx.args.push(ParsedArgument { name: name.clone(), start, end, value });
                ctx.with_node(id, start, end);
                Ok(())
            }
        }
    }

    fn parse_nodes(&self, node: NodeId, reader: &StringReader, ctx: ContextBuilder, source: &S) -> Parsed {
        let mut errors = Vec::new();
        let mut potentials: Vec<Parsed> = Vec::new();
        for child in self.relevant_nodes(node, reader) {
            if !self.can_use(child, source) {
                continue;
            }
            let mut context = ctx.clone();
            let mut r = reader.clone();
            let parsed = self.parse_node(child, &mut r, &mut context, source).and_then(|()| {
                if r.can_read() && r.peek() != ' ' { Err(CommandError::expected_separator().at(&r)) } else { Ok(()) }
            });
            if let Err(e) = parsed {
                errors.push((child, e));
                continue;
            }
            let n = self.node(child);
            context.command = n.command.is_some().then_some(child);
            if r.can_read_n(if n.redirect.is_none() { 2 } else { 1 }) {
                r.skip();
                if let Some(target) = n.redirect {
                    let child_ctx = ContextBuilder::new(target, r.cursor());
                    let (parsed, cursor, errs) = self.parse_nodes(target, &r, child_ctx, source);
                    context.child = Some(Box::new(parsed));
                    return (context, cursor, errs);
                }
                potentials.push(self.parse_nodes(child, &r, context, source));
            } else {
                potentials.push((context, r.cursor(), Vec::new()));
            }
        }
        if potentials.is_empty() {
            return (ctx, reader.cursor(), errors);
        }
        let len = reader.total_len();
        potentials.sort_by_key(|(_, cursor, errs)| (*cursor < len, !errs.is_empty()));
        potentials.swap_remove(0)
    }

    /// Parses and runs `input` (without a leading `/`).
    pub fn execute(&self, input: &str, source: &mut S) -> Result<i32, CommandError> {
        let parse = self.parse(input, source);
        self.execute_parsed(&parse, source)
    }

    /// Runs a parse result. Parse errors carry the input and cursor; errors raised by the
    /// command itself do not (vanilla shows only their message).
    ///
    /// Returns the error to show the source, if any, or the sum of the results of every
    /// execution (the source's stack is restored afterwards). After a fork, failures are
    /// silent, as vanilla handles them.
    pub fn execute_parsed(&self, parse: &ParseResults, source: &mut S) -> Result<i32, CommandError> {
        // A top-level command: its own frame and command quota.
        let original = source.stack().clone();
        let limit = source.command_limit().max(1);
        let stack = source.stack_mut();
        stack.frame = Frame::new(0);
        stack.quota = Arc::new(AtomicI32::new(limit));
        let result = self.execute_in_frame(parse, source);
        *source.stack_mut() = original;
        result
    }

    /// Parses and runs `input` in the current frame and command quota (a function's line).
    /// The source's stack is restored afterwards.
    pub fn run_nested(&self, input: &str, source: &mut S) -> Result<i32, CommandError> {
        let parse = self.parse(input, source);
        self.execute_in_frame(&parse, source)
    }

    /// `Commands.validateParseResults` plus `ContextChain.tryFlatten`: the error a parse
    /// result gives before anything runs, if any.
    pub fn check_parse(&self, parse: &ParseResults) -> Result<(), CommandError> {
        self.parsed_chain(parse).map(|_| ())
    }

    fn parsed_chain<'p>(&self, parse: &'p ParseResults) -> Result<(Vec<&'p ContextBuilder>, NodeId), CommandError> {
        if parse.cursor < parse.input.len() {
            if let [(_, e)] = parse.errors.as_slice() {
                return Err(e.clone());
            }
            let e = if parse.context.range.0 == parse.context.range.1 {
                CommandError::unknown_command()
            } else {
                CommandError::unknown_argument()
            };
            return Err(e.with_context(parse.input, parse.cursor));
        }
        let mut chain = vec![&parse.context];
        while let Some(child) = &chain[chain.len() - 1].child {
            chain.push(child);
        }
        let Some(command) = chain[chain.len() - 1].command else {
            return Err(CommandError::unknown_command().with_context(parse.input, parse.cursor));
        };
        Ok((chain, command))
    }

    fn execute_in_frame(&self, parse: &ParseResults, source: &mut S) -> Result<i32, CommandError> {
        let (chain, command) = self.parsed_chain(parse)?;
        let original = source.stack().clone();
        let result = self.run_chain(&chain, command, parse.input, source);
        *source.stack_mut() = original;
        result
    }

    /// `BuildContexts.execute` then one `ExecuteCommand` per source: every modifier stage runs
    /// for all current sources (at most `max_command_forks` per stage), then the command runs
    /// once per resulting source while `max_command_sequence_length` allows, telling each
    /// source's result callbacks the outcome.
    fn run_chain(
        &self,
        chain: &[&ContextBuilder],
        command: NodeId,
        input: &str,
        source: &mut S,
    ) -> Result<i32, CommandError> {
        let fork_limit = source.fork_limit();
        let quota = source.stack().quota.clone();
        let frame = source.stack().frame.clone();
        let mut forked = false;
        let mut returning = false;
        let mut sources = vec![source.stack().clone()];
        for &ctx in &chain[..chain.len() - 1] {
            let Some(&(last, _, _)) = ctx.nodes.last() else { continue };
            let node = self.node(last);
            forked |= node.forks;
            let Some(modifier) = node.modifier.clone() else { continue };
            returning |= sources.iter().any(|s| s.returning);
            quota.fetch_sub(1, Ordering::Relaxed);
            let cctx = CommandContext { dispatcher: self, input, context: ctx };
            let mut next = Vec::new();
            for stack in sources {
                *source.stack_mut() = stack;
                match modifier(&cctx, source) {
                    Ok(found) => {
                        if next.len() + found.len() >= fork_limit {
                            return if forked { Ok(0) } else { Err(CommandError::fork_limit(fork_limit)) };
                        }
                        next.extend(found);
                    }
                    Err(_) if forked => {}
                    Err(e) => return Err(e.without_context()),
                }
            }
            sources = next;
        }
        returning |= sources.iter().any(|s| s.returning);
        if returning {
            // `return run`: the first source's result is the frame's return value; with no
            // source left the frame returns a failure (`FallthroughTask`).
            if sources.is_empty() {
                frame.report(false, 0);
                return Ok(0);
            }
            sources.truncate(1);
        }
        let handler = self.node(command).command.clone().expect("command node has a handler");
        let ctx = CommandContext { dispatcher: self, input, context: chain[chain.len() - 1] };
        if self.node(command).custom {
            let mut ready = Vec::new();
            for mut stack in sources {
                stack.preparing = true;
                *source.stack_mut() = stack.clone();
                match handler(&ctx, source) {
                    Ok(_) => ready.push(stack),
                    Err(e) if e.is_deferred() => ready.push(stack),
                    Err(_) if forked => {}
                    Err(e) => return Err(e.without_context()),
                }
            }
            for mut stack in ready {
                stack.preparing = false;
                *source.stack_mut() = stack;
                match handler(&ctx, source) {
                    Ok(_) => {}
                    Err(e) if e.is_deferred() || forked => {}
                    Err(e) => return Err(e.without_context()),
                }
            }
            return Ok(0);
        }
        let mut total = 0i32;
        for stack in sources {
            if quota.load(Ordering::Relaxed) <= 0 {
                break;
            }
            quota.fetch_sub(1, Ordering::Relaxed);
            *source.stack_mut() = stack;
            let result = handler(&ctx, source);
            if matches!(&result, Err(e) if e.is_deferred()) {
                continue;
            }
            let (success, value) = match &result {
                Ok(v) => (true, *v),
                Err(_) => (false, 0),
            };
            let callbacks = source.stack().callbacks.clone();
            for callback in &callbacks {
                callback(source, success, value);
            }
            if returning {
                frame.report(success, value);
            }
            match result {
                Ok(v) => total = total.wrapping_add(v),
                Err(_) if forked => {}
                Err(e) => return Err(e.without_context()),
            }
        }
        Ok(total)
    }

    /// Completions at byte `cursor` of `input` (which is parsed in full, as Brigadier does).
    pub fn suggestions(&self, input: &str, cursor: usize, source: &S) -> Suggestions {
        let parse = self.parse(input, source);
        self.completion_suggestions(&parse, cursor, source)
    }

    /// `getCompletionSuggestions`: completions at byte `cursor` of the parsed input.
    pub fn completion_suggestions(&self, parse: &ParseResults, cursor: usize, source: &S) -> Suggestions {
        let mut cursor = cursor.min(parse.input.len());
        while !parse.input.is_char_boundary(cursor) {
            cursor -= 1;
        }
        let (parent, start) = parse.context.find_suggestion_context(cursor);
        let start = start.min(cursor);
        let truncated = &parse.input[..cursor];
        let ctx = CommandContext { dispatcher: self, input: truncated, context: &parse.context };
        let mut all = Vec::new();
        for &child in &self.node(parent).children {
            if !self.can_use(child, source) {
                continue;
            }
            let mut b = SuggestionsBuilder::new(truncated, start);
            let n = self.node(child);
            match (&n.kind, &n.suggestions) {
                (NodeKind::Literal(l), _) => {
                    if l.to_lowercase().starts_with(b.remaining_lowercase()) {
                        b.suggest(l.clone());
                    }
                }
                (NodeKind::Argument { .. }, Some(SuggestionProvider::Server(f))) => f(&ctx, source, &mut b),
                (NodeKind::Argument { ty, .. }, _) => ty.suggest(&mut b, source),
                (NodeKind::Root, _) => {}
            }
            all.push(b.build());
        }
        Suggestions::merge(parse.input, all)
    }

    /// Completes `text` (the input up to the cursor, with or without its leading `/`) the way
    /// the server answers a `command_suggestion` request.
    pub fn complete(&self, text: &str, source: &S) -> Suggestions {
        let start = usize::from(text.starts_with('/'));
        let parse = self.parse_from(text, start, source);
        self.completion_suggestions(&parse, text.len(), source)
    }

    /// The `command_suggestions` packet answering request `id` for `text`.
    pub fn suggestions_packet(&self, id: i32, text: &str, source: &S) -> Bytes {
        self.complete(text, source).to_packet(id, text)
    }

    /// `getSmartUsage`: one compact usage line per usable child of `node`.
    pub fn smart_usage(&self, node: NodeId, source: &S) -> Vec<(NodeId, String)> {
        let optional = self.node(node).command.is_some();
        self.node(node)
            .children
            .iter()
            .filter_map(|&c| self.smart_usage_of(c, source, optional, false).map(|u| (c, u)))
            .collect()
    }

    fn smart_usage_of(&self, id: NodeId, source: &S, optional: bool, deep: bool) -> Option<String> {
        if !self.can_use(id, source) {
            return None;
        }
        let n = self.node(id);
        let this = if optional { format!("[{}]", self.usage_text(id)) } else { self.usage_text(id) };
        let child_optional = n.command.is_some();
        let (open, close) = if child_optional { ("[", "]") } else { ("(", ")") };
        if deep {
            return Some(this);
        }
        if let Some(target) = n.redirect {
            let redirect =
                if target == self.root() { "...".to_owned() } else { format!("-> {}", self.usage_text(target)) };
            return Some(format!("{this} {redirect}"));
        }
        let children: Vec<NodeId> = n.children.iter().copied().filter(|&c| self.can_use(c, source)).collect();
        match children.len() {
            0 => {}
            1 => {
                if let Some(usage) = self.smart_usage_of(children[0], source, child_optional, child_optional) {
                    return Some(format!("{this} {usage}"));
                }
            }
            _ => {
                let mut usages: Vec<String> = Vec::new();
                for &c in &children {
                    if let Some(u) = self.smart_usage_of(c, source, child_optional, true)
                        && !usages.contains(&u)
                    {
                        usages.push(u);
                    }
                }
                if usages.len() == 1 {
                    let u = &usages[0];
                    return Some(if child_optional { format!("{this} [{u}]") } else { format!("{this} {u}") });
                }
                if usages.len() > 1 {
                    let alternatives: Vec<String> = children.iter().map(|&c| self.usage_text(c)).collect();
                    return Some(format!("{this} {open}{}{close}", alternatives.join("|")));
                }
            }
        }
        Some(this)
    }

    /// `getAllUsage`: every executable path below `node`, one line each.
    pub fn all_usage(&self, node: NodeId, source: &S, restricted: bool) -> Vec<String> {
        let mut out = Vec::new();
        self.all_usage_of(node, source, &mut out, String::new(), restricted);
        out
    }

    fn all_usage_of(&self, id: NodeId, source: &S, out: &mut Vec<String>, prefix: String, restricted: bool) {
        if restricted && !self.can_use(id, source) {
            return;
        }
        let n = self.node(id);
        if n.command.is_some() {
            out.push(prefix.clone());
        }
        if let Some(target) = n.redirect {
            let redirect =
                if target == self.root() { "...".to_owned() } else { format!("-> {}", self.usage_text(target)) };
            out.push(if prefix.is_empty() {
                format!("{} {redirect}", self.usage_text(id))
            } else {
                format!("{prefix} {redirect}")
            });
        } else {
            for &c in &n.children {
                let p = if prefix.is_empty() { self.usage_text(c) } else { format!("{prefix} {}", self.usage_text(c)) };
                self.all_usage_of(c, source, out, p, restricted);
            }
        }
    }
}

impl<S: Source> Dispatcher<S> {
    /// The `commands` packet for a player at permission `level`: the nodes that level may use
    /// (`Commands.sendCommands`), numbered breadth-first like `ClientboundCommandsPacket`.
    pub fn commands_packet(&self, level: u8) -> Bytes {
        let usable: Vec<bool> = {
            let mut ok = vec![false; self.nodes.len()];
            ok[0] = true;
            let mut stack = vec![self.root()];
            while let Some(id) = stack.pop() {
                for &c in &self.node(id).children {
                    if self.node(c).permission <= level {
                        ok[c.0 as usize] = true;
                        stack.push(c);
                    }
                }
            }
            ok
        };
        let mut index: Vec<Option<i32>> = vec![None; self.nodes.len()];
        let mut order = Vec::new();
        let mut queue = std::collections::VecDeque::from([self.root()]);
        while let Some(id) = queue.pop_front() {
            if index[id.0 as usize].is_some() {
                continue;
            }
            index[id.0 as usize] = Some(order.len() as i32);
            order.push(id);
            let n = self.node(id);
            queue.extend(n.children.iter().copied().filter(|c| usable[c.0 as usize]));
            if let Some(r) = n.redirect.filter(|r| usable[r.0 as usize]) {
                queue.push_back(r);
            }
        }
        let nodes: Vec<wire::Node> = order
            .iter()
            .map(|&id| {
                let n = self.node(id);
                let kind = match &n.kind {
                    NodeKind::Root => wire::NodeKind::Root,
                    NodeKind::Literal(l) => wire::NodeKind::Literal(l),
                    NodeKind::Argument { name, ty } => wire::NodeKind::Argument {
                        name,
                        parser: ty.wire(),
                        suggestions: n.suggestions.as_ref().map(SuggestionProvider::id),
                    },
                };
                wire::Node {
                    kind,
                    executable: n.command.is_some(),
                    restricted: n.permission > 0,
                    children: n.children.iter().filter_map(|c| index[c.0 as usize]).collect(),
                    redirect: n.redirect.and_then(|r| index[r.0 as usize]),
                }
            })
            .collect();
        wire::commands(&nodes, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Src {
        level: u8,
        log: Vec<String>,
        stack: SourceStack<Src>,
        forks: usize,
        limit: i32,
    }

    impl Source for Src {
        type Entity = crate::selector::NoEntity;
        fn permission_level(&self) -> u8 {
            self.level
        }
        fn stack(&self) -> &SourceStack<Src> {
            &self.stack
        }
        fn stack_mut(&mut self) -> &mut SourceStack<Src> {
            &mut self.stack
        }
        fn fork_limit(&self) -> usize {
            self.forks
        }
        fn command_limit(&self) -> i32 {
            self.limit
        }
    }

    fn src(level: u8) -> Src {
        let stack = SourceStack::new(crate::text::Text::literal("Server"), "minecraft:overworld", [0.0; 3]);
        Src { level, log: Vec::new(), stack, forks: 65536, limit: 65536 }
    }

    /// `run` redirects to the root; `each <n>` forks into n sources at x = 0..n; `move <x>`
    /// moves without forking; `fail` fails; `x` reports the source's x.
    fn forking_tree() -> Dispatcher<Src> {
        let mut d: Dispatcher<Src> = Dispatcher::new();
        let exec = d.register(literal("exec"));
        d.register(
            literal("exec")
                .then(literal("run").redirect(d.root()))
                .then(argument("n", ArgumentType::integer_min(0)).fork(exec, |c, s: &mut Src| {
                    Ok((0..c.integer("n")).map(|i| s.stack().clone().with_position([i as f64, 0.0, 0.0])).collect())
                }))
                .then(literal("move").then(argument("x", ArgumentType::integer()).redirect_with(exec, |c, s: &mut Src| {
                    Ok(s.stack().clone().with_position([c.integer("x") as f64, 0.0, 0.0]))
                })))
                .then(literal("bad").redirect_with(exec, |_, _| Err(CommandError::unknown_argument())))
                .then(literal("store").redirect_with(exec, |_, s: &mut Src| {
                    Ok(s.stack().clone().with_callback(Arc::new(|s: &mut Src, ok, v| s.log.push(format!("store {ok} {v}")))))
                })),
        );
        d.register(literal("x").executes(|_, s: &mut Src| {
            let x = s.stack().position[0] as i32;
            s.log.push(format!("x={x}"));
            Ok(x)
        }));
        d.register(literal("fail").executes(|_, _| Err(CommandError::no_entities_found())));
        d
    }

    #[test]
    fn forks_and_redirect_modifiers() {
        let d = forking_tree();
        let s = &mut src(4);
        assert_eq!(d.execute("exec 3 run x", s), Ok(3));
        assert_eq!(s.log, ["x=0", "x=1", "x=2"]);
        assert_eq!(s.stack.position, [0.0; 3], "the stack is restored");
        s.log.clear();
        // Stages run breadth-first: each of the 2 sources forks into 2 more.
        assert_eq!(d.execute("exec 2 move 5 2 run x", s), Ok(2), "x = 0, 1, 0, 1");
        assert_eq!(d.execute("exec 0 run x", s), Ok(0), "no sources: nothing runs");
        assert_eq!(d.execute("exec move 7 run x", s), Ok(7));
        // Without a fork errors are reported; after one they are not.
        assert_eq!(d.execute("exec move 1 run fail", s).unwrap_err().key(), Some("argument.entity.notfound.entity"));
        assert_eq!(d.execute("exec 2 run fail", s), Ok(0));
        assert_eq!(d.execute("exec bad run x", s).unwrap_err().key(), Some("command.unknown.argument"));
        assert_eq!(d.execute("exec 1 bad run x", s), Ok(0));
        let e = d.execute("exec run", s).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("command.unknown.command"), Some(8)));
    }

    #[test]
    fn fork_and_command_limits() {
        let d = forking_tree();
        let s = &mut src(4);
        s.forks = 3;
        // A stage may produce fewer than max_command_forks sources.
        assert_eq!(d.execute("exec 2 run x", s), Ok(1));
        // The stage that forks is already forked: its limit error is silent.
        assert_eq!(d.execute("exec move 1 3 run x", s), Ok(0));
        // Only a non-forking stage reports it, which needs a limit of 1.
        s.forks = 1;
        assert_eq!(d.execute("exec move 1 run x", s).unwrap_err().key(), Some("command.forkLimit"));
        assert_eq!(d.execute("x", s), Ok(0), "no modifier, no limit");
        s.forks = 65536;
        // Each modifier stage and each execution costs one of max_command_sequence_length.
        s.limit = 3;
        s.log.clear();
        assert_eq!(d.execute("exec 5 run x", s), Ok(1), "x = 0, 1");
        assert_eq!(s.log, ["x=0", "x=1"]);
    }

    #[test]
    fn result_callbacks_follow_each_execution() {
        let d = forking_tree();
        let s = &mut src(4);
        d.execute("exec store 2 run x", s).unwrap();
        assert_eq!(s.log, ["x=0", "store true 0", "x=1", "store true 1"]);
        s.log.clear();
        d.execute("exec store run fail", s).unwrap_err();
        assert_eq!(s.log, ["store false 0"]);
    }

    #[test]
    fn packet_encodes_redirects() {
        let d = forking_tree();
        let p = d.commands_packet(4);
        let mut r = kiln_proto::Reader::new(&p);
        r.varint().unwrap();
        let n = r.varint().unwrap();
        let mut redirects = Vec::new();
        for i in 0..n {
            let flags = r.u8().unwrap();
            let children = r.varint().unwrap();
            for _ in 0..children {
                r.varint().unwrap();
            }
            if flags & 0x08 != 0 {
                redirects.push((i, r.varint().unwrap()));
            }
            match flags & 3 {
                1 => drop(r.string(32767).unwrap()),
                2 => {
                    r.string(32767).unwrap();
                    let parser = r.varint().unwrap();
                    if kiln_data::builtin_entries("minecraft:command_argument_type").unwrap()[parser as usize]
                        == "brigadier:integer"
                    {
                        let f = r.u8().unwrap();
                        r.bytes(4 * (f & 1) as usize + 4 * ((f >> 1) & 1) as usize).unwrap();
                    }
                }
                _ => {}
            }
        }
        // run -> root, <n> -> exec, <x> -> exec, bad -> exec, store -> exec.
        assert_eq!(redirects.len(), 5);
        assert!(redirects.iter().any(|&(_, target)| target == 0), "run redirects to the root");
        assert_eq!(redirects.iter().filter(|&&(_, target)| target == 1).count(), 4, "the rest to exec (index 1)");
    }

    fn tree() -> Dispatcher<Src> {
        let mut d = Dispatcher::new();
        let base = d.register(
            literal("base")
                .then(literal("foo").executes(|_, s: &mut Src| {
                    s.log.push("foo".into());
                    Ok(1)
                }))
                .then(argument("n", ArgumentType::integer_range(0, 10)).executes(|c, s: &mut Src| {
                    s.log.push(format!("n={}", c.integer("n")));
                    Ok(c.integer("n"))
                }))
                .then(argument("word", ArgumentType::word()).then(
                    argument("rest", ArgumentType::greedy_string()).executes(|c, s: &mut Src| {
                        s.log.push(format!("{}/{}", c.string("word"), c.string("rest")));
                        Ok(2)
                    }),
                )),
        );
        d.register(literal("alias").redirect(base));
        d.register(literal("admin").requires(3).executes(|_, _| Ok(3)));
        d.register(literal("base").then(literal("bar").executes(|_, _| Ok(4))));
        d
    }

    fn key(r: Result<i32, CommandError>) -> (String, Option<usize>) {
        let e = r.unwrap_err();
        (e.key().unwrap().to_owned(), e.cursor())
    }

    #[test]
    fn dispatch_and_backtracking() {
        let d = tree();
        let s = &mut src(0);
        assert_eq!(d.execute("base foo", s), Ok(1));
        assert_eq!(d.execute("base 7", s), Ok(7));
        // "foox" is not the literal "foo", so the word argument takes it.
        assert_eq!(d.execute("base foox and more", s), Ok(2));
        assert_eq!(d.execute("base bar", s), Ok(4), "registered later and merged");
        assert_eq!(d.execute("alias 3", s), Ok(3), "redirect");
        assert_eq!(s.log, ["foo", "n=7", "foox/and more", "n=3"]);
    }

    #[test]
    fn errors() {
        let d = tree();
        let s = &mut src(0);
        assert_eq!(key(d.execute("nope", s)), ("command.unknown.command".into(), Some(0)));
        assert_eq!(key(d.execute("base", s)), ("command.unknown.command".into(), Some(4)));
        assert_eq!(key(d.execute("base ", s)), ("command.unknown.argument".into(), Some(4)));
        // 11 is out of range for <n> but a valid <word>; parsing then stops without <rest>.
        assert_eq!(key(d.execute("base 11", s)), ("command.unknown.command".into(), Some(7)));
        assert_eq!(key(d.execute("base foo extra", s)), ("command.unknown.argument".into(), Some(9)));
        assert_eq!(key(d.execute("admin", s)), ("command.unknown.command".into(), Some(0)), "hidden below level 3");
        assert_eq!(d.execute("admin", &mut src(3)), Ok(3));
    }

    #[test]
    fn single_error_is_reported() {
        let mut d: Dispatcher<Src> = Dispatcher::new();
        d.register(literal("n").then(argument("v", ArgumentType::integer_min(5)).executes(|_, _| Ok(0))));
        let e = d.execute("n 3", &mut src(0)).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("argument.integer.low"), Some(2)));
        let e = d.execute("n 3.5", &mut src(0)).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("parsing.int.invalid"), Some(2)));
        let e = d.execute("n 6x", &mut src(0)).unwrap_err();
        assert_eq!((e.key(), e.cursor()), (Some("command.expected.separator"), Some(3)));
    }

    #[test]
    fn handler_errors_lose_context() {
        let mut d: Dispatcher<Src> = Dispatcher::new();
        d.register(literal("fail").executes(|_, _| Err(CommandError::no_players_found().with_context("fail", 0))));
        let e = d.execute("fail", &mut src(0)).unwrap_err();
        assert_eq!(e.cursor(), None);
        assert_eq!(e.chat_lines("fail").len(), 1);
    }

    #[test]
    fn suggestions() {
        let d = tree();
        let s = &src(0);
        assert_eq!(d.complete("/", s).texts(), ["alias", "base"]);
        assert_eq!(d.complete("/", &src(4)).texts(), ["admin", "alias", "base"]);
        assert_eq!(d.complete("/ba", s).texts(), ["base"]);
        let sug = d.complete("/base ", s);
        assert_eq!((sug.start, sug.texts()), (6, vec!["bar", "foo"]));
        assert_eq!(d.complete("/base f", s).texts(), ["foo"]);
        assert_eq!(d.complete("/alias f", s).texts(), ["foo"]);
        assert_eq!(d.complete("base b", s).texts(), ["bar"]);
        // Mid-input cursor: completes the first word even though more follows.
        let sug = d.suggestions("ba foo", 2, s);
        assert_eq!((sug.start, sug.end, sug.texts()), (0, 2, vec!["base"]));
        assert_eq!(d.suggestions("base foo", 7, s).texts(), ["foo"]);
    }

    #[test]
    fn usage() {
        let d = tree();
        let s = &src(0);
        let base = d.find(&["base"]).unwrap();
        let usage: Vec<String> = d.smart_usage(base, s).into_iter().map(|u| u.1).collect();
        assert_eq!(usage, ["foo", "<n>", "<word> <rest>", "bar"]);
        let root: Vec<String> = d.smart_usage(d.root(), s).into_iter().map(|u| u.1).collect();
        assert_eq!(root, ["base (foo|<n>|<word>|bar)", "alias -> base"]);
        assert_eq!(d.all_usage(base, s, true), ["foo", "<n>", "<word> <rest>", "bar"]);
        assert_eq!(d.path(d.find(&["base", "word", "rest"]).unwrap()), ["base", "word", "rest"]);
    }

    #[test]
    fn packet_filters_by_level() {
        let d = tree();
        let count = |level| {
            let p = d.commands_packet(level);
            let mut r = kiln_proto::Reader::new(&p);
            r.varint().unwrap();
            r.varint().unwrap()
        };
        assert_eq!(count(0), 8);
        assert_eq!(count(3), 9);
    }
}
