//! Display entities (`Display`: block, item and text displays): decoration the server stores and sends and the
//! client draws. They do nothing on the server but keep their data (transformation, interpolation, billboard,
//! brightness, shadow, size, glow color and the content), read and written the way the game's codecs do.

use crate::entity::{Entity, EntityKind};
use crate::ext_entity::EntityExt;
use crate::level::EntityLevel;
use crate::persist::{Input, Output};
use kiln_item::ItemStack;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

/// `Display$BillboardConstraints` names by id.
const BILLBOARDS: [&str; 4] = ["fixed", "vertical", "horizontal", "center"];

/// `ItemDisplayContext` names by id.
const CONTEXTS: [&str; 10] =
    ["none", "thirdperson_lefthand", "thirdperson_righthand", "firstperson_lefthand", "firstperson_righthand", "head", "gui", "ground", "fixed", "on_shelf"];

/// The data every display has.
#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    /// `DATA_TRANSFORMATION_INTERPOLATION_START_DELTA_TICKS_ID` (`start_interpolation`).
    pub start_delta: i32,
    /// `interpolation_duration`.
    pub duration: i32,
    /// `teleport_duration` (0 to 59).
    pub pos_rot_duration: i32,
    pub translation: [f32; 3],
    pub left_rotation: [f32; 4],
    pub scale: [f32; 3],
    pub right_rotation: [f32; 4],
    pub billboard: u8,
    /// The packed brightness override, -1 for none.
    pub brightness: i32,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color: i32,
}

impl Default for Common {
    fn default() -> Self {
        Common {
            start_delta: 0,
            duration: 0,
            pos_rot_duration: 0,
            translation: [0.0; 3],
            left_rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            right_rotation: [0.0, 0.0, 0.0, 1.0],
            billboard: 0,
            brightness: -1,
            view_range: 1.0,
            shadow_radius: 0.0,
            shadow_strength: 1.0,
            width: 0.0,
            height: 0.0,
            glow_color: -1,
        }
    }
}

/// What a display shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    Block { state: u16 },
    Item { stack: ItemStack, context: u8 },
    Text { text: Tag, line_width: i32, background: i32, opacity: i8, flags: u8 },
}

#[derive(Clone, Debug)]
pub struct DisplayEntity {
    pub common: Common,
    pub content: Content,
    /// A text with selectors or scores in it, as read, until the simulation has resolved it (the text shown is empty
    /// meanwhile, and what is saved is this).
    pub unresolved: Option<Tag>,
    /// The request for its resolution has been made.
    asked: bool,
}

/// Whether a text component has contents that need the level to be turned into text.
fn needs_resolving(text: &Tag) -> bool {
    use kiln_command::component::{Component, Contents, TranslateArg};
    fn walk(c: &Component) -> bool {
        let own = match &c.content {
            Contents::Selector { .. } | Contents::Score { .. } | Contents::Nbt(_) => true,
            Contents::Translate { with, .. } => with.iter().any(|a| matches!(a, TranslateArg::Component(c) if walk(c))),
            _ => false,
        };
        own || c.extra.iter().any(walk)
    }
    kiln_command::component::decode(text).is_ok_and(|c| walk(&c))
}

/// The text of display `e` is `text` (the simulation resolved it).
pub fn set_text(e: &mut Entity, text: Tag) {
    if let Some(d) = crate::ext_entity::get_mut::<DisplayEntity>(e) {
        if let Content::Text { text: t, .. } = &mut d.content {
            *t = text;
        }
        d.unresolved = None;
    }
}

/// Whether `name` is a display entity type.
pub fn is_display(name: &str) -> bool {
    matches!(name, "minecraft:block_display" | "minecraft:item_display" | "minecraft:text_display")
}

fn floats<const N: usize>(tag: &Tag) -> Option<[f32; N]> {
    let list = tag.as_list()?;
    if list.len() != N {
        return None;
    }
    let mut out = [0.0f32; N];
    for (o, t) in out.iter_mut().zip(list) {
        *o = t.as_f64()? as f32;
    }
    Some(out)
}

/// `ExtraCodecs.QUATERNIONF`: four components, or an axis and an angle.
fn quaternion(tag: &Tag) -> Option<[f32; 4]> {
    if let Some(q) = floats::<4>(tag) {
        return Some(q);
    }
    // `AxisAngle4f` -> `Quaternionf`.
    let axis = floats::<3>(tag.get("axis")?)?;
    let angle = tag.get("angle")?.as_f64()? as f32;
    let half = angle / 2.0;
    let sin = (half as f64).sin() as f32;
    let inv = 1.0 / (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    // (`Math.cosFromSin`: the cosine from the sine, with the sign of the cosine of the angle.)
    let cos = cos_from_sin(sin, half);
    Some([axis[0] * inv * sin, axis[1] * inv * sin, axis[2] * inv * sin, cos])
}

/// JOML's `Math.cosFromSin(sin, angle)`.
fn cos_from_sin(sin: f32, angle: f32) -> f32 {
    let cos = (1.0 - sin * sin).max(0.0).sqrt();
    let a = angle as f64 + std::f64::consts::FRAC_PI_2;
    let b = a - (a / (2.0 * std::f64::consts::PI)).trunc() * (2.0 * std::f64::consts::PI);
    if b < 0.0 {
        return -cos;
    }
    if b < std::f64::consts::PI { cos } else { -cos }
}

/// `Transformation.EXTENDED_CODEC`: the four parts, or a 4 by 4 matrix (row by row).
fn transformation(tag: &Tag) -> Option<([f32; 3], [f32; 4], [f32; 3], [f32; 4])> {
    if let Tag::Compound(_) = tag {
        let translation = floats::<3>(tag.get("translation")?)?;
        let left = quaternion(tag.get("left_rotation")?)?;
        let scale = floats::<3>(tag.get("scale")?)?;
        let right = quaternion(tag.get("right_rotation")?)?;
        return Some((translation, left, scale, right));
    }
    let m = floats::<16>(tag)?;
    decompose(m)
}

/// `Transformation(Matrix4f).getDecomposed`: the matrix read row by row. Only a matrix that is a translation
/// and an axis-aligned scale (a diagonal 3 by 3 part) is taken apart; others are not supported (`None`).
fn decompose(m: [f32; 16]) -> Option<([f32; 3], [f32; 4], [f32; 3], [f32; 4])> {
    let at = |r: usize, c: usize| m[r * 4 + c];
    // `Matrix4f` is built column-major from the list read row by row (`Matrix4f.set`/transpose).
    let off_diagonal = [at(0, 1), at(0, 2), at(1, 0), at(1, 2), at(2, 0), at(2, 1), at(3, 0), at(3, 1), at(3, 2)];
    if off_diagonal.iter().any(|v| *v != 0.0) || at(3, 3) == 0.0 {
        return None;
    }
    let f = 1.0 / at(3, 3);
    if f != 1.0 {
        return None;
    }
    // (The decomposition leaves the right rotation with negative zeros.)
    Some(([at(0, 3), at(1, 3), at(2, 3)], [0.0, 0.0, 0.0, 1.0], [at(0, 0), at(1, 1), at(2, 2)], [-0.0, -0.0, -0.0, 1.0]))
}

/// `Brightness.CODEC`: block and sky light, 0 to 15 each.
fn brightness(tag: &Tag) -> Option<i32> {
    let b = tag.get("block")?.as_i64()?;
    let s = tag.get("sky")?.as_i64()?;
    if !(0..=15).contains(&b) || !(0..=15).contains(&s) {
        return None;
    }
    Some((b as i32) << 4 | (s as i32) << 20)
}

/// Whether two vectors differ in a bit (a negative zero is not a zero: `Vector3f.equals` compares floats like `Float.compare`).
fn differs(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).any(|(x, y)| x.to_bits() != y.to_bits())
}

fn vec_tag(v: &[f32]) -> Tag {
    Tag::List(v.iter().map(|&f| Tag::Float(f + 0.0)).collect())
}

/// A block state as the game's codec reads it: a block id (its default state), or `{id, properties}` where an
/// unknown property or value is ignored; anything else is no state (`None`).
fn block_state(tag: &Tag) -> Option<u16> {
    if let Some(name) = tag.as_str() {
        return Some(kiln_data::blocks_types::block_by_name(&resource(name)?)?.default);
    }
    let Tag::Compound(_) = tag else { return None };
    let block = kiln_data::blocks_types::block_by_name(&resource(tag.get("id")?.as_str()?)?)?;
    let mut state = block.default;
    if let Some(Tag::Compound(props)) = tag.get("properties") {
        for (k, v) in props {
            if let Some(v) = v.as_str() {
                state = block.with_property(state, k, v).unwrap_or(state);
            }
        }
    }
    Some(state)
}

/// A `ResourceLocation` as written: `minecraft:` is implied. `None` for one with characters it may not have.
fn resource(s: &str) -> Option<String> {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ok = |c: char, slash: bool| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.') || (slash && c == '/');
    if ns.is_empty() || path.is_empty() || !ns.chars().all(|c| ok(c, false)) || !path.chars().all(|c| ok(c, true)) {
        return None;
    }
    Some(format!("{ns}:{path}"))
}

/// A text component as the game's codec reads and writes it.
fn component(tag: &Tag) -> Option<Tag> {
    kiln_command::component::decode(tag).ok().map(|c| c.to_nbt())
}

pub fn load(name: &str, r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let mut c = Common::default();
    if let Some((t, l, s, rr)) = r.get("transformation").and_then(transformation) {
        c.translation = t;
        c.left_rotation = l;
        c.scale = s;
        c.right_rotation = rr;
    }
    c.duration = r.int_or("interpolation_duration", 0);
    c.start_delta = r.int_or("start_interpolation", 0);
    c.pos_rot_duration = r.int_or("teleport_duration", 0).clamp(0, 59);
    c.billboard = r.get("billboard").and_then(Tag::as_str).and_then(|s| BILLBOARDS.iter().position(|b| *b == s)).unwrap_or(0) as u8;
    c.view_range = r.float_or("view_range", 1.0);
    c.shadow_radius = r.float_or("shadow_radius", 0.0);
    c.shadow_strength = r.float_or("shadow_strength", 1.0);
    c.width = r.float_or("width", 0.0);
    c.height = r.float_or("height", 0.0);
    c.glow_color = r.int_or("glow_color_override", -1);
    c.brightness = r.get("brightness").and_then(brightness).unwrap_or(-1);
    let content = match name {
        "minecraft:block_display" => Content::Block { state: r.get("block_state").and_then(block_state).unwrap_or(kiln_data::blocks::default_state::AIR) },
        "minecraft:item_display" => Content::Item {
            stack: r.get("item").and_then(|t| ItemStack::from_nbt(t).ok()).filter(|s| !s.is_empty()).unwrap_or_else(ItemStack::empty),
            context: r.get("item_display").and_then(Tag::as_str).and_then(|s| CONTEXTS.iter().position(|b| *b == s)).unwrap_or(0) as u8,
        },
        _ => {
            let line_width = r.int_or("line_width", 200);
            let opacity = r.byte_or("text_opacity", -1);
            let background = r.int_or("background", 1073741824);
            let mut flags = 0u8;
            for (key, bit) in [("shadow", 1u8), ("see_through", 2), ("default_background", 4)] {
                if r.bool_or(key, false) {
                    flags |= bit;
                }
            }
            match r.get("alignment").and_then(Tag::as_str) {
                Some("left") => flags |= 8,
                Some("right") => flags |= 16,
                _ => {}
            }
            let text = r.get("text").and_then(component).unwrap_or_else(|| Tag::String(String::new()));
            Content::Text { text, line_width, background, opacity, flags }
        }
    };
    // `ComponentUtils.resolve` at load: a text that needs the level to be resolved waits for the simulation.
    let mut content = content;
    let mut unresolved = None;
    if let Content::Text { text, .. } = &mut content
        && needs_resolving(text)
    {
        unresolved = Some(std::mem::replace(text, Tag::String(String::new())));
    }
    Some(Box::new(DisplayEntity { common: c, content, unresolved, asked: false }))
}

impl EntityExt for DisplayEntity {
    crate::entity_ext_boilerplate!();

    /// `Display.tick`: a display leaves a vehicle that is gone.
    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // A text waiting to be resolved asks for it once.
        if !self.asked
            && let Some(text) = &self.unresolved
        {
            self.asked = true;
            level.emit(crate::level::Event::ResolveText { entity: e.id, uuid: e.uuid, text: text.clone() });
        }
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        let c = &self.common;
        o.put(
            "transformation",
            Tag::Compound(vec![
                ("left_rotation".into(), vec_tag(&c.left_rotation)),
                ("translation".into(), vec_tag(&c.translation)),
                ("right_rotation".into(), vec_tag(&c.right_rotation)),
                ("scale".into(), vec_tag(&c.scale)),
            ]),
        );
        o.put("billboard", Tag::String(BILLBOARDS[c.billboard as usize % 4].into()));
        o.put("interpolation_duration", Tag::Int(c.duration));
        o.put("teleport_duration", Tag::Int(c.pos_rot_duration));
        o.put("view_range", Tag::Float(c.view_range));
        o.put("shadow_radius", Tag::Float(c.shadow_radius));
        o.put("shadow_strength", Tag::Float(c.shadow_strength));
        o.put("width", Tag::Float(c.width));
        o.put("height", Tag::Float(c.height));
        o.put("glow_color_override", Tag::Int(c.glow_color));
        if c.brightness != -1 {
            o.put("brightness", Tag::Compound(vec![("sky".into(), Tag::Int(c.brightness >> 20 & 15)), ("block".into(), Tag::Int(c.brightness >> 4 & 15))]));
        }
        match &self.content {
            Content::Block { state } => o.put("block_state", crate::persist::state_to_tag(*state)),
            Content::Item { stack, context } => {
                if !stack.is_empty() {
                    o.put("item", stack.to_nbt());
                }
                o.put("item_display", Tag::String(CONTEXTS[*context as usize % 10].into()));
            }
            Content::Text { text, line_width, background, opacity, flags } => {
                o.put("text", self.unresolved.clone().unwrap_or_else(|| text.clone()));
                o.put("line_width", Tag::Int(*line_width));
                o.put("background", Tag::Int(*background));
                o.put("text_opacity", Tag::Byte(*opacity));
                o.put("shadow", Tag::Byte((flags & 1 != 0) as i8));
                o.put("see_through", Tag::Byte((flags & 2 != 0) as i8));
                o.put("default_background", Tag::Byte((flags & 4 != 0) as i8));
                let align = if flags & 8 != 0 {
                    "left"
                } else if flags & 16 != 0 {
                    "right"
                } else {
                    "center"
                };
                o.put("alignment", Tag::String(align.into()));
            }
        }
    }

    /// The values that differ from their defaults (what a viewer gets when it starts tracking).
    fn entity_data(&self, _e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::{display, display_block_display, display_item_display, display_text_display};
        let c = &self.common;
        let n = Common::default();
        if c.start_delta != n.start_delta {
            d.set(display::TRANSFORMATION_INTERPOLATION_START_DELTA_TICKS, &DataValue::Int(c.start_delta));
        }
        if c.duration != n.duration {
            d.set(display::TRANSFORMATION_INTERPOLATION_DURATION, &DataValue::Int(c.duration));
        }
        if c.pos_rot_duration != n.pos_rot_duration {
            d.set(display::POS_ROT_INTERPOLATION_DURATION, &DataValue::Int(c.pos_rot_duration));
        }
        if differs(&c.translation, &n.translation) {
            d.set(display::TRANSLATION, &DataValue::Vector3(c.translation));
        }
        if differs(&c.scale, &n.scale) {
            d.set(display::SCALE, &DataValue::Vector3(c.scale));
        }
        if differs(&c.left_rotation, &n.left_rotation) {
            d.set(display::LEFT_ROTATION, &DataValue::Quaternion(c.left_rotation));
        }
        if differs(&c.right_rotation, &n.right_rotation) {
            d.set(display::RIGHT_ROTATION, &DataValue::Quaternion(c.right_rotation));
        }
        if c.billboard != n.billboard {
            d.set(display::BILLBOARD_RENDER_CONSTRAINTS, &DataValue::Byte(c.billboard as i8));
        }
        if c.brightness != n.brightness {
            d.set(display::BRIGHTNESS_OVERRIDE, &DataValue::Int(c.brightness));
        }
        if c.view_range != n.view_range {
            d.set(display::VIEW_RANGE, &DataValue::Float(c.view_range));
        }
        if c.shadow_radius != n.shadow_radius {
            d.set(display::SHADOW_RADIUS, &DataValue::Float(c.shadow_radius));
        }
        if c.shadow_strength != n.shadow_strength {
            d.set(display::SHADOW_STRENGTH, &DataValue::Float(c.shadow_strength));
        }
        if c.width != n.width {
            d.set(display::WIDTH, &DataValue::Float(c.width));
        }
        if c.height != n.height {
            d.set(display::HEIGHT, &DataValue::Float(c.height));
        }
        if c.glow_color != n.glow_color {
            d.set(display::GLOW_COLOR_OVERRIDE, &DataValue::Int(c.glow_color));
        }
        match &self.content {
            Content::Block { state } => {
                if *state != kiln_data::blocks::default_state::AIR {
                    d.set(display_block_display::BLOCK_STATE, &DataValue::BlockState(*state as i32));
                }
            }
            Content::Item { stack, context } => {
                if !stack.is_empty() {
                    let mut bytes = bytes::BytesMut::new();
                    stack.write_optional(&mut bytes);
                    d.set(display_item_display::ITEM_STACK, &DataValue::EncodedItemStack(bytes.freeze()));
                }
                if *context != 0 {
                    d.set(display_item_display::ITEM_DISPLAY, &DataValue::Byte(*context as i8));
                }
            }
            Content::Text { text, line_width, background, opacity, flags } => {
                if *text != Tag::String(String::new()) {
                    d.set(display_text_display::TEXT, &DataValue::Component(text.clone()));
                }
                if *line_width != 200 {
                    d.set(display_text_display::LINE_WIDTH, &DataValue::Int(*line_width));
                }
                if *background != 1073741824 {
                    d.set(display_text_display::BACKGROUND_COLOR, &DataValue::Int(*background));
                }
                if *opacity != -1 {
                    d.set(display_text_display::TEXT_OPACITY, &DataValue::Byte(*opacity));
                }
                if *flags != 0 {
                    d.set(display_text_display::STYLE_FLAGS, &DataValue::Byte(*flags as i8));
                }
            }
        }
    }
}

/// The display state of `e`, if it is a display.
pub fn get(e: &Entity) -> Option<&DisplayEntity> {
    crate::ext_entity::get::<DisplayEntity>(e)
}

/// Marks a display entity as one that does not collide or fall (`noPhysics`).
pub fn prepare(e: &mut Entity) {
    e.no_physics = true;
    if !matches!(e.kind, EntityKind::Ext(_)) {
        return;
    }
}
