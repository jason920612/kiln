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

/// Feature packs built into the server jar (`data/minecraft/datapacks/<name>` of the vanilla
/// data), each needing its feature flag.
const FEATURE_PACKS: [&str; 3] = ["minecart_improvements", "redstone_experiments", "trade_rebalance"];
const VANILLA_FEATURE: &str = "minecraft:vanilla";
const LOAD_TAG: &str = "minecraft:load";
const TICK_TAG: &str = "minecraft:tick";

pub(crate) struct Pack {
    pub info: PackInfo,
    /// The directory holding `data/`, for packs Kiln can read (a zip pack's unpacked copy).
    root: Option<PathBuf>,
}

impl Pack {
    /// `PackSource.shouldAddAutomatically`: feature packs are only enabled on request.
    fn adds_automatically(&self) -> bool {
        self.info.source != PackSource::Feature
    }

    /// Whether every feature the pack requests is in `features`.
    fn features_in(&self, features: &[String]) -> bool {
        self.info.required_features.iter().all(|f| features.contains(f))
    }
}

pub(crate) struct Packs {
    /// Folder and zip packs live here.
    dir: Option<PathBuf>,
    vanilla: PathBuf,
    pub available: Vec<Pack>,
    pub selected: Vec<String>,
    pub disabled: Vec<String>,
    /// The world's feature flags (`WorldData.enabledFeatures`), fixed when it was created.
    pub features: Vec<String>,
    pub library: FunctionLibrary,
    pub timers: TimerQueue,
    /// `#minecraft:load` runs on the next tick (`ServerFunctionManager.postReload`).
    pub load_pending: bool,
    /// The enabled/disabled lists changed since `level.dat` was written.
    pub dirty: bool,
}

/// What a world starts from: its saved pack lists and features, or for a new world the
/// initial packs (`initial-enabled-packs`, `initial-disabled-packs`).
pub(crate) struct PackConfig {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    /// `None` for a new world (`initMode`): every feature counts as available and the
    /// features come from the packs chosen.
    pub features: Option<Vec<String>>,
}

impl PackConfig {
    /// A new world's packs from `KILN_INITIAL_PACKS` / `KILN_INITIAL_DISABLED_PACKS` (comma
    /// separated, like the server properties; default `vanilla` and none).
    pub fn initial() -> Self {
        let list = |var: &str, default: &str| -> Vec<String> {
            let value = std::env::var(var).unwrap_or_else(|_| default.to_owned());
            value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
        };
        PackConfig { enabled: list("KILN_INITIAL_PACKS", "vanilla"), disabled: list("KILN_INITIAL_DISABLED_PACKS", ""), features: None }
    }
}

impl Packs {
    /// `MinecraftServer.configurePackRepository`: the saved (or initial) enabled packs that
    /// exist, new packs that need no missing feature enabled (`Found new data pack ...,
    /// loading it automatically`), packs whose features the world lacks dropped, `vanilla`
    /// if nothing is left; the world's features are its saved ones, or for a new world those
    /// of the chosen packs.
    pub fn new(dir: Option<PathBuf>, vanilla: PathBuf, config: PackConfig) -> Self {
        let mut packs = Packs {
            dir,
            vanilla,
            available: Vec::new(),
            selected: Vec::new(),
            disabled: Vec::new(),
            features: Vec::new(),
            library: FunctionLibrary::default(),
            timers: TimerQueue::default(),
            load_pending: true,
            dirty: false,
        };
        packs.discover();
        let init = config.features.is_none();
        let base = config.features.clone().unwrap_or_default();
        let all: Vec<String> = packs.available.iter().flat_map(|p| p.info.required_features.iter().cloned()).collect();
        let check = if init { all } else { base.clone() };
        let mut selected: Vec<String> = Vec::new();
        for id in config.enabled {
            if packs.find(&id).is_some() {
                if !selected.contains(&id) {
                    selected.push(id);
                }
            } else {
                warn!("Missing data pack {id}");
            }
        }
        for pack in &packs.available {
            let id = &pack.info.id;
            if config.disabled.contains(id) {
                continue;
            }
            let is_selected = selected.contains(id);
            if !is_selected && pack.adds_automatically() {
                if pack.features_in(&check) {
                    info!("Found new data pack {id}, loading it automatically");
                    selected.push(id.clone());
                    packs.dirty = true;
                } else {
                    info!("Found new data pack {id}, but can't load it due to missing features {}", missing(pack, &check));
                }
            }
            if is_selected && !pack.features_in(&check) {
                warn!("Pack {id} requires features {} that are not enabled for this world, disabling pack.", missing(pack, &check));
                selected.retain(|s| s != id);
                packs.dirty = true;
            }
        }
        if selected.is_empty() {
            info!("No datapacks selected, forcing vanilla");
            selected.push("vanilla".into());
        }
        packs.selected = selected;
        let mut features = base;
        if !features.iter().any(|f| f == VANILLA_FEATURE) && init {
            features.push(VANILLA_FEATURE.into());
        }
        for id in &packs.selected {
            for f in packs.find(id).map(|p| p.info.required_features.clone()).unwrap_or_default() {
                if !features.contains(&f) {
                    features.push(f);
                }
            }
        }
        if features.is_empty() {
            features.push(VANILLA_FEATURE.into());
        }
        packs.features = features;
        packs.update_disabled();
        packs
    }

    fn find(&self, id: &str) -> Option<&Pack> {
        self.available.iter().find(|p| p.info.id == id)
    }

    /// `getSelectedPacks`: every available pack that is not enabled.
    fn update_disabled(&mut self) {
        let selected = &self.selected;
        self.disabled = self.available.iter().map(|p| p.info.id.clone()).filter(|id| !selected.contains(id)).collect();
    }

    /// `PackRepository.reload`: what exists now, sorted by id.
    pub fn discover(&mut self) {
        let mut found: BTreeMap<String, Pack> = BTreeMap::new();
        let vanilla = PackInfo {
            id: "vanilla".into(),
            source: PackSource::BuiltIn,
            description: Text::translate("dataPack.vanilla.description", vec![]),
            required_features: vec![VANILLA_FEATURE.into()],
        };
        found.insert("vanilla".into(), Pack { info: vanilla, root: Some(self.vanilla.clone()) });
        for name in FEATURE_PACKS {
            let root = self.vanilla.join("data/minecraft/datapacks").join(name);
            let meta = read_meta(&root.join("pack.mcmeta"));
            let required_features = meta.as_ref().map(meta_features).filter(|f| !f.is_empty());
            let info = PackInfo {
                id: name.into(),
                source: PackSource::Feature,
                description: Text::translate(format!("dataPack.{name}.description"), vec![]),
                required_features: required_features.unwrap_or_else(|| vec![format!("minecraft:{name}")]),
            };
            found.insert(name.into(), Pack { info, root: meta.is_some().then_some(root) });
        }
        if let Some(dir) = &self.dir
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
                // `FolderRepositorySource`: directories with a `pack.mcmeta`, and zip files.
                let root = if path.is_dir() {
                    if !path.join("pack.mcmeta").is_file() {
                        continue;
                    }
                    path
                } else if path.is_file() && name.to_ascii_lowercase().ends_with(".zip") && crate::zip_pack::is_pack(&path) {
                    match crate::zip_pack::unpacked(&path) {
                        Ok(root) => root,
                        Err(e) => {
                            warn!("Failed to read data pack {}: {e}", path.display());
                            continue;
                        }
                    }
                } else {
                    continue;
                };
                let id = format!("file/{name}");
                let meta = read_meta(&root.join("pack.mcmeta"));
                let description = meta.as_ref().map_or_else(|| Text::literal(""), meta_description);
                let required_features = meta.as_ref().map(meta_features).unwrap_or_default();
                let info = PackInfo { id: id.clone(), source: PackSource::World, description, required_features };
                found.insert(id, Pack { info, root: Some(root) });
            }
        }
        self.available = found.into_values().collect();
        let available: Vec<String> = self.available.iter().map(|p| p.info.id.clone()).collect();
        self.selected.retain(|id| available.contains(id));
    }

    /// `ReloadCommand.discoverNewPacks`: packs neither enabled nor disabled whose features the
    /// world has.
    fn new_packs(&self) -> Vec<String> {
        self.available
            .iter()
            .filter(|p| p.features_in(&self.features))
            .map(|p| p.info.id.clone())
            .filter(|id| !self.selected.contains(id) && !self.disabled.contains(id))
            .collect()
    }

    /// Directories of the enabled packs, in load order.
    fn roots(&self) -> Vec<PathBuf> {
        self.selected.iter().filter_map(|id| self.find(id)?.root.clone()).collect()
    }

    /// The tag sources of the enabled packs in load order: the vanilla pack without its files
    /// is the built-in copy of its tags.
    fn tag_roots(&self) -> Vec<(bool, Option<PathBuf>)> {
        self.selected
            .iter()
            .filter_map(|id| {
                let root = self.find(id)?.root.clone();
                let builtin = id == "vanilla" && !root.as_ref().is_some_and(|r| r.join("data/minecraft/tags").is_dir());
                Some((builtin, root))
            })
            .collect()
    }

    pub fn snapshot(&self) -> DataPacks {
        DataPacks {
            available: self.available.iter().map(|p| p.info.clone()).collect(),
            selected: self.selected.clone(),
            features: self.features.clone(),
        }
    }
}

/// `FeatureFlags.printMissingFlags`.
fn missing(pack: &Pack, features: &[String]) -> String {
    let missing: Vec<&str> = pack.info.required_features.iter().filter(|f| !features.contains(f)).map(String::as_str).collect();
    missing.join(", ")
}

/// `pack.mcmeta` as NBT (JSON is read as SNBT).
fn read_meta(path: &Path) -> Option<kiln_proto::nbt::Tag> {
    let text = std::fs::read_to_string(path).ok()?;
    kiln_command::snbt::parse_tag(&mut kiln_command::StringReader::new(text.trim_start_matches('\u{feff}'))).ok()
}

/// `pack.description`, a text component.
fn meta_description(meta: &kiln_proto::nbt::Tag) -> Text {
    match meta.get("pack").and_then(|p| p.get("description")) {
        Some(d) => kiln_command::component::decode(d).map_or_else(|_| Text::literal(""), |c| c.to_text()),
        None => Text::literal(""),
    }
}

/// `features.enabled`: the feature flags a pack requests.
fn meta_features(meta: &kiln_proto::nbt::Tag) -> Vec<String> {
    let list = meta.get("features").and_then(|f| f.get("enabled")).and_then(kiln_proto::nbt::Tag::as_list).unwrap_or(&[]);
    list.iter().filter_map(|t| t.as_str()).map(|s| if s.contains(':') { s.to_owned() } else { format!("minecraft:{s}") }).collect()
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
pub(crate) fn pack_files(root: &Path, dir: &str, ext: &str) -> Vec<(String, String, PathBuf)> {
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
        let level = self.storage.as_ref().map(|s| &s.level);
        let config = match level.and_then(|l| l.data_packs()) {
            Some((enabled, disabled)) => {
                // Worlds without saved features have vanilla's defaults.
                let features = level.and_then(|l| l.enabled_features()).unwrap_or_else(|| vec![VANILLA_FEATURE.into()]);
                PackConfig { enabled, disabled, features: Some(features) }
            }
            None => PackConfig::initial(),
        };
        self.commands.packs = Packs::new(dir, vanilla, config);
        self.config.data_sync.set_features(self.commands.packs.features.clone());
        if let Some(data) = self.storage.as_ref().and_then(|s| kiln_storage::saved_data::read(&s.dir, "scheduled_events")) {
            self.commands.packs.timers.load_nbt(&data);
        }
        let only_vanilla = self.commands.packs.selected == ["vanilla"];
        self.load_packs(!only_vanilla);
        if !only_vanilla {
            self.load_tags();
        }
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
        packs.update_disabled();
        packs.dirty = true;
        self.load_packs(true);
        // `PlayerList.reloadResources`: the tags to everyone, then each player's recipes and
        // recipe book.
        let tags = self.load_tags();
        let tags = kiln_proto::packets::update_tags_owned(kiln_data::packets::play::clientbound::UPDATE_TAGS, &tags);
        let recipes = kiln_inventory::recipe::sync::update_recipes(&self.rules.recipes);
        let rules = self.rules.clone();
        for p in self.players.values_mut() {
            p.send(tags.clone());
        }
        for p in self.players.values_mut() {
            p.send(recipes.clone());
            p.send_initial_recipe_book(&rules);
        }
    }

    /// The enabled packs' tags for the network; logins get them from now on.
    fn load_tags(&mut self) -> crate::tags::NetworkTags {
        use crate::tags::TagSource;
        let roots = self.commands.packs.tag_roots();
        let sources: Vec<TagSource> = roots
            .iter()
            .filter_map(|(builtin, root)| match (builtin, root) {
                (true, _) => Some(TagSource::BuiltIn),
                (false, Some(r)) => Some(TagSource::Dir(r)),
                (false, None) => None,
            })
            .collect();
        let tags = crate::tags::load(&sources);
        let config = kiln_proto::packets::update_tags_owned(kiln_data::packets::configuration::clientbound::UPDATE_TAGS, &tags);
        self.config.data_sync.set_config_tags(Some(config));
        tags
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sim, SimConfig};
    use kiln_link::ToSim;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kiln-packs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The vanilla data (with the feature packs), if this machine has it.
    fn vanilla() -> Option<PathBuf> {
        let root = crate::datapack_dir(None);
        let root = if root.is_absolute() { root } else { Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(root) };
        root.join("data/minecraft/datapacks/minecart_improvements/pack.mcmeta").is_file().then_some(root)
    }

    #[test]
    fn zip_packs_load_and_reload_sends_tags() {
        let world = scratch("zip");
        let packs = world.join("datapacks");
        std::fs::create_dir_all(packs.join("tagpack/data/minecraft/tags/block")).unwrap();
        std::fs::write(packs.join("tagpack/pack.mcmeta"), r#"{"pack":{"description":"t","min_format":121,"max_format":121}}"#).unwrap();
        let zip = crate::zip_pack::tests::build(
            &[
                ("pack.mcmeta", br#"{"pack":{"description":{"text":"zipped"},"min_format":121,"max_format":121}}"#),
                ("data/kz/function/hi.mcfunction", b"say hi\n"),
            ],
            true,
        );
        std::fs::write(packs.join("kz.zip"), zip).unwrap();
        std::fs::write(packs.join("notapack.zip"), b"junk").unwrap();
        let mut sim = Sim::new(SimConfig::new(8, 2, Some(world.clone())));
        let p = &sim.commands.packs;
        assert!(p.selected.iter().any(|s| s == "file/kz.zip"), "{:?}", p.selected);
        assert!(p.find("file/notapack.zip").is_none());
        assert_eq!(p.find("file/kz.zip").unwrap().info.description.to_plain(), "zipped");
        assert!(p.library.get(&Identifier::parse("kz:hi").unwrap()).is_some());

        let (msg, stats) = crate::testing::join(1, "Tagger", 2);
        *stats.log.lock().unwrap() = Some(Vec::new());
        assert!(sim.step([msg]));
        std::fs::write(
            packs.join("tagpack/data/minecraft/tags/block/kiln_test.json"),
            r#"{"values":["minecraft:stone","minecraft:dirt"]}"#,
        )
        .unwrap();
        stats.log.lock().unwrap().as_mut().unwrap().clear();
        assert!(sim.step([ToSim::Console("reload".into())]));
        let log = stats.log.lock().unwrap().clone().unwrap();
        let id = |p: &bytes::Bytes| kiln_proto::codec::Reader::new(p).varint().ok();
        let tags = log.iter().position(|p| id(p) == Some(kiln_data::packets::play::clientbound::UPDATE_TAGS)).expect("update tags");
        let recipes =
            log.iter().position(|p| id(p) == Some(kiln_data::packets::play::clientbound::UPDATE_RECIPES)).expect("update recipes");
        assert!(tags < recipes);
        let needle = b"minecraft:kiln_test";
        assert!(log[tags].windows(needle.len()).any(|w| w == needle));
        // Logins from now on get the same tags.
        let config = sim.config.data_sync.config_tags().expect("config tags");
        assert!(config.windows(needle.len()).any(|w| w == needle));
        drop(sim);
        let _ = std::fs::remove_dir_all(&world);
    }

    #[test]
    fn feature_packs_need_their_world_features() {
        let Some(vanilla) = vanilla() else {
            eprintln!("skipped: no vanilla data");
            return;
        };
        let config = |enabled: &[&str], features: Option<&[&str]>| PackConfig {
            enabled: enabled.iter().map(|s| (*s).to_owned()).collect(),
            disabled: Vec::new(),
            features: features.map(|f| f.iter().map(|s| (*s).to_owned()).collect()),
        };
        // A new world with a feature pack gets its feature.
        let p = Packs::new(None, vanilla.clone(), config(&["vanilla", "minecart_improvements"], None));
        assert_eq!(p.selected, ["vanilla", "minecart_improvements"]);
        assert_eq!(p.features, ["minecraft:vanilla", "minecraft:minecart_improvements"]);
        assert!(p.find("minecart_improvements").unwrap().root.is_some());
        assert_eq!(p.disabled, ["redstone_experiments", "trade_rebalance"]);
        // A saved world without the feature drops the pack.
        let p = Packs::new(None, vanilla.clone(), config(&["vanilla", "trade_rebalance"], Some(&["minecraft:vanilla"])));
        assert_eq!(p.selected, ["vanilla"]);
        assert_eq!(p.features, ["minecraft:vanilla"]);
        // Nothing left: vanilla.
        let p = Packs::new(None, vanilla, config(&["nope"], Some(&["minecraft:vanilla"])));
        assert_eq!(p.selected, ["vanilla"]);
    }
}
