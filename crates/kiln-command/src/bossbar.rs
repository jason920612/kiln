//! Custom boss bars (`CustomBossEvents`): the bars `/bossbar` makes, which players see them,
//! the packets vanilla sends on each change and the `custom_boss_events` saved data.
//!
//! A bar keeps the UUIDs of its players across restarts; of those, the online ones see it
//! (`ServerBossEvent.players`). Hosts call [`BossBars::player_joined`] and
//! [`BossBars::player_left`], and send what [`BossBars::take_packets`] returns.

use crate::scoreboard::{JavaHashSet, java_string_hash};
use crate::text::Text;
use crate::types::Identifier;
use bytes::Bytes;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::hud::{self, BossBarColor, BossBarOverlay, BossEvent};
use uuid::Uuid;

/// `BossEvent.BossBarColor` names, by ordinal.
pub const COLOR_NAMES: [&str; 7] = ["pink", "blue", "red", "green", "yellow", "purple", "white"];
/// `BossEvent.BossBarOverlay` names, by ordinal.
pub const OVERLAY_NAMES: [&str; 5] = ["progress", "notched_6", "notched_10", "notched_12", "notched_20"];
const COLORS: [BossBarColor; 7] = [
    BossBarColor::Pink,
    BossBarColor::Blue,
    BossBarColor::Red,
    BossBarColor::Green,
    BossBarColor::Yellow,
    BossBarColor::Purple,
    BossBarColor::White,
];
const OVERLAYS: [BossBarOverlay; 5] = [
    BossBarOverlay::Progress,
    BossBarOverlay::Notched6,
    BossBarOverlay::Notched10,
    BossBarOverlay::Notched12,
    BossBarOverlay::Notched20,
];
/// `BossBarColor.getFormatting`, as chat color names.
const CHAT_COLORS: [&str; 7] = ["red", "blue", "dark_red", "green", "yellow", "dark_blue", "white"];

/// One bar (`CustomBossEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    pub id: Identifier,
    /// The id clients know the bar by.
    pub uuid: Uuid,
    pub name: Text,
    /// Index into [`COLOR_NAMES`].
    pub color: usize,
    /// Index into [`OVERLAY_NAMES`].
    pub overlay: usize,
    pub darken_screen: bool,
    pub play_boss_music: bool,
    pub create_world_fog: bool,
    pub visible: bool,
    pub value: i32,
    pub max: i32,
    pub progress: f32,
    /// Saved players (`CustomBossEvent.players`).
    players: Vec<Uuid>,
    /// Online players who see the bar (`ServerBossEvent.players`), in the order added.
    online: Vec<Uuid>,
}

impl BossBar {
    /// `getDisplayName`: `[name]` in the bar's color, hovering and inserting the id.
    pub fn display_name(&self) -> Text {
        let id = self.id.to_string();
        Text::translate("chat.square_brackets", vec![self.name.clone().into()])
            .color(CHAT_COLORS[self.color])
            .hover(Text::literal(&id))
            .insertion(id)
    }

    /// Online players who see the bar (`getPlayers`).
    pub fn online_players(&self) -> &[Uuid] {
        &self.online
    }

    fn flags(&self) -> u8 {
        let mut f = 0;
        if self.darken_screen {
            f |= hud::boss_flags::DARKEN_SCREEN;
        }
        if self.play_boss_music {
            f |= hud::boss_flags::PLAY_BOSS_MUSIC;
        }
        if self.create_world_fog {
            f |= hud::boss_flags::CREATE_WORLD_FOG;
        }
        f
    }

    /// `ClientboundBossEventPacket.createAddPacket`.
    fn add_packet(&self) -> Bytes {
        let name = self.name.to_nbt();
        hud::boss_event(
            self.uuid,
            &BossEvent::Add {
                name: &name,
                progress: self.progress,
                color: COLORS[self.color],
                overlay: OVERLAYS[self.overlay],
                flags: self.flags(),
            },
        )
    }

    fn remove_packet(&self) -> Bytes {
        hud::boss_event(self.uuid, &BossEvent::Remove)
    }

    /// `CustomBossEvent.pack`.
    fn to_nbt(&self) -> Tag {
        let mut f = vec![("Name".to_owned(), self.name.to_nbt())];
        if self.visible {
            f.push(("Visible".into(), Tag::Byte(1)));
        }
        if self.value != 0 {
            f.push(("Value".into(), Tag::Int(self.value)));
        }
        if self.max != 100 {
            f.push(("Max".into(), Tag::Int(self.max)));
        }
        if self.color != 6 {
            f.push(("Color".into(), Tag::String(COLOR_NAMES[self.color].into())));
        }
        if self.overlay != 0 {
            f.push(("Overlay".into(), Tag::String(OVERLAY_NAMES[self.overlay].into())));
        }
        for (key, v) in [
            ("DarkenScreen", self.darken_screen),
            ("PlayBossMusic", self.play_boss_music),
            ("CreateWorldFog", self.create_world_fog),
        ] {
            if v {
                f.push((key.into(), Tag::Byte(1)));
            }
        }
        let uuid = |u: &Uuid| {
            let b = u.as_u128();
            Tag::IntArray(vec![(b >> 96) as i32, (b >> 64) as i32, (b >> 32) as i32, b as i32])
        };
        f.push(("Players".into(), Tag::List(self.players.iter().map(uuid).collect())));
        Tag::Compound(f)
    }
}

/// `Mth.clamp(value / max, 0, 1)` in floats.
fn progress(value: i32, max: i32) -> f32 {
    let p = value as f32 / max as f32;
    if p < 0.0 {
        0.0
    } else if p > 1.0 {
        1.0
    } else {
        p
    }
}

/// All custom boss bars, in `HashMap` order by id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BossBars {
    bars: Vec<BossBar>,
    order: JavaHashSet,
    /// (recipient, packet) since the last [`take_packets`](Self::take_packets).
    packets: Vec<(Uuid, Bytes)>,
    dirty: bool,
    rng: u64,
}

/// `Identifier.hashCode`.
fn id_hash(id: &Identifier) -> i32 {
    31i32.wrapping_mul(java_string_hash(id.namespace())).wrapping_add(java_string_hash(id.path()))
}

impl BossBars {
    /// Packets per recipient, in order.
    pub fn take_packets(&mut self) -> Vec<(Uuid, Bytes)> {
        std::mem::take(&mut self.packets)
    }

    /// Whether the saved data changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// `getEvents()` in iteration order.
    pub fn bars(&self) -> Vec<&BossBar> {
        self.order.iter().filter_map(|id| self.bars.iter().find(|b| b.id.as_str() == id)).collect()
    }

    pub fn get(&self, id: &Identifier) -> Option<&BossBar> {
        self.bars.iter().find(|b| b.id == *id)
    }

    fn bar_mut(&mut self, id: &Identifier) -> Option<&mut BossBar> {
        self.bars.iter_mut().find(|b| b.id == *id)
    }

    /// `Mth.createInsecureUUID`: random bits with version 4.
    fn next_uuid(&mut self) -> Uuid {
        let mut next = || {
            self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let hi = (next() & !0xF000) | 0x4000;
        let lo = (next() & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        Uuid::from_u64_pair(hi, lo)
    }

    /// Seeds the client ids of new bars.
    pub fn seed(&mut self, seed: u64) {
        self.rng = seed;
    }

    /// `CustomBossEvents.create`: visible, white, 0 of 100. False if the id is taken.
    pub fn create(&mut self, id: &Identifier, name: Text) -> bool {
        if self.get(id).is_some() {
            return false;
        }
        let uuid = self.next_uuid();
        self.bars.push(BossBar {
            id: id.clone(),
            uuid,
            name,
            color: 6,
            overlay: 0,
            darken_screen: false,
            play_boss_music: false,
            create_world_fog: false,
            visible: true,
            value: 0,
            max: 100,
            progress: 0.0,
            players: Vec::new(),
            online: Vec::new(),
        });
        self.order.insert_hashed(id.as_str(), id_hash(id));
        self.dirty = true;
        true
    }

    /// `removeAllPlayers` then `CustomBossEvents.remove`.
    pub fn remove(&mut self, id: &Identifier) {
        let Some(i) = self.bars.iter().position(|b| b.id == *id) else { return };
        let bar = self.bars.remove(i);
        if bar.visible {
            for p in &bar.online {
                self.packets.push((*p, bar.remove_packet()));
            }
        }
        self.order.remove(id.as_str());
        self.dirty = true;
    }

    /// `ServerBossEvent.broadcast`: to the bar's players while it is visible.
    fn broadcast(&mut self, id: &Identifier, op: impl FnOnce(&BossBar) -> Bytes) {
        let Some(bar) = self.get(id) else { return };
        if bar.visible {
            let pkt = op(bar);
            let targets: Vec<Uuid> = bar.online.clone();
            self.packets.extend(targets.into_iter().map(|p| (p, pkt.clone())));
        }
    }

    fn set_progress(&mut self, id: &Identifier) {
        let Some(bar) = self.bar_mut(id) else { return };
        let p = progress(bar.value, bar.max);
        if p != bar.progress {
            bar.progress = p;
            self.broadcast(id, |b| hud::boss_event(b.uuid, &BossEvent::Progress(b.progress)));
        }
    }

    pub fn set_value(&mut self, id: &Identifier, value: i32) {
        let Some(bar) = self.bar_mut(id) else { return };
        bar.value = value;
        self.set_progress(id);
        self.dirty = true;
    }

    pub fn set_max(&mut self, id: &Identifier, max: i32) {
        let Some(bar) = self.bar_mut(id) else { return };
        bar.max = max;
        self.set_progress(id);
        self.dirty = true;
    }

    pub fn set_color(&mut self, id: &Identifier, color: usize) {
        let Some(bar) = self.bar_mut(id).filter(|b| b.color != color) else { return };
        bar.color = color;
        self.broadcast(id, style_packet);
        self.dirty = true;
    }

    pub fn set_overlay(&mut self, id: &Identifier, overlay: usize) {
        let Some(bar) = self.bar_mut(id).filter(|b| b.overlay != overlay) else { return };
        bar.overlay = overlay;
        self.broadcast(id, style_packet);
        self.dirty = true;
    }

    pub fn set_name(&mut self, id: &Identifier, name: Text) {
        let Some(bar) = self.bar_mut(id).filter(|b| b.name.to_nbt() != name.to_nbt()) else { return };
        bar.name = name;
        self.broadcast(id, |b| hud::boss_event(b.uuid, &BossEvent::Name(&b.name.to_nbt())));
        self.dirty = true;
    }

    /// `ServerBossEvent.setVisible`: shows or hides the bar for its players.
    pub fn set_visible(&mut self, id: &Identifier, visible: bool) {
        let Some(bar) = self.bar_mut(id).filter(|b| b.visible != visible) else { return };
        bar.visible = visible;
        let bar = bar.clone();
        for p in &bar.online {
            let pkt = if visible { bar.add_packet() } else { bar.remove_packet() };
            self.packets.push((*p, pkt));
        }
        self.dirty = true;
    }

    /// `ServerBossEvent.addPlayer` plus the saved set.
    fn add_player(&mut self, id: &Identifier, player: Uuid) {
        let Some(bar) = self.bar_mut(id) else { return };
        if !bar.online.contains(&player) {
            bar.online.push(player);
            if bar.visible {
                let pkt = bar.add_packet();
                self.packets.push((player, pkt));
            }
        }
        let Some(bar) = self.bar_mut(id) else { return };
        if !bar.players.contains(&player) {
            bar.players.push(player);
            self.dirty = true;
        }
    }

    /// `CustomBossEvent.removePlayer`.
    fn remove_player(&mut self, id: &Identifier, player: Uuid) {
        let Some(bar) = self.bar_mut(id) else { return };
        if let Some(i) = bar.online.iter().position(|p| *p == player) {
            bar.online.remove(i);
            if bar.visible {
                let pkt = bar.remove_packet();
                self.packets.push((player, pkt));
            }
        }
        let Some(bar) = self.bar_mut(id) else { return };
        if let Some(i) = bar.players.iter().position(|p| *p == player) {
            bar.players.remove(i);
            self.dirty = true;
        }
    }

    /// `CustomBossEvent.setPlayers`: the bar's players become `players` (online players);
    /// returns whether that changed anything. Saved players not listed are dropped even
    /// when offline.
    pub fn set_players(&mut self, id: &Identifier, players: &[Uuid]) -> bool {
        let Some(bar) = self.get(id) else { return false };
        let to_remove: Vec<Uuid> = bar.players.iter().filter(|u| !players.contains(u)).copied().collect();
        let to_add: Vec<Uuid> = players.iter().filter(|u| !bar.players.contains(u)).copied().collect();
        for u in &to_remove {
            self.remove_player(id, *u);
            if let Some(bar) = self.bar_mut(id) {
                bar.players.retain(|p| p != u);
            }
        }
        for u in &to_add {
            self.add_player(id, *u);
        }
        let changed = !to_add.is_empty() || !to_remove.is_empty();
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// `CustomBossEvents.onPlayerConnect`: bars that saved the player show again.
    pub fn player_joined(&mut self, player: Uuid) {
        let ids: Vec<Identifier> = self.bars().iter().filter(|b| b.players.contains(&player)).map(|b| b.id.clone()).collect();
        for id in ids {
            self.add_player(&id, player);
        }
    }

    /// `CustomBossEvents.onPlayerDisconnect`: the player stays saved.
    pub fn player_left(&mut self, player: Uuid) {
        for bar in &mut self.bars {
            bar.online.retain(|p| *p != player);
        }
    }

    /// `CustomBossEvents` saved data: bars by id.
    pub fn to_nbt(&self) -> Tag {
        Tag::Compound(self.bars().into_iter().map(|b| (b.id.to_string(), b.to_nbt())).collect())
    }

    /// Loads saved bars (`CustomBossEvent.load`); nobody sees them until they join.
    pub fn load_nbt(&mut self, data: &Tag) {
        let Tag::Compound(entries) = data else { return };
        for (key, b) in entries {
            let Some(id) = Identifier::parse(key) else { continue };
            let name = b.get("Name").cloned().map_or_else(|| Text::literal(""), Text::raw);
            if !self.create(&id, name) {
                continue;
            }
            let bar = self.bar_mut(&id).expect("just created");
            let flag = |k: &str| b.get(k).and_then(Tag::as_i64).is_some_and(|v| v != 0);
            bar.visible = flag("Visible");
            bar.value = b.get("Value").and_then(Tag::as_i64).unwrap_or(0) as i32;
            bar.max = b.get("Max").and_then(Tag::as_i64).unwrap_or(100) as i32;
            bar.progress = progress(bar.value, bar.max);
            let name_of = |k: &str, names: &[&str]| b.get(k).and_then(Tag::as_str).and_then(|n| names.iter().position(|x| *x == n));
            bar.color = name_of("Color", &COLOR_NAMES).unwrap_or(6);
            bar.overlay = name_of("Overlay", &OVERLAY_NAMES).unwrap_or(0);
            bar.darken_screen = flag("DarkenScreen");
            bar.play_boss_music = flag("PlayBossMusic");
            bar.create_world_fog = flag("CreateWorldFog");
            for u in b.get("Players").and_then(Tag::as_list).unwrap_or(&[]) {
                if let Tag::IntArray(v) = u
                    && v.len() == 4
                {
                    let bits = v.iter().fold(0u128, |acc, &x| acc << 32 | u128::from(x as u32));
                    bar.players.push(Uuid::from_u128(bits));
                }
            }
        }
        self.packets.clear();
        self.dirty = false;
    }
}

fn style_packet(b: &BossBar) -> Bytes {
    hud::boss_event(b.uuid, &BossEvent::Style { color: COLORS[b.color], overlay: OVERLAYS[b.overlay] })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> Identifier {
        Identifier::parse(s).unwrap()
    }

    #[test]
    fn bars_list_in_hash_map_order() {
        // A `HashMap` keyed by `Identifier.hashCode` iterates these so (seen in jshell).
        let mut bars = BossBars::default();
        for s in ["minecraft:a", "kiln:bar", "minecraft:boss", "x:y"] {
            assert!(bars.create(&id(s), Text::literal(s)));
        }
        let order: Vec<String> = bars.bars().iter().map(|b| b.id.to_string()).collect();
        assert_eq!(order, ["x:y", "minecraft:a", "kiln:bar", "minecraft:boss"]);
    }

    #[test]
    fn players_see_visible_bars() {
        let mut bars = BossBars::default();
        let b = id("kiln:b");
        let (alice, bob) = (Uuid::from_u128(1), Uuid::from_u128(2));
        bars.create(&b, Text::literal("B"));
        assert!(bars.set_players(&b, &[alice]));
        assert!(!bars.set_players(&b, &[alice]));
        assert_eq!(bars.take_packets().len(), 1, "add for Alice");
        bars.set_value(&b, 50);
        assert_eq!(bars.get(&b).unwrap().progress, 0.5);
        assert_eq!(bars.take_packets().len(), 1, "progress");
        bars.set_visible(&b, false);
        bars.set_value(&b, 60);
        assert_eq!(bars.take_packets().len(), 1, "hidden: remove only");
        bars.set_visible(&b, true);
        assert!(bars.set_players(&b, &[bob]));
        assert_eq!(bars.take_packets().len(), 3, "add Alice again, remove Alice, add Bob");
        bars.player_left(bob);
        assert!(bars.get(&b).unwrap().online_players().is_empty());
        let nbt = bars.to_nbt();
        let mut loaded = BossBars::default();
        loaded.load_nbt(&nbt);
        assert_eq!(loaded.to_nbt(), nbt);
        loaded.player_joined(bob);
        assert_eq!(loaded.take_packets().len(), 1, "saved player sees it on join");
        assert_eq!(loaded.get(&b).unwrap().display_name().to_plain(), "[B]");
    }
}
