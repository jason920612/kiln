//! Mannequin (`Mannequin`, an `Avatar`): a living entity with a player's looks and nothing of a mob. It has a skin
//! (a `ResolvableProfile`), the layers of the skin shown, a main hand, a pose, an optional description under its
//! name, and can be made immovable. It has no goals, never despawns, takes no leads or name tags, drops nothing
//! when it dies and is worth no experience; it falls, is pushed and hurt like any living entity.

use super::profile::Profile;
use crate::entity::Entity;
use crate::mob::attributes::Attr::*;
use crate::mob::ext::{self, Info, Kind, MobExt};
use crate::mob::interact::{Interactor, Outcome};
use crate::mob::{Category, MobData};
use crate::persist::{Input, Output};
use kiln_data::entities::{data, pose};
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::metadata::HumanoidArm;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub struct Mannequin;

pub static KIND: Mannequin = Mannequin;

/// `LivingEntity.createLivingAttributes`; the mob extras of the shared code are never read.
static INFO: Info = Info { sounds: Some("generic"), category: Category::Misc, ..Info::misc("minecraft:mannequin", &[(MaxHealth, 20.0)]) };

/// `PlayerModelPart` names by mask bit (`getMask`: `1 << id`).
const PARTS: [&str; 7] = ["cape", "jacket", "left_sleeve", "right_sleeve", "left_pants_leg", "right_pants_leg", "hat"];

/// All seven layers shown (`Mannequin.ALL_LAYERS`).
const ALL_LAYERS: u8 = 0x7f;

#[derive(Clone, Debug)]
pub struct State {
    /// The profile (`DATA_PROFILE`; `None`: `Static.EMPTY`).
    pub profile: Option<Profile>,
    /// `DATA_PLAYER_MODE_CUSTOMISATION`: the layers shown.
    pub layers: u8,
    pub left_handed: bool,
    /// One of `VALID_POSES` as a pose id.
    pub pose: i32,
    pub immovable: bool,
    /// The description as saved (`None`: the default text); `hide_description`.
    pub description: Option<Tag>,
    pub hide_description: bool,
}

impl Default for State {
    fn default() -> State {
        State { profile: None, layers: ALL_LAYERS, left_handed: false, pose: pose::STANDING, immovable: false, description: None, hide_description: false }
    }
}

fn st(m: &MobData) -> &State {
    ext::state::<State>(m).expect("mannequin state")
}

fn st_mut(m: &mut MobData) -> &mut State {
    ext::state_mut::<State>(m).expect("mannequin state")
}

/// `Pose.getSerializedName` of the poses a mannequin may have.
fn pose_name(p: i32) -> &'static str {
    match p {
        pose::CROUCHING => "crouching",
        pose::SWIMMING => "swimming",
        pose::FALL_FLYING => "fall_flying",
        pose::SLEEPING => "sleeping",
        _ => "standing",
    }
}

fn pose_of(name: &str) -> Option<i32> {
    Some(match name {
        "standing" => pose::STANDING,
        "crouching" => pose::CROUCHING,
        "swimming" => pose::SWIMMING,
        "fall_flying" => pose::FALL_FLYING,
        "sleeping" => pose::SLEEPING,
        _ => return None,
    })
}

/// `Mannequin.DEFAULT_DESCRIPTION` as a tag.
fn default_description() -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String("entity.minecraft.mannequin.label".into()))])
}

impl Kind for Mannequin {
    fn info(&self) -> &'static Info {
        &INFO
    }

    fn new_state(&self, _m: &mut MobData, _random: &mut dyn RandomSource) -> Option<Box<dyn MobExt>> {
        Some(Box::new(State::default()))
    }

    /// No goals.
    fn register_goals(&self, _m: &mut MobData) {}

    /// `Mannequin` is not a `Mob`: no leads, name tags or shears; nothing happens on a click.
    fn interact(&self, _e: &mut Entity, _m: &mut MobData, _level: &mut dyn crate::level::EntityLevel, _who: &Interactor, _stack: &ItemStack) -> Option<Outcome> {
        Some(Outcome::PASS)
    }

    fn despawns(&self) -> bool {
        false
    }

    /// `getBaseExperienceReward` of a `LivingEntity`.
    fn experience(&self, _e: &mut Entity, _m: &MobData) -> Option<i32> {
        Some(0)
    }

    /// `Avatar.getDefaultDimensions` by pose.
    fn dimensions(&self, m: &MobData, _base: (f32, f32, f32)) -> (f32, f32, f32) {
        match st(m).pose {
            pose::CROUCHING => (0.6, 1.5, 1.27),
            pose::SWIMMING | pose::FALL_FLYING => (0.6, 0.6, 0.4),
            pose::SLEEPING => (0.2, 0.2, 0.2),
            _ => (0.6, 1.8, 1.62),
        }
    }

    /// `isImmobile` / `isEffectiveAi`: an immovable mannequin takes no input and runs no AI.
    fn is_immobile(&self, m: &MobData) -> bool {
        st(m).immovable
    }

    fn load(&self, e: &mut Entity, m: &mut MobData, r: &mut Input) {
        let s = st_mut(m);
        if let Some(p) = r.get("profile").and_then(Profile::from_nbt) {
            s.profile = Some(p);
        }
        // `LAYERS_CODEC`: the hidden layers; unknown names fail the list (all layers shown).
        s.layers = ALL_LAYERS;
        if let Some(Tag::List(items)) = r.get("hidden_layers") {
            let mut hidden = 0u8;
            for t in items {
                match t.as_str().and_then(|n| PARTS.iter().position(|p| *p == n)) {
                    Some(i) => hidden |= 1 << i,
                    None => {}
                }
            }
            s.layers = ALL_LAYERS & !hidden;
        }
        s.left_handed = matches!(r.get("main_hand").and_then(Tag::as_str), Some("left"));
        s.pose = r.get("pose").and_then(Tag::as_str).and_then(pose_of).unwrap_or(pose::STANDING);
        s.immovable = r.bool_or("immovable", false);
        s.hide_description = r.bool_or("hide_description", false);
        s.description = match r.get("description") {
            Some(t) if kiln_item::Text::from_nbt(t.clone()).is_some() => Some(t.clone()),
            _ => None,
        };
        crate::mob::refresh_dimensions(e, m);
    }

    fn save(&self, _e: &Entity, m: &MobData, o: &mut Output) {
        let s = st(m);
        o.put("profile", s.profile.as_ref().map_or_else(|| Tag::Compound(Vec::new()), Profile::to_nbt));
        let hidden: Vec<Tag> = PARTS.iter().enumerate().filter(|(i, _)| s.layers & (1 << i) == 0).map(|(_, p)| Tag::String((*p).into())).collect();
        o.put("hidden_layers", Tag::List(hidden));
        o.put("main_hand", Tag::String(if s.left_handed { "left" } else { "right" }.into()));
        o.put("pose", Tag::String(pose_name(s.pose).into()));
        o.put("immovable", Tag::Byte(s.immovable as i8));
        // The description: absent when it is the default; `hide_description` when hidden.
        if s.hide_description {
            o.put("hide_description", Tag::Byte(1));
        } else if let Some(d) = s.description.as_ref().filter(|d| **d != default_description()) {
            o.put("description", d.clone());
        }
    }

    fn entity_data(&self, _e: &Entity, m: &MobData, d: &mut EntityData) {
        let s = st(m);
        if s.left_handed {
            d.set(data::avatar::PLAYER_MAIN_HAND, &DataValue::HumanoidArm(HumanoidArm::Left));
        }
        // (`Mannequin()` sets all layers: a value that differs from the `Avatar` default of 0.)
        if s.layers != 0 {
            d.set(data::avatar::PLAYER_MODE_CUSTOMISATION, &DataValue::Byte(s.layers as i8));
        }
        if let Some(p) = s.profile.as_ref().filter(|p| **p != Profile::default()) {
            let mut b = bytes::BytesMut::new();
            p.write(&mut b);
            d.set(data::mannequin::PROFILE, &DataValue::EncodedProfile(b.freeze()));
        }
        if s.immovable {
            d.set(data::mannequin::IMMOVABLE, &DataValue::Boolean(true));
        }
        let shown = if s.hide_description { None } else { Some(s.description.clone().unwrap_or_else(default_description)) };
        if shown != Some(default_description()) {
            d.set(data::mannequin::DESCRIPTION, &DataValue::OptionalComponent(shown));
        }
        if s.pose != pose::STANDING {
            d.set(data::entity::POSE, &DataValue::Pose(s.pose));
        }
    }
}
