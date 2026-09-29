//! `fetchprofile` (vanilla `FetchProfileCommand`): looks a game profile up by name or id, or
//! reads a player's or mannequin's own, and shows it with buttons that copy it, give a head or
//! summon a mannequin wearing it.
//!
//! Names and ids are resolved off the tick thread by the host ([`Host::fetch_profile`]); the
//! host reports the result with [`success_text`] or [`failure_text`] when it arrives.

use super::LEVEL_GAMEMASTERS;
use crate::arguments::ArgumentType;
use crate::dispatcher::{Dispatcher, argument, literal};
use crate::error::CommandError;
use crate::host::{Host, ProfileQuery, ResolvedProfile};
use crate::nbt_text::snbt;
use crate::selector::SelectorTarget;
use crate::text::{ClickEvent, Text};
use crate::tr;
use kiln_proto::nbt::Tag;

/// `ResolvableProfile.createResolved(profile)` in NBT (`ResolvableProfile.CODEC`, the full
/// form): `id` as an int array, `name`, and the properties when there are any.
pub fn profile_nbt(profile: &ResolvedProfile) -> Tag {
    let bits = profile.id.as_u128();
    let mut fields = vec![
        ("id".to_owned(), Tag::IntArray(vec![(bits >> 96) as i32, (bits >> 64) as i32, (bits >> 32) as i32, bits as i32])),
        ("name".to_owned(), Tag::String(profile.name.clone())),
    ];
    if !profile.properties.is_empty() {
        let list = profile
            .properties
            .iter()
            .map(|p| {
                let mut f = vec![("name".to_owned(), Tag::String(p.name.clone())), ("value".to_owned(), Tag::String(p.value.clone()))];
                if let Some(sig) = &p.signature {
                    f.push(("signature".to_owned(), Tag::String(sig.clone())));
                }
                Tag::Compound(f)
            })
            .collect();
        fields.push(("properties".to_owned(), Tag::List(list)));
    }
    Tag::Compound(fields)
}

/// The rest of a result message: the four buttons after `Resolved profile for ...: `.
fn buttons(profile: &ResolvedProfile) -> Text {
    let nbt = snbt(&profile_nbt(profile));
    // `Component.object(new PlayerSprite(profile, true))`, shown as `[<name> head]`.
    let head = Text::object(
        vec![
            ("object".to_owned(), Tag::String("player".to_owned())),
            ("player".to_owned(), profile_nbt(profile)),
        ],
        format!("[{} head]", profile.name),
    );
    let head_nbt = snbt(&head.to_nbt());
    let items = [
        tr!("commands.fetchprofile.copy_component").click(ClickEvent::CopyToClipboard(nbt.clone())),
        tr!("commands.fetchprofile.give_item").click(ClickEvent::RunCommand(format!("give @s minecraft:player_head[profile={nbt}]"))),
        tr!("commands.fetchprofile.summon_mannequin").click(ClickEvent::RunCommand(format!("summon minecraft:mannequin ~ ~ ~ {{profile:{nbt}}}"))),
        tr!("commands.fetchprofile.copy_text", head.color("white")).click(ClickEvent::CopyToClipboard(head_nbt)),
    ];
    // `ComponentUtils.formatList(list, SPACE, c -> wrapInSquareBrackets(c.withStyle(GREEN)))`.
    let mut out = Text::empty();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.extra.push(Text::literal(" "));
        }
        out.extra.push(item.color("green").bracketed());
    }
    out
}

/// `commands.fetchprofile.{name,id,entity}.success` (`key`) for `subject`.
pub fn success_text(key: &str, subject: Text, profile: &ResolvedProfile) -> Text {
    Text::translate(key, vec![subject.into(), buttons(profile).into()])
}

/// The failure of a lookup by name (`by_name`) or id: `commands.fetchprofile.{name,id}.failure`.
pub fn failure_text(query: &ProfileQuery) -> Text {
    match query {
        ProfileQuery::Name(name) => tr!("commands.fetchprofile.name.failure", Text::literal(name.as_str())),
        ProfileQuery::Id(id) => tr!("commands.fetchprofile.id.failure", Text::literal(id.to_string())),
    }
}

/// The subject of a lookup in its success message (`Component.literal(name)` /
/// `Component.translationArg(id)`).
pub fn subject_text(query: &ProfileQuery) -> Text {
    match query {
        ProfileQuery::Name(name) => Text::literal(name.as_str()),
        ProfileQuery::Id(id) => Text::literal(id.to_string()),
    }
}

/// `commands.fetchprofile.{name,id}.success` for a lookup that found `profile`.
pub fn lookup_success_text(query: &ProfileQuery, profile: &ResolvedProfile) -> Text {
    let key = match query {
        ProfileQuery::Name(_) => "commands.fetchprofile.name.success",
        ProfileQuery::Id(_) => "commands.fetchprofile.id.success",
    };
    success_text(key, subject_text(query), profile)
}

pub fn fetchprofile<S: Host + 'static>(d: &mut Dispatcher<S>) {
    d.register(
        literal("fetchprofile")
            .requires(LEVEL_GAMEMASTERS)
            .then(literal("name").then(argument("name", ArgumentType::greedy_string()).executes(|c, s: &mut S| {
                s.fetch_profile(ProfileQuery::Name(c.string("name").to_owned()));
                Ok(1)
            })))
            .then(literal("id").then(argument("id", ArgumentType::Uuid).executes(|c, s: &mut S| {
                let id = c.string("id").parse().expect("the argument parsed as a UUID");
                s.fetch_profile(ProfileQuery::Id(id));
                Ok(1)
            })))
            .then(literal("entity").then(argument("entity", ArgumentType::entity()).executes(|c, s: &mut S| {
                let entity = c.selector("entity").entity(s)?;
                let Some(profile) = s.entity_profile(&entity) else {
                    return Err(CommandError::new(tr!("commands.fetchprofile.no_profile", entity.display_name())));
                };
                let text = success_text("commands.fetchprofile.entity.success", entity.display_name(), &profile);
                s.send_success(text, false);
                Ok(1)
            }))),
    );
}
