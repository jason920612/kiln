//! Advancements: the definitions of the enabled data packs (`ServerAdvancementManager`, the
//! `AdvancementTree` with vanilla's `TreeNodePosition` layout), each player's progress
//! (`PlayerAdvancements`, [`progress`]) and the criteria triggers ([`criteria`]).

pub(crate) mod criteria;
pub(crate) mod progress;
pub(crate) mod triggers;

use bytes::{Bytes, BytesMut};
use criteria::Criterion;
use kiln_item::ItemStackTemplate;
use kiln_loot::{Json, LootData};
use kiln_proto::WriteExt;
use kiln_proto::nbt::Tag;
use std::collections::HashMap;
use std::path::Path;
use tracing::{error, info};

/// `AdvancementType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Frame {
    Task,
    Challenge,
    Goal,
}

impl Frame {
    fn name(self) -> &'static str {
        match self {
            Frame::Task => "task",
            Frame::Challenge => "challenge",
            Frame::Goal => "goal",
        }
    }
    /// `getChatColor`.
    fn color(self) -> &'static str {
        match self {
            Frame::Challenge => "dark_purple",
            _ => "green",
        }
    }
}

/// `DisplayInfo`.
#[derive(Debug, Clone)]
pub(crate) struct Display {
    pub icon: ItemStackTemplate,
    /// Text components as NBT.
    pub title: Tag,
    pub description: Tag,
    pub frame: Frame,
    pub background: Option<String>,
    pub show_toast: bool,
    pub announce_to_chat: bool,
    pub hidden: bool,
}

/// `AdvancementRewards`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Rewards {
    pub experience: i32,
    pub loot: Vec<String>,
    pub recipes: Vec<String>,
    pub function: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Advancement {
    pub id: String,
    pub parent: Option<String>,
    pub display: Option<Display>,
    pub criteria: Vec<(String, Criterion)>,
    /// `AdvancementRequirements`: every group needs one of its criteria (indices).
    pub requirements: Vec<Vec<usize>>,
    pub rewards: Rewards,
    pub sends_telemetry_event: bool,
}

impl Advancement {
    pub fn criterion_index(&self, name: &str) -> Option<usize> {
        self.criteria.iter().position(|(n, _)| n == name)
    }

    /// `Advancement.name`: `[title]` in the frame's color hovering the title and description,
    /// or the id for advancements without a display.
    pub fn name(&self) -> kiln_command::Text {
        match &self.display {
            None => kiln_command::Text::literal(self.id.clone()),
            Some(d) => {
                let color = Tag::String(d.frame.color().into());
                // The title in the frame's color, then a line break and the description.
                let mut hover = compound(&d.title);
                set(&mut hover, "color", color.clone());
                push_extra(&mut hover, Tag::String("\n".into()));
                push_extra(&mut hover, d.description.clone());
                let mut inner = compound(&d.title);
                set(
                    &mut inner,
                    "hover_event",
                    Tag::Compound(vec![("action".into(), Tag::String("show_text".into())), ("value".into(), hover)]),
                );
                kiln_command::Text::raw(Tag::Compound(vec![
                    ("translate".into(), Tag::String("chat.square_brackets".into())),
                    ("with".into(), Tag::List(vec![inner])),
                    ("color".into(), color),
                ]))
            }
        }
    }

    /// `AdvancementType.createAnnouncement`.
    pub fn announcement(&self, player: kiln_command::Text) -> Option<kiln_command::Text> {
        let d = self.display.as_ref()?;
        Some(kiln_command::Text::translate(
            format!("chat.type.advancement.{}", d.frame.name()),
            vec![kiln_command::text::Arg::Text(player), kiln_command::text::Arg::Text(self.name())],
        ))
    }
}

/// Every advancement of the enabled packs, as a tree.
#[derive(Debug, Default)]
pub(crate) struct Advancements {
    /// In registry order.
    pub list: Vec<Advancement>,
    pub by_id: HashMap<String, usize>,
    pub parent: Vec<Option<usize>>,
    /// Children in insertion order.
    pub children: Vec<Vec<usize>>,
    /// `AdvancementNode.x/y` (`TreeNodePosition`), zero for nodes without display.
    pub position: Vec<(f32, f32)>,
    /// Criteria by trigger id: (advancement, criterion).
    pub by_trigger: HashMap<String, Vec<(usize, usize)>>,
    /// Encoded `AdvancementHolder` + position per advancement, for Update Advancements.
    pub encoded: Vec<Bytes>,
    /// Files that did not load: (id, why).
    pub errors: Vec<(String, String)>,
}

impl Advancements {
    pub fn get(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    /// `AdvancementNode.root`.
    pub fn root(&self, mut i: usize) -> usize {
        while let Some(p) = self.parent[i] {
            i = p;
        }
        i
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn criteria_for(&self, trigger: &str) -> &[(usize, usize)] {
        self.by_trigger.get(trigger).map_or(&[], Vec::as_slice)
    }

    /// Loads `data/*/advancement/**.json` of the packs (later packs replace earlier files) and
    /// builds the tree. Files that do not decode are logged and skipped, as are advancements
    /// whose parent is missing.
    pub fn load(packs: &[&Path], loot: Option<&LootData>) -> Advancements {
        let mut files: HashMap<(String, String), std::path::PathBuf> = HashMap::new();
        for root in packs {
            let Ok(namespaces) = std::fs::read_dir(root.join("data")) else { continue };
            for ns in namespaces.flatten() {
                let Some(namespace) = ns.file_name().to_str().map(str::to_owned) else { continue };
                let mut found = Vec::new();
                collect(&ns.path().join("advancement"), "", &mut found);
                for (rel, path) in found {
                    files.insert((namespace.clone(), rel), path);
                }
            }
        }
        let mut files: Vec<((String, String), std::path::PathBuf)> = files.into_iter().collect();
        // Registry order: `Identifier.compareTo` (path, then namespace).
        files.sort_by(|a, b| (a.0.1.as_str(), a.0.0.as_str()).cmp(&(b.0.1.as_str(), b.0.0.as_str())));
        let fallback = LootData::default();
        let loot = loot.unwrap_or(&fallback);
        let parser = loot.parser();
        let mut parsed = Vec::new();
        let mut errors = Vec::new();
        for ((ns, rel), path) in files {
            let id = format!("{ns}:{rel}");
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let json = match Json::parse(&text) {
                Ok(j) => j,
                Err(e) => {
                    error!("Couldn't parse element {id}: {e}");
                    errors.push((id, e.to_string()));
                    continue;
                }
            };
            match parse_advancement(&parser, &id, &json) {
                Ok(a) => parsed.push(a),
                Err(e) => {
                    error!("Couldn't parse element {id}: {e}");
                    errors.push((id, e));
                }
            }
        }
        let n = parsed.len();
        let mut tree = Advancements::build(parsed);
        tree.errors = errors;
        let staged: usize = tree.by_trigger.iter().filter(|(t, _)| !criteria::FIRED.contains(&t.as_str())).map(|(_, v)| v.len()).sum();
        info!("Loaded {} advancements ({} not in the tree, {staged} criteria of triggers Kiln does not fire)", tree.len(), n - tree.len());
        tree
    }

    /// `AdvancementTree.addAll` (advancements whose parent is not in the tree are dropped),
    /// then `repositionNodes`.
    fn build(mut pending: Vec<Advancement>) -> Advancements {
        let mut t = Advancements::default();
        loop {
            let before = pending.len();
            pending.retain(|a| {
                let parent = match &a.parent {
                    None => None,
                    Some(p) => match t.by_id.get(p) {
                        Some(&i) => Some(i),
                        None => return true,
                    },
                };
                let i = t.list.len();
                t.by_id.insert(a.id.clone(), i);
                t.list.push(a.clone());
                t.parent.push(parent);
                t.children.push(Vec::new());
                if let Some(p) = parent {
                    t.children[p].push(i);
                }
                false
            });
            if pending.is_empty() {
                break;
            }
            if pending.len() == before {
                error!("Couldn't load advancements: {:?}", pending.iter().map(|a| a.id.as_str()).collect::<Vec<_>>());
                break;
            }
        }
        t.position = vec![(0.0, 0.0); t.list.len()];
        for i in 0..t.list.len() {
            if t.parent[i].is_none() && t.list[i].display.is_some() {
                layout::run(&mut t, i);
            }
        }
        for (i, a) in t.list.iter().enumerate() {
            for (c, (_, crit)) in a.criteria.iter().enumerate() {
                t.by_trigger.entry(crit.trigger_id.clone()).or_default().push((i, c));
            }
        }
        t.encoded = (0..t.list.len()).map(|i| t.encode(i)).collect();
        t
    }

    /// `ClientboundUpdateAdvancementsPacket.PositionedAdvancement`: the holder (id and
    /// `Advancement.STREAM_CODEC`) and the node's position.
    fn encode(&self, i: usize) -> Bytes {
        let a = &self.list[i];
        let mut b = BytesMut::with_capacity(128);
        b.put_string(&a.id);
        match &a.parent {
            Some(p) => {
                b.put_u8(1);
                b.put_string(p);
            }
            None => b.put_u8(0),
        }
        match &a.display {
            Some(d) => {
                b.put_u8(1);
                d.title.write_network(&mut b);
                d.description.write_network(&mut b);
                d.icon.write(&mut b);
                b.put_varint(d.frame as i32);
                let flags = d.background.is_some() as i32 | (d.show_toast as i32) << 1 | (d.hidden as i32) << 2;
                bytes::BufMut::put_i32(&mut b, flags);
                if let Some(bg) = &d.background {
                    b.put_string(bg);
                }
            }
            None => b.put_u8(0),
        }
        b.put_varint(a.requirements.len() as i32);
        for group in &a.requirements {
            b.put_varint(group.len() as i32);
            for &c in group {
                b.put_string(&a.criteria[c].0);
            }
        }
        b.put_u8(a.sends_telemetry_event as u8);
        let (x, y) = self.position[i];
        bytes::BufMut::put_f32(&mut b, x);
        bytes::BufMut::put_f32(&mut b, y);
        b.freeze()
    }
}

trait PutU8 {
    fn put_u8(&mut self, v: u8);
}

impl PutU8 for BytesMut {
    fn put_u8(&mut self, v: u8) {
        bytes::BufMut::put_u8(self, v);
    }
}

/// A text component as a compound (a bare string becomes `{text}`).
fn compound(t: &Tag) -> Tag {
    match t {
        Tag::Compound(_) => t.clone(),
        Tag::String(s) => Tag::Compound(vec![("text".into(), Tag::String(s.clone()))]),
        other => Tag::Compound(vec![("text".into(), Tag::String(String::new())), ("extra".into(), Tag::List(vec![other.clone()]))]),
    }
}

fn set(t: &mut Tag, key: &str, v: Tag) {
    if let Tag::Compound(f) = t {
        f.retain(|(k, _)| k != key);
        f.push((key.into(), v));
    }
}

/// Appends a sibling (NBT lists hold one type: every sibling becomes a compound).
fn push_extra(t: &mut Tag, v: Tag) {
    if let Tag::Compound(f) = t {
        match f.iter_mut().find(|(k, _)| k == "extra") {
            Some((_, Tag::List(items))) => {
                for i in items.iter_mut() {
                    *i = compound(i);
                }
                items.push(compound(&v));
            }
            _ => f.push(("extra".into(), Tag::List(vec![compound(&v)]))),
        }
    }
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        if path.is_dir() {
            collect(&path, &format!("{prefix}{name}/"), out);
        } else if let Some(stem) = name.strip_suffix(".json") {
            out.push((format!("{prefix}{stem}"), path));
        }
    }
}

fn component(j: &Json, what: &str) -> Result<Tag, String> {
    kiln_loot::text::parse(j).map(kiln_item::Text::into_nbt).map_err(|e| format!("{what}: {e}"))
}

/// `Advancement.CODEC` with its validation.
fn parse_advancement(p: &kiln_loot::parse::Parser, id: &str, j: &Json) -> Result<Advancement, String> {
    let parent = match j.get("parent") {
        None => None,
        Some(v) => Some(v.as_str().and_then(kiln_item::Identifier::parse).ok_or("parent: not a valid identifier")?.to_string()),
    };
    let display = match j.get("display") {
        None => None,
        Some(d) => {
            let icon_json = d.get("icon").ok_or("display: No key icon")?;
            let icon = ItemStackTemplate::from_value(&icon_json.to_value()).map_err(|e| format!("display.icon: {}", e.0))?;
            let frame = match d.get("frame").and_then(Json::as_str).unwrap_or("task") {
                "task" => Frame::Task,
                "challenge" => Frame::Challenge,
                "goal" => Frame::Goal,
                other => return Err(format!("display.frame: unknown frame {other}")),
            };
            Some(Display {
                icon,
                title: component(d.get("title").ok_or("display: No key title")?, "display.title")?,
                description: component(d.get("description").ok_or("display: No key description")?, "display.description")?,
                frame,
                background: d.get("background").and_then(Json::as_str).and_then(kiln_item::Identifier::parse).map(|i| i.to_string()),
                show_toast: d.get("show_toast").and_then(Json::as_bool).unwrap_or(true),
                announce_to_chat: d.get("announce_to_chat").and_then(Json::as_bool).unwrap_or(true),
                hidden: d.get("hidden").and_then(Json::as_bool).unwrap_or(false),
            })
        }
    };
    let Some(Json::Obj(crit_json)) = j.get("criteria") else { return Err("No key criteria".into()) };
    if crit_json.is_empty() {
        return Err("Advancement criteria cannot be empty".into());
    }
    let mut criteria = Vec::new();
    for (name, c) in crit_json {
        let crit = Criterion::parse(p, c).map_err(|e| format!("criteria.{name}: {e}"))?;
        criteria.push((name.clone(), crit));
    }
    let requirements = match j.get("requirements") {
        None => (0..criteria.len()).map(|i| vec![i]).collect(),
        Some(Json::Arr(groups)) => {
            let mut out = Vec::new();
            for g in groups {
                let Some(names) = g.as_array() else { return Err("requirements: not a list of lists".into()) };
                let mut group = Vec::new();
                for n in names {
                    let n = n.as_str().ok_or("requirements: not a string")?;
                    let i = criteria.iter().position(|(c, _)| c == n).ok_or_else(|| format!("Advancement completion requirements did not exactly match specified criteria. Missing: [{n}]"))?;
                    group.push(i);
                }
                out.push(group);
            }
            let used: std::collections::HashSet<usize> = out.iter().flatten().copied().collect();
            if used.len() != criteria.len() {
                return Err("Advancement completion requirements did not exactly match specified criteria".into());
            }
            out
        }
        Some(_) => return Err("requirements: not a list".into()),
    };
    let mut rewards = Rewards::default();
    if let Some(r) = j.get("rewards") {
        rewards.experience = r.get("experience").and_then(Json::as_i32).unwrap_or(0);
        let ids = |key: &str| -> Vec<String> {
            r.get(key)
                .and_then(Json::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(|v| v.as_str().and_then(kiln_item::Identifier::parse).map(|i| i.to_string()))
                .collect()
        };
        rewards.loot = ids("loot");
        rewards.recipes = ids("recipes");
        rewards.function = r.get("function").and_then(Json::as_str).and_then(kiln_item::Identifier::parse).map(|i| i.to_string());
    }
    Ok(Advancement {
        id: id.to_owned(),
        parent,
        display,
        criteria,
        requirements,
        rewards,
        sends_telemetry_event: j.get("sends_telemetry_event").and_then(Json::as_bool).unwrap_or(false),
    })
}

/// `TreeNodePosition`: the Walker-style layout vanilla computes on the server. Only nodes with
/// a display take part (their undisplayed descendants' children are adopted).
mod layout {
    use super::Advancements;

    struct Node {
        adv: usize,
        parent: Option<usize>,
        previous_sibling: Option<usize>,
        child_index: i32,
        children: Vec<usize>,
        ancestor: usize,
        thread: Option<usize>,
        x: i32,
        y: f32,
        modifier: f32,
        change: f32,
        shift: f32,
    }

    struct Tree<'a> {
        t: &'a Advancements,
        nodes: Vec<Node>,
    }

    impl Tree<'_> {
        fn new_node(&mut self, adv: usize, parent: Option<usize>, previous: Option<usize>, child_index: i32, x: i32) -> usize {
            let me = self.nodes.len();
            self.nodes.push(Node {
                adv,
                parent,
                previous_sibling: previous,
                child_index,
                children: Vec::new(),
                ancestor: me,
                thread: None,
                x,
                y: -1.0,
                modifier: 0.0,
                change: 0.0,
                shift: 0.0,
            });
            let mut last = None;
            for &c in &self.t.children[adv] {
                last = self.add_child(me, c, last);
            }
            me
        }

        fn add_child(&mut self, me: usize, adv: usize, previous: Option<usize>) -> Option<usize> {
            if self.t.list[adv].display.is_some() {
                let index = self.nodes[me].children.len() as i32 + 1;
                let x = self.nodes[me].x + 1;
                let n = self.new_node(adv, Some(me), previous, index, x);
                self.nodes[me].children.push(n);
                Some(n)
            } else {
                let mut previous = previous;
                for &c in &self.t.children[adv] {
                    previous = self.add_child(me, c, previous);
                }
                previous
            }
        }

        fn first_walk(&mut self, n: usize) {
            if self.nodes[n].children.is_empty() {
                self.nodes[n].y = match self.nodes[n].previous_sibling {
                    Some(p) => self.nodes[p].y + 1.0,
                    None => 0.0,
                };
                return;
            }
            let mut default_ancestor: Option<usize> = None;
            for c in self.nodes[n].children.clone() {
                self.first_walk(c);
                default_ancestor = Some(self.apportion(c, default_ancestor.unwrap_or(c)));
            }
            self.execute_shifts(n);
            let first = self.nodes[n].children[0];
            let last = *self.nodes[n].children.last().unwrap();
            let mid = (self.nodes[first].y + self.nodes[last].y) / 2.0;
            match self.nodes[n].previous_sibling {
                Some(p) => {
                    self.nodes[n].y = self.nodes[p].y + 1.0;
                    self.nodes[n].modifier = self.nodes[n].y - mid;
                }
                None => self.nodes[n].y = mid,
            }
        }

        fn second_walk(&mut self, n: usize, modifier: f32, depth: i32, mut min: f32) -> f32 {
            self.nodes[n].y += modifier;
            self.nodes[n].x = depth;
            if self.nodes[n].y < min {
                min = self.nodes[n].y;
            }
            let m = self.nodes[n].modifier;
            for c in self.nodes[n].children.clone() {
                min = self.second_walk(c, modifier + m, depth + 1, min);
            }
            min
        }

        fn third_walk(&mut self, n: usize, d: f32) {
            self.nodes[n].y += d;
            for c in self.nodes[n].children.clone() {
                self.third_walk(c, d);
            }
        }

        fn execute_shifts(&mut self, n: usize) {
            let (mut shift, mut change) = (0.0f32, 0.0f32);
            for &c in self.nodes[n].children.clone().iter().rev() {
                let node = &mut self.nodes[c];
                node.y += shift;
                node.modifier += shift;
                change += node.change;
                shift += node.shift + change;
            }
        }

        fn previous_or_thread(&self, n: usize) -> Option<usize> {
            self.nodes[n].thread.or_else(|| self.nodes[n].children.first().copied())
        }

        fn next_or_thread(&self, n: usize) -> Option<usize> {
            self.nodes[n].thread.or_else(|| self.nodes[n].children.last().copied())
        }

        fn apportion(&mut self, me: usize, default_ancestor: usize) -> usize {
            let Some(prev) = self.nodes[me].previous_sibling else { return default_ancestor };
            let mut default_ancestor = default_ancestor;
            let parent = self.nodes[me].parent.expect("a sibling has a parent");
            let (mut inner_right, mut outer_right) = (me, me);
            let mut inner_left = prev;
            let mut outer_left = self.nodes[parent].children[0];
            let mut s_ir = self.nodes[me].modifier;
            let mut s_or = self.nodes[me].modifier;
            let mut s_il = self.nodes[inner_left].modifier;
            let mut s_ol = self.nodes[outer_left].modifier;
            while let (Some(il), Some(ir)) = (self.next_or_thread(inner_left), self.previous_or_thread(inner_right)) {
                inner_left = il;
                inner_right = ir;
                outer_left = self.previous_or_thread(outer_left).expect("outer left contour");
                outer_right = self.next_or_thread(outer_right).expect("outer right contour");
                self.nodes[outer_right].ancestor = me;
                let shift = self.nodes[inner_left].y + s_il - (self.nodes[inner_right].y + s_ir) + 1.0;
                if shift > 0.0 {
                    let a = self.get_ancestor(inner_left, me, default_ancestor);
                    self.move_subtree(a, me, shift);
                    s_ir += shift;
                    s_or += shift;
                }
                s_il += self.nodes[inner_left].modifier;
                s_ir += self.nodes[inner_right].modifier;
                s_ol += self.nodes[outer_left].modifier;
                s_or += self.nodes[outer_right].modifier;
            }
            if self.next_or_thread(inner_left).is_some() && self.next_or_thread(outer_right).is_none() {
                self.nodes[outer_right].thread = self.next_or_thread(inner_left);
                self.nodes[outer_right].modifier += s_il - s_or;
            } else {
                if self.previous_or_thread(inner_right).is_some() && self.previous_or_thread(outer_left).is_none() {
                    self.nodes[outer_left].thread = self.previous_or_thread(inner_right);
                    self.nodes[outer_left].modifier += s_ir - s_ol;
                }
                default_ancestor = me;
            }
            default_ancestor
        }

        fn move_subtree(&mut self, from: usize, to: usize, shift: f32) {
            let subtrees = (self.nodes[to].child_index - self.nodes[from].child_index) as f32;
            if subtrees != 0.0 {
                self.nodes[to].change -= shift / subtrees;
                self.nodes[from].change += shift / subtrees;
            }
            let t = &mut self.nodes[to];
            t.shift += shift;
            t.y += shift;
            t.modifier += shift;
        }

        fn get_ancestor(&self, n: usize, me: usize, default: usize) -> usize {
            let a = self.nodes[n].ancestor;
            let parent = self.nodes[me].parent.expect("parent");
            if self.nodes[parent].children.contains(&a) { a } else { default }
        }
    }

    /// `TreeNodePosition.run` for a root with a display.
    pub fn run(t: &mut Advancements, root: usize) {
        let positions = {
            let mut tree = Tree { t: &*t, nodes: Vec::new() };
            let r = tree.new_node(root, None, None, 1, 0);
            tree.first_walk(r);
            let y = tree.nodes[r].y;
            let min = tree.second_walk(r, 0.0, 0, y);
            if min < 0.0 {
                tree.third_walk(r, -min);
            }
            tree.nodes.iter().map(|n| (n.adv, n.x as f32, n.y)).collect::<Vec<_>>()
        };
        for (adv, x, y) in positions {
            t.position[adv] = (x, y);
        }
    }
}

#[cfg(test)]
mod wp53_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn adv(id: &str, parent: Option<&str>, display: bool) -> Advancement {
        let crit = Criterion { trigger_id: "minecraft:impossible".into(), trigger: criteria::Trigger::Impossible, player: None };
        Advancement {
            id: id.into(),
            parent: parent.map(Into::into),
            display: display.then(|| Display {
                icon: ItemStackTemplate::new(1, 1),
                title: Tag::String(id.into()),
                description: Tag::String(String::new()),
                frame: Frame::Task,
                background: None,
                show_toast: true,
                announce_to_chat: true,
                hidden: false,
            }),
            criteria: vec![("c".into(), crit)],
            requirements: vec![vec![0]],
            rewards: Rewards::default(),
            sends_telemetry_event: false,
        }
    }

    #[test]
    fn tree_and_layout() {
        // A root with two children, the first having a child; children listed before parents
        // still attach.
        let t = Advancements::build(vec![
            adv("a:c", Some("a:b"), true),
            adv("a:root", None, true),
            adv("a:b", Some("a:root"), true),
            adv("a:d", Some("a:root"), true),
            adv("a:orphan", Some("a:missing"), true),
        ]);
        assert_eq!(t.len(), 4);
        let pos = |id: &str| t.position[t.get(id).unwrap()];
        assert_eq!(pos("a:root"), (0.0, 0.5));
        assert_eq!(pos("a:b"), (1.0, 0.0));
        assert_eq!(pos("a:c"), (2.0, 0.0));
        assert_eq!(pos("a:d"), (1.0, 1.0));
        // The decorated name encodes (siblings of mixed kinds become compounds).
        let name = t.list[t.get("a:root").unwrap()].name();
        let _ = name.to_nbt();
        assert_eq!(name.to_plain(), "chat.square_brackets[a:root]");
    }

    #[test]
    fn visibility_reaches_two_levels_below_a_done_node() {
        let chain = ["a:root", "a:1", "a:2", "a:3"];
        let mut list: Vec<Advancement> =
            chain.iter().enumerate().map(|(i, id)| adv(id, (i > 0).then(|| chain[i - 1]), true)).collect();
        list.push(adv("a:nodisplay", Some("a:root"), false));
        let t = std::sync::Arc::new(Advancements::build(list));
        let mut pa = progress::PlayerAdvancements::new(t.clone());
        assert!(pa.flush(true).is_none(), "nothing done: nothing visible");
        pa.award(t.get("a:root").unwrap(), 0, 0);
        let pkt = pa.flush(true).expect("the root and two levels below appear");
        let has = |id: &str| pkt.windows(id.len()).any(|w| w == id.as_bytes());
        assert!(has("a:root") && has("a:1") && has("a:2") && !has("a:3") && !has("a:nodisplay"));
        // A done advancement without a display is sent too (vanilla's done flag shows it).
        pa.award(t.get("a:nodisplay").unwrap(), 0, 0);
        let pkt = pa.flush(true).expect("update");
        assert!(pkt.windows(11).any(|w| w == b"a:nodisplay"));
    }

    #[test]
    fn vanilla_advancements_load() {
        let dir = crate::datapack_dir(None);
        if !dir.join("data/minecraft/advancement").is_dir() {
            return;
        }
        let loot = LootData::load_lenient(&dir).ok();
        let t = Advancements::load(&[dir.as_path()], loot.as_ref());
        assert_eq!(t.errors.first(), None);
        assert_eq!(t.len(), 1866, "every vanilla advancement is in the tree");
        let root = t.get("minecraft:story/root").unwrap();
        assert!(t.list[root].display.is_some());
        assert_eq!(t.position[root], (0.0, t.position[root].1));
        assert!(t.criteria_for("minecraft:inventory_changed").len() > 1000);
    }
}
