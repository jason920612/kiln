//! Data packs (`PackRepository`): the built-in `vanilla` pack (the generated data Kiln loads
//! recipes and loot from), the feature packs, and folder packs in `<world>/datapacks` (or
//! `KILN_DATAPACKS` without a world). Enabled packs provide functions and function tags
//! (`ServerFunctionLibrary`), recipes and loot tables; `/reload` and `/datapack` reload them.
//! `#minecraft:load` runs on the tick after a reload, `#minecraft:tick` every tick, then the
//! scheduled functions that are due.

use crate::Sim;
use kiln_command::functions::{
    CommandFunction, DataPacks, FunctionLibrary, PackInfo, PackSource, TimerCallback, TimerQueue,
};
use kiln_command::{Identifier, SourceStack, Text};
use kiln_loot::Json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};

/// Feature packs built into the server jar, each needing its feature flag.
const FEATURE_PACKS: [&str; 3] = ["minecart_improvements", "redstone_experiments", "trade_rebalance"];
const LOAD_TAG: &str = "minecraft:load";
const TICK_TAG: &str = "minecraft:tick";

pub(crate) struct Pack {
    pub info: PackInfo,
    /// The directory holding `data/`, for packs Kiln can read.
    root: Option<PathBuf>,
}

pub(crate) struct Packs {
    /// Folder packs live here.
    dir: Option<PathBuf>,
    vanilla: PathBuf,
    pub available: Vec<Pack>,
    pub selected: Vec<String>,
    pub disabled: Vec<String>,
    pub library: FunctionLibrary,
    pub timers: TimerQueue,
    /// `#minecraft:load` runs on the next tick (`ServerFunctionManager.postReload`).
    pub load_pending: bool,
    /// The enabled/disabled lists changed since `level.dat` was written.
    pub dirty: bool,
}

impl Packs {
    /// The packs of a world whose `level.dat` saved `saved` (enabled, disabled); new folder
    /// packs are enabled (`Found new data pack ..., loading it automatically`).
    pub fn new(dir: Option<PathBuf>, vanilla: PathBuf, saved: Option<(Vec<String>, Vec<String>)>) -> Self {
        let mut packs = Packs {
            dir,
            vanilla,
            available: Vec::new(),
            selected: Vec::new(),
            disabled: Vec::new(),
            library: FunctionLibrary::default(),
            timers: TimerQueue::default(),
            load_pending: true,
            dirty: false,
        };
        packs.discover();
        let (enabled, disabled) =
            saved.unwrap_or_else(|| (vec!["vanilla".into()], FEATURE_PACKS.iter().map(|s| (*s).to_owned()).collect()));
        packs.selected = enabled.into_iter().filter(|id| packs.find(id).is_some()).collect();
        packs.disabled = disabled;
        for id in packs.new_packs() {
            info!("Found new data pack {id}, loading it automatically");
            packs.selected.push(id);
            packs.dirty = true;
        }
        packs
    }

    fn find(&self, id: &str) -> Option<&Pack> {
        self.available.iter().find(|p| p.info.id == id)
    }

    /// `PackRepository.reload`: what exists now, sorted by id.
    pub fn discover(&mut self) {
        let mut found: BTreeMap<String, Pack> = BTreeMap::new();
        let vanilla = PackInfo {
            id: "vanilla".into(),
            source: PackSource::BuiltIn,
            description: Text::translate("dataPack.vanilla.description", vec![]),
            required_features: Vec::new(),
        };
        found.insert("vanilla".into(), Pack { info: vanilla, root: Some(self.vanilla.clone()) });
        for name in FEATURE_PACKS {
            let info = PackInfo {
                id: name.into(),
                source: PackSource::Feature,
                description: Text::translate(format!("dataPack.{name}.description"), vec![]),
                required_features: vec![format!("minecraft:{name}")],
            };
            found.insert(name.into(), Pack { info, root: None });
        }
        if let Some(dir) = &self.dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
                if !path.is_dir() || !path.join("pack.mcmeta").is_file() {
                    continue;
                }
                let id = format!("file/{name}");
                let description = pack_description(&path);
                let info = PackInfo { id: id.clone(), source: PackSource::World, description, required_features: Vec::new() };
                found.insert(id, Pack { info, root: Some(path) });
            }
        }
        self.available = found.into_values().collect();
        let available: Vec<String> = self.available.iter().map(|p| p.info.id.clone()).collect();
        self.selected.retain(|id| available.contains(id));
    }

    /// Packs neither enabled nor disabled that need no missing features.
    fn new_packs(&self) -> Vec<String> {
        self.available
            .iter()
            .filter(|p| p.info.required_features.is_empty())
            .map(|p| p.info.id.clone())
            .filter(|id| !self.selected.contains(id) && !self.disabled.contains(id))
            .collect()
    }

    /// Directories of the enabled packs, in load order.
    fn roots(&self) -> Vec<PathBuf> {
        self.selected.iter().filter_map(|id| self.find(id)?.root.clone()).collect()
    }

    pub fn snapshot(&self) -> DataPacks {
        DataPacks {
            available: self.available.iter().map(|p| p.info.clone()).collect(),
            selected: self.selected.clone(),
            features: Vec::new(),
        }
    }
}

/// `pack.mcmeta`'s description, when it is a plain string.
fn pack_description(dir: &Path) -> Text {
    let text = std::fs::read_to_string(dir.join("pack.mcmeta")).unwrap_or_default();
    match Json::parse(&text).ok().as_ref().and_then(|j| j.get("pack")?.get("description")?.as_str().map(str::to_owned)) {
        Some(d) => Text::literal(d),
        None => Text::literal(""),
    }
}

/// A text component's NBT as JSON (bytes become booleans, as component fields use them).
fn component_json(tag: &kiln_proto::nbt::Tag) -> String {
    use kiln_proto::nbt::Tag;
    let string = |s: &str| {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    match tag {
        Tag::String(s) => string(s),
        Tag::Byte(b) => (if *b != 0 { "true" } else { "false" }).into(),
        Tag::Short(v) => v.to_string(),
        Tag::Int(v) => v.to_string(),
        Tag::Long(v) => v.to_string(),
        Tag::Float(v) => v.to_string(),
        Tag::Double(v) => v.to_string(),
        Tag::List(items) => format!("[{}]", items.iter().map(component_json).collect::<Vec<_>>().join(",")),
        Tag::Compound(fields) => {
            let body: Vec<String> = fields.iter().map(|(k, v)| format!("{}:{}", string(k), component_json(v))).collect();
            format!("{{{}}}", body.join(","))
        }
        Tag::ByteArray(v) => format!("[{}]", v.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")),
        Tag::IntArray(v) => format!("[{}]", v.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")),
        Tag::LongArray(v) => format!("[{}]", v.iter().map(ToString::to_string).collect::<Vec<_>>().join(",")),
    }
}

/// Files under `root` with `ext`, as (relative path without extension, full path).
fn walk(root: &Path, ext: &str, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        if path.is_dir() {
            walk(&path, ext, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(ext) {
            out.push((format!("{prefix}{stem}"), path));
        }
    }
}

/// Every `(namespace, relative path, file)` of `data/<ns>/<dir>/**.<ext>` in `root`.
fn pack_files(root: &Path, dir: &str, ext: &str) -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(namespaces) = std::fs::read_dir(root.join("data")) else { return out };
    for ns in namespaces.flatten() {
        let Some(namespace) = ns.file_name().to_str().map(str::to_owned) else { continue };
        let mut files = Vec::new();
        walk(&ns.path().join(dir), ext, "", &mut files);
        out.extend(files.into_iter().map(|(rel, path)| (namespace.clone(), rel, path)));
    }
    out
}

/// A function tag entry: an id or `#tag`, required unless `required: false`.
struct TagEntry {
    id: String,
    required: bool,
}

fn tag_entries(json: &Json) -> Vec<TagEntry> {
    let values = json.get("values").and_then(Json::as_array).unwrap_or(&[]);
    values
        .iter()
        .filter_map(|v| match v.as_str() {
            Some(s) => Some(TagEntry { id: s.to_owned(), required: true }),
            None => Some(TagEntry {
                id: v.get("id")?.as_str()?.to_owned(),
                required: v.get("required").and_then(Json::as_bool).unwrap_or(true),
            }),
        })
        .collect()
}

/// Resolves a tag's entries to functions (`TagLoader.build`); `None` if a required entry
/// is missing.
fn resolve_tag(
    id: &str,
    raw: &BTreeMap<String, Vec<TagEntry>>,
    library: &FunctionLibrary,
    visiting: &mut Vec<String>,
) -> Option<Vec<Identifier>> {
    if visiting.iter().any(|v| v == id) {
        return None;
    }
    visiting.push(id.to_owned());
    let mut out: Vec<Identifier> = Vec::new();
    for e in raw.get(id).map(Vec::as_slice).unwrap_or(&[]) {
        let found = match e.id.strip_prefix('#') {
            Some(tag) => {
                let tag = Identifier::parse(tag).map(|t| t.to_string());
                tag.filter(|t| raw.contains_key(t)).and_then(|t| resolve_tag(&t, raw, library, visiting))
            }
            None => Identifier::parse(&e.id).filter(|f| library.get(f).is_some()).map(|f| vec![f]),
        };
        match found {
            Some(ids) => {
                for f in ids {
                    if !out.contains(&f) {
                        out.push(f);
                    }
                }
            }
            None if e.required => {
                visiting.pop();
                return None;
            }
            None => {}
        }
    }
    visiting.pop();
    Some(out)
}

impl Sim {
    /// Sets up the packs from the world (or `KILN_DATAPACKS`) and loads what they provide.
    pub(crate) fn init_packs(&mut self, vanilla: PathBuf) {
        let dir = match &self.config.world {
            Some(world) => Some(world.join("datapacks")),
            None => std::env::var_os("KILN_DATAPACKS").map(PathBuf::from),
        };
        let saved = self.storage.as_ref().and_then(|s| s.level.data_packs());
        self.commands.packs = Packs::new(dir, vanilla, saved);
        if let Some(data) = self.storage.as_ref().and_then(|s| kiln_storage::saved_data::read(&s.dir, "scheduled_events")) {
            self.commands.packs.timers.load_nbt(&data);
        }
        let only_vanilla = self.commands.packs.selected == ["vanilla"];
        self.load_packs(!only_vanilla);
    }

    /// `/reload` and `/datapack enable|disable`: the new pack list, then everything the packs
    /// provide.
    pub(crate) fn reload_data_packs(&mut self, selected: Option<Vec<String>>) {
        let packs = &mut self.commands.packs;
        packs.discover();
        match selected {
            Some(list) => packs.selected = list,
            None => {
                for id in packs.new_packs() {
                    packs.selected.push(id);
                }
            }
        }
        let selected = packs.selected.clone();
        packs.disabled = packs.available.iter().map(|p| p.info.id.clone()).filter(|id| !selected.contains(id)).collect();
        packs.dirty = true;
        self.load_packs(true);
        let pkt = kiln_inventory::recipe::sync::update_recipes(&self.rules.recipes);
        for p in self.players.values_mut() {
            p.send(pkt.clone());
        }
    }

    /// Loads functions and function tags, and with `data`, recipes and loot tables, from the
    /// enabled packs.
    fn load_packs(&mut self, data: bool) {
        let roots = self.commands.packs.roots();
        if data {
            let refs: Vec<&Path> = roots.iter().map(PathBuf::as_path).collect();
            if refs.is_empty() {
                self.rules = std::sync::Arc::new(kiln_inventory::Rules::with_recipes(Default::default()));
                self.loot = None;
            } else {
                match kiln_inventory::Rules::load_packs(&refs) {
                    Ok(rules) => self.rules = std::sync::Arc::new(rules),
                    Err(e) => warn!("recipes not reloaded: {e}"),
                }
                match kiln_loot::LootData::load_lenient_packs(&refs) {
                    Ok(loot) => self.loot = Some(std::sync::Arc::new(loot)),
                    Err(e) => warn!("loot tables not reloaded: {e}"),
                }
            }
            // Players read enchantment definitions from the loot data.
            for p in self.players.values_mut() {
                p.loot = self.loot.clone();
            }
        }
        self.commands.packs.library = self.load_functions(&roots);
        self.load_advancements(&roots);
        self.commands.packs.load_pending = true;
    }

    /// `ServerFunctionLibrary.reload`: later packs replace functions; every plain line must
    /// parse (at permission level 2) or the function is not loaded.
    fn load_functions(&mut self, roots: &[PathBuf]) -> FunctionLibrary {
        let mut sources: BTreeMap<Identifier, PathBuf> = BTreeMap::new();
        let mut raw_tags: BTreeMap<String, Vec<TagEntry>> = BTreeMap::new();
        for root in roots {
            for (ns, rel, path) in pack_files(root, "function", ".mcfunction") {
                if let Some(id) = Identifier::parse(&format!("{ns}:{rel}")) {
                    sources.insert(id, path);
                }
            }
            for (ns, rel, path) in pack_files(root, "tags/function", ".json") {
                let Some(id) = Identifier::parse(&format!("{ns}:{rel}")) else { continue };
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let Ok(json) = Json::parse(&text) else {
                    error!("Couldn't read tag list {id} from {}", path.display());
                    continue;
                };
                let slot = raw_tags.entry(id.to_string()).or_default();
                if json.get("replace").and_then(Json::as_bool).unwrap_or(false) {
                    slot.clear();
                }
                slot.extend(tag_entries(&json));
            }
        }
        let dispatcher = self.commands.dispatcher.clone();
        let saved = self.commands.stack.clone();
        let mut check = SourceStack::new(Text::literal("Server"), crate::commands::OVERWORLD, [0.0; 3]);
        check.max_permission = 2;
        check.silent = true;
        self.commands.stack = check;
        let mut library = FunctionLibrary::default();
        for (id, path) in sources {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            let function = match CommandFunction::from_lines(id.clone(), &lines) {
                Ok(f) => f,
                Err(e) => {
                    error!("Failed to load function {id}: {e}");
                    continue;
                }
            };
            let bad = function.plain_lines().enumerate().find_map(|(i, line)| {
                let parse = dispatcher.parse(line, self);
                dispatcher.check_parse(&parse).err().map(|e| (i, e))
            });
            if let Some((i, e)) = bad {
                error!("Failed to load function {id}: Whilst parsing command on line {}: {}", i + 1, e.message().to_plain());
                continue;
            }
            library.insert(function);
        }
        self.commands.stack = saved;
        for id in raw_tags.keys() {
            match resolve_tag(id, &raw_tags, &library, &mut Vec::new()) {
                Some(functions) => library.set_tag(Identifier::parse(id).expect("tag id"), functions),
                None => error!("Couldn't load tag {id} as it is missing following references"),
            }
        }
        info!("loaded {} functions from {} data packs", library.len(), roots.len());
        library
    }

    /// `ServerFunctionManager.tick` then the due scheduled functions (`TimerQueue.tick`).
    /// Whether `#minecraft:tick` has functions (they run every tick, with the whole server).
    pub(crate) fn has_tick_functions(&self) -> bool {
        !self.commands.packs.library.tag(&Identifier::parse(TICK_TAG).expect("tag")).is_empty()
    }

    /// Whether server functions run at game time `time`: `#minecraft:load` after a (re)load,
    /// `#minecraft:tick`, or a `/schedule` due.
    pub(crate) fn functions_due(&self, time: i64) -> bool {
        self.commands.packs.load_pending
            || self.has_tick_functions()
            || self.commands.packs.timers.next_trigger().is_some_and(|t| t <= time)
    }

    pub(crate) fn tick_functions(&mut self) {
        if std::mem::take(&mut self.commands.packs.load_pending) {
            for f in self.commands.packs.library.tag(&Identifier::parse(LOAD_TAG).expect("tag")) {
                self.run_server_function(&f);
            }
        }
        for f in self.commands.packs.library.tag(&Identifier::parse(TICK_TAG).expect("tag")) {
            self.run_server_function(&f);
        }
        for callback in self.commands.packs.timers.due(self.game_time) {
            let lib = &self.commands.packs.library;
            let functions = match callback {
                TimerCallback::Function(id) => lib.get(&id).into_iter().collect(),
                TimerCallback::Tag(id) => lib.tag(&id),
            };
            for f in functions {
                self.run_server_function(&f);
            }
        }
    }

    /// Runs a function as the server (`getGameLoopSender`): at the world spawn, silent, at
    /// permission level 2.
    fn run_server_function(&mut self, f: &CommandFunction) {
        let dispatcher = self.commands.dispatcher.clone();
        let previous = std::mem::replace(&mut self.commands.source, crate::commands::CommandSource::Console);
        let stack = SourceStack::new(Text::literal("Server"), crate::commands::OVERWORLD, self.spawn.map(|v| v as f64));
        let saved = std::mem::replace(&mut self.commands.stack, stack.clone());
        if let Err(e) = kiln_command::vanilla::run_as_server(&dispatcher, self, f, stack) {
            warn!("Failed to execute function {}: {}", f.id, e.to_plain());
        }
        self.commands.stack = saved;
        self.commands.source = previous;
        self.flush_scoreboard();
    }

    /// Writes the scheduled functions if they changed.
    pub(crate) fn save_timers(&mut self) {
        let Some(storage) = &self.storage else { return };
        if self.commands.packs.timers.take_dirty()
            && let Err(e) = kiln_storage::saved_data::write(&storage.dir, "scheduled_events", self.commands.packs.timers.to_nbt())
        {
            warn!("failed to save scheduled functions: {e}");
        }
    }

    /// `/datapack create`: an empty pack in the world's `datapacks` directory.
    pub(crate) fn create_data_pack(&mut self, id: &str, description: &Text) -> Result<(), kiln_command::CommandError> {
        use kiln_command::{CommandError, tr};
        let Some(dir) = self.commands.packs.dir.clone() else {
            return Err(CommandError::new(tr!("commands.datapack.create.io_failure", id)));
        };
        if id.is_empty() || id.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || id == "." || id == ".." {
            return Err(CommandError::new(tr!("commands.datapack.create.invalid_name", id)));
        }
        let path = dir.join(id);
        if path.exists() {
            return Err(CommandError::new(tr!("commands.datapack.create.already_exists", id)));
        }
        let meta = format!(
            "{{\n  \"pack\": {{\n    \"description\": {},\n    \"min_format\": 121,\n    \"max_format\": 121\n  }}\n}}",
            component_json(&description.to_nbt())
        );
        let made = std::fs::create_dir_all(path.join("data")).and_then(|()| std::fs::write(path.join("pack.mcmeta"), meta));
        made.map_err(|e| {
            warn!("Failed to create pack at {}: {e}", path.display());
            CommandError::new(tr!("commands.datapack.create.io_failure", id))
        })
    }
}
