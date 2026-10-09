//! Entity NBT as vanilla saves it in entity chunks (`Entity.save` / `Entity.load` with each
//! type's `addAdditionalSaveData` / `readAdditionalSaveData`), for the kinds this crate
//! simulates. Fields Kiln does not model stay in [`Entity::extra`] and are written back.

use crate::arrow::ArrowData;
use crate::blocks::{Tag as BlockTag, has_tag};
use crate::entity::{Entity, EntityKind};
use crate::falling_block::FallingBlockData;
use crate::item::ItemData;
use crate::math::Vec3;
use crate::projectile::{Throwable, ThrowableData};
use crate::tnt::TntData;
use crate::xp_orb::OrbData;
use kiln_item::ItemStack;
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;

/// Why a saved entity did not become a simulated one.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    /// A type this crate does not simulate: keep the compound as it is.
    NotSimulated,
    /// Vanilla would discard it on load (an item entity without an item).
    Discarded,
    /// Vanilla would fail to load it, or Kiln cannot read it: keep the compound as it is.
    Invalid(&'static str),
}

const THROWABLES: [Throwable; 6] = [
    Throwable::Snowball,
    Throwable::Egg,
    Throwable::EnderPearl,
    Throwable::SplashPotion,
    Throwable::LingeringPotion,
    Throwable::ExperienceBottle,
];

/// Whether entities of `type_name` are simulated (and so loaded by [`load`]).
pub fn is_simulated(type_name: &str) -> bool {
    matches!(
        type_name,
        "minecraft:item" | "minecraft:experience_orb" | "minecraft:falling_block" | "minecraft:tnt" | "minecraft:arrow" | "minecraft:spectral_arrow"
    ) || THROWABLES.iter().any(|t| t.type_name() == type_name)
        || crate::ext_entity::TYPES.contains(&type_name)
        || crate::ext_entity::boat::is_boat(type_name)
        || crate::ext_entity::minecart::is_minecart(type_name)
        || crate::mob::MobKind::by_name(type_name).is_some()
}

/// The `id` of a saved entity.
pub fn type_of(tag: &Tag) -> Option<&str> {
    tag.get("id")?.as_str()
}

/// `UUIDUtil.CODEC`: four ints, most significant first.
pub fn uuid_to_tag(uuid: u128) -> Tag {
    Tag::IntArray(vec![(uuid >> 96) as i32, (uuid >> 64) as i32, (uuid >> 32) as i32, uuid as i32])
}

pub fn uuid_from_tag(tag: &Tag) -> Option<u128> {
    match tag {
        Tag::IntArray(v) if v.len() == 4 => Some(v.iter().fold(0u128, |acc, &i| (acc << 32) | i as u32 as u128)),
        _ => None,
    }
}

/// `BlockState.CODEC`: the block id for a default state, else `{id, properties}`.
pub fn state_to_tag(state: u16) -> Tag {
    let block = kiln_data::blocks_types::block_of(state);
    if state == block.default || block.properties.is_empty() {
        return Tag::String(block.name.to_owned());
    }
    let props = block
        .properties
        .iter()
        .zip(block.property_indices(state))
        .map(|(p, i)| (p.name.to_owned(), Tag::String(p.values[i].to_owned())))
        .collect();
    Tag::Compound(vec![("id".into(), Tag::String(block.name.to_owned())), ("properties".into(), Tag::Compound(props))])
}

/// Reads [`state_to_tag`]'s forms and the older `{Name, Properties}`; `None` for an unknown
/// block (vanilla's codec fails and the caller's default applies).
pub fn state_from_tag(tag: &Tag) -> Option<u16> {
    if let Some(name) = tag.as_str() {
        return Some(kiln_data::blocks_types::block_by_name(name)?.default);
    }
    let name = tag.get("id").or_else(|| tag.get("Name"))?.as_str()?;
    let block = kiln_data::blocks_types::block_by_name(name)?;
    let mut state = block.default;
    if let Some(Tag::Compound(props)) = tag.get("properties").or_else(|| tag.get("Properties")) {
        for (k, v) in props {
            if let Some(v) = v.as_str() {
                state = block.with_property(state, k, v).unwrap_or(state);
            }
        }
    }
    Some(state)
}

/// A compound being read: remembers which keys the reader used, so the rest can be kept.
pub struct Input<'a> {
    pub(crate) fields: &'a [(String, Tag)],
    pub(crate) used: Vec<&'static str>,
}

impl<'a> Input<'a> {
    pub fn get(&mut self, key: &'static str) -> Option<&'a Tag> {
        self.used.push(key);
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `getDoubleOr` and friends: any numeric tag.
    pub fn num(&mut self, key: &'static str) -> Option<f64> {
        self.get(key).and_then(Tag::as_f64)
    }

    pub fn int_or(&mut self, key: &'static str, default: i32) -> i32 {
        self.num(key).map_or(default, |v| v as i64 as i32)
    }

    /// `getShortOr`: the value truncated to a short.
    pub fn short_or(&mut self, key: &'static str, default: i16) -> i32 {
        self.num(key).map_or(default, |v| v as i64 as i16) as i32
    }

    pub fn byte_or(&mut self, key: &'static str, default: i8) -> i8 {
        self.num(key).map_or(default, |v| v as i64 as i8)
    }

    pub fn bool_or(&mut self, key: &'static str, default: bool) -> bool {
        self.num(key).map_or(default, |v| v != 0.0)
    }

    pub fn float_or(&mut self, key: &'static str, default: f32) -> f32 {
        self.num(key).map_or(default, |v| v as f32)
    }

    pub fn uuid(&mut self, key: &'static str) -> Option<u128> {
        self.get(key).and_then(uuid_from_tag)
    }

    pub fn vec3(&mut self, key: &'static str) -> Option<[f64; 3]> {
        let list = self.get(key)?.as_list()?;
        if list.len() != 3 {
            return None;
        }
        let mut out = [0.0; 3];
        for (o, t) in out.iter_mut().zip(list) {
            *o = t.as_f64()?;
        }
        Some(out)
    }

    /// Fields no reader used.
    pub fn rest(&self) -> Vec<(String, Tag)> {
        self.fields.iter().filter(|(k, _)| !self.used.contains(&k.as_str())).cloned().collect()
    }
}

/// `EntityType.create` + `Entity.load`: the entity a saved compound describes, with network
/// id `id` and its own random seeded by `seed`. A compound without a `UUID` gets uuid 0 (the
/// caller assigns one).
pub fn load(tag: &Tag, id: i32, seed: i64) -> Result<Entity, LoadError> {
    let Tag::Compound(fields) = tag else { return Err(LoadError::Invalid("not a compound")) };
    let mut r = Input { fields, used: Vec::new() };
    let type_name = r.get("id").and_then(Tag::as_str).ok_or(LoadError::Invalid("no id"))?;
    let t = kiln_data::entities::by_name(type_name).ok_or(LoadError::NotSimulated)?;
    if !is_simulated(t.name) {
        return Err(LoadError::NotSimulated);
    }
    let kind = read_kind(t.name, &mut r)?;
    let mut e = Entity::new(t.name, id, 0, kind, seed);
    if let Some(kind) = crate::mob::MobKind::by_name(t.name) {
        crate::mob::persist::load(&mut e, kind, &mut r);
    }
    if let EntityKind::Item(d) = &mut e.kind {
        // `ItemEntity` constructor: the bob offset (the yaw is overwritten by `Rotation`).
        d.bob_offset = e.random.next_float() * std::f32::consts::PI * 2.0;
    }
    // `Leashable.readLeashData`: the lead looks for its holder on the first tick.
    if crate::leash::is_leashable(&e)
        && let Some(t) = r.get("leash")
    {
        e.leash = crate::leash::load(t).map(Box::new);
    }

    let pos = r.vec3("Pos").unwrap_or([0.0; 3]);
    let motion = r.vec3("Motion").unwrap_or([0.0; 3]).map(|v| if v.abs() > 10.0 { 0.0 } else { v });
    let rot = match r.get("Rotation").and_then(Tag::as_list) {
        Some([y, x]) => [y.as_f64().unwrap_or(0.0) as f32, x.as_f64().unwrap_or(0.0) as f32],
        _ => [0.0; 2],
    };
    if !pos.iter().all(|v| v.is_finite()) {
        return Err(LoadError::Invalid("Entity has invalid position"));
    }
    if !rot.iter().all(|v| v.is_finite()) {
        return Err(LoadError::Invalid("Entity has invalid rotation"));
    }
    e.delta = Vec3::new(motion[0], motion[1], motion[2]);
    e.needs_sync = true;
    e.set_pos(Vec3::new(
        pos[0].clamp(-3.0000512E7, 3.0000512E7),
        pos[1].clamp(-2.0E7, 2.0E7),
        pos[2].clamp(-3.0000512E7, 3.0000512E7),
    ));
    // `BlockAttachedEntity.setPos`: a leash knot sits on the middle of its block.
    if e.type_name == crate::leash::KNOT {
        let p = e.position();
        e.set_pos(Vec3::new(p.x.floor() + 0.5, p.y.floor() + 0.375, p.z.floor() + 0.5));
    }
    // `setRot` through `setYRot`/`setXRot`: both taken mod 360, the pitch clamped to 90.
    e.y_rot = rot[0] % 360.0;
    e.x_rot = (rot[1] % 360.0).clamp(-90.0, 90.0);
    e.set_old_pos_and_rot();
    e.fall_distance = r.num("fall_distance").unwrap_or(0.0);
    e.remaining_fire_ticks = r.short_or("Fire", 0);
    e.air_supply = r.int_or("Air", 300);
    e.on_ground = r.bool_or("OnGround", false);
    e.invulnerable = r.bool_or("Invulnerable", false);
    e.invulnerable_time = r.int_or("invulnerable_time", 0);
    e.uuid = r.uuid("UUID").unwrap_or(0);
    e.silent = r.bool_or("Silent", false);
    e.no_gravity = r.bool_or("NoGravity", false);
    e.ticks_frozen = r.int_or("TicksFrozen", 0);
    // `BlockAttachedEntity.readAdditionalSaveData`: a hanging entity hangs in the block `block_pos` names
    // (when it is near), and its box follows.
    let block_pos = r.get("block_pos");
    match e.type_name {
        "minecraft:item_frame" | "minecraft:glow_item_frame" => crate::ext_entity::item_frame::after_load(&mut e, block_pos),
        "minecraft:painting" => crate::ext_entity::painting::after_load(&mut e, block_pos),
        "minecraft:armor_stand" => crate::ext_entity::armor_stand::after_load(&mut e),
        "minecraft:interaction" => crate::ext_entity::interaction::after_load(&mut e),
        n if crate::ext_entity::display::is_display(n) => crate::ext_entity::display::prepare(&mut e),
        "minecraft:marker" => e.no_physics = true,
        _ => {}
    }
    // (`Passengers` is `EntityType.loadEntityRecursive`'s: see [`load_stack`].)
    r.get("Passengers");
    e.extra = r.rest();
    Ok(e)
}

/// A rider of a loaded stack: the entity and what it rides.
#[derive(Debug, Clone)]
pub struct Rider {
    pub entity: Entity,
    /// 0: the root; n: the rider at index n - 1.
    pub vehicle: usize,
}

/// `EntityType.loadEntityRecursive`: the root a compound describes and the riders of its
/// `Passengers` (and theirs), depth first, each seated on its vehicle (`startRiding(vehicle,
/// true)`: the caller links them by network ids). `seed` gives an entity's random seed from its
/// UUID. A rider vanilla would discard is left out; a rider Kiln cannot simulate or read keeps
/// the whole stack as saved (the error of the rider) when `strict`, else is left out too.
pub fn load_stack(tag: &Tag, id: i32, seed: &dyn Fn(u128) -> i64, strict: bool) -> Result<(Entity, Vec<Rider>), LoadError> {
    let uuid = tag.get("UUID").and_then(uuid_from_tag).unwrap_or(0);
    let root = load(tag, id, seed(uuid))?;
    let mut riders = Vec::new();
    load_riders(tag, 0, seed, strict, &mut riders)?;
    Ok((root, riders))
}

/// [`save_with`] of a stack as [`load_stack`] gave it.
pub fn save_stack(root: &Entity, riders: &[Rider], owner_uuid: &dyn Fn(i32) -> Option<u128>) -> Tag {
    let mut all: Vec<Entity> = std::iter::once(root.clone()).chain(riders.iter().map(|r| r.entity.clone())).collect();
    for (i, e) in all.iter_mut().enumerate() {
        e.id = i as i32 + 1;
        e.vehicle = None;
        e.passengers.clear();
    }
    for (i, r) in riders.iter().enumerate() {
        all[i + 1].vehicle = Some(r.vehicle as i32 + 1);
        all[r.vehicle].passengers.push(i as i32 + 2);
    }
    let lookup = |id: i32| usize::try_from(id - 1).ok().and_then(|i| all.get(i));
    save_with(&all[0], owner_uuid, &lookup)
}

fn load_riders(tag: &Tag, vehicle: usize, seed: &dyn Fn(u128) -> i64, strict: bool, out: &mut Vec<Rider>) -> Result<(), LoadError> {
    let Some(list) = tag.get("Passengers").and_then(Tag::as_list) else { return Ok(()) };
    for p in list {
        let uuid = p.get("UUID").and_then(uuid_from_tag).unwrap_or(0);
        match load(p, 0, seed(uuid)) {
            Ok(entity) => {
                out.push(Rider { entity, vehicle });
                let me = out.len();
                load_riders(p, me, seed, strict, out)?;
            }
            Err(LoadError::Discarded) => {}
            Err(e) if strict => return Err(e),
            Err(_) => {}
        }
    }
    Ok(())
}

fn read_kind(type_name: &'static str, r: &mut Input) -> Result<EntityKind, LoadError> {
    Ok(match type_name {
        "minecraft:item" => {
            let health = r.short_or("Health", 5);
            let age = r.short_or("Age", 0);
            let pickup_delay = r.short_or("PickupDelay", 0);
            let target = r.uuid("Owner");
            let thrower = r.uuid("Thrower");
            let stack = match r.get("Item") {
                None => return Err(LoadError::Discarded),
                Some(t) => ItemStack::from_nbt(t).map_err(|_| LoadError::Invalid("unreadable item"))?,
            };
            if stack.is_empty() {
                return Err(LoadError::Discarded);
            }
            EntityKind::Item(ItemData { stack, age, pickup_delay, health, thrower, target, bob_offset: 0.0 })
        }
        "minecraft:experience_orb" => {
            let health = r.short_or("Health", 5);
            let age = r.short_or("Age", 0);
            let value = r.short_or("Value", 0);
            // `ExtraCodecs.POSITIVE_INT`: anything else reads as absent.
            let count = r.get("Count").and_then(Tag::as_i64).filter(|&c| c > 0 && c <= i32::MAX as i64).unwrap_or(1) as i32;
            EntityKind::ExperienceOrb(OrbData { value, count, age, health, following: None })
        }
        "minecraft:falling_block" => {
            let state = r.get("BlockState").and_then(state_from_tag).unwrap_or(kiln_data::blocks::default_state::SAND);
            let mut d = FallingBlockData::new(state);
            d.time = r.int_or("Time", 0);
            d.hurt_entities = r.bool_or("HurtEntities", has_tag(state, BlockTag::Anvil));
            d.fall_damage_per_distance = r.float_or("FallHurtAmount", 0.0);
            d.fall_damage_max = r.int_or("FallHurtMax", 40);
            d.drop_item = r.bool_or("DropItem", true);
            d.cancel_drop = r.bool_or("CancelDrop", false);
            EntityKind::FallingBlock(d)
        }
        "minecraft:tnt" => {
            let mut d = TntData::new();
            d.fuse = r.short_or("fuse", 80);
            d.block_state = r.get("block_state").and_then(state_from_tag).unwrap_or(kiln_data::blocks::default_state::TNT);
            d.explosion_power = r.float_or("explosion_power", 4.0).clamp(0.0, 128.0);
            // `owner` stays in `extra` until the simulation resolves it.
            EntityKind::Tnt(d)
        }
        "minecraft:arrow" | "minecraft:spectral_arrow" => {
            let (left_owner, has_been_shot) = read_projectile(r);
            let life = r.short_or("life", 0);
            let last_state = r.get("inBlockState").and_then(state_from_tag);
            let shake_time = (r.byte_or("shake", 0) as i32) & 255;
            let in_ground = r.bool_or("inGround", false);
            let base_damage = r.num("damage").unwrap_or(2.0);
            let crit = r.bool_or("crit", false);
            let pickup = r.byte_or("pickup", 0).clamp(0, 2) as u8;
            let pierce_level = r.byte_or("PierceLevel", 0) as u8;
            let item = |t: Option<Tag>| t.and_then(|t| kiln_item::ItemStack::from_nbt(&t).ok()).filter(|s| !s.is_empty());
            let pickup_item = item(r.get("item").cloned());
            let weapon = item(r.get("weapon").cloned());
            let glowing = r.get("Duration").and_then(|t| match t {
                Tag::Int(v) => Some(*v),
                _ => None,
            });
            EntityKind::Arrow(ArrowData {
                pickup,
                pickup_item,
                weapon,
                pierce_level,
                pierced: Vec::new(),
                killed: Vec::new(),
                knockback: 0.0,
                glowing: glowing.unwrap_or(200),
                owner: None,
                left_owner,
                left_owner_checked: false,
                has_been_shot,
                in_ground,
                in_ground_time: 0,
                shake_time,
                life,
                last_state,
                crit,
                base_damage,
                effects: Vec::new(),
            })
        }
        name if crate::mob::MobKind::by_name(name).is_some() => EntityKind::MobTicking { gravity: 0.08 },
        name if crate::ext_entity::TYPES.contains(&name)
            || crate::ext_entity::boat::is_boat(name)
            || crate::ext_entity::minecart::is_minecart(name) =>
        {
            EntityKind::Ext(crate::ext_entity::load(name, r).ok_or(LoadError::NotSimulated)?)
        }
        name => {
            let kind = THROWABLES.into_iter().find(|t| t.type_name() == name).ok_or(LoadError::NotSimulated)?;
            let (left_owner, has_been_shot) = read_projectile(r);
            EntityKind::Throwable(ThrowableData { kind, owner: None, left_owner, left_owner_checked: false, has_been_shot, item: None })
        }
    })
}

/// `Projectile.readAdditionalSaveData` (`Owner` and `can_break` stay in `extra`).
fn read_projectile(r: &mut Input) -> (bool, bool) {
    (r.bool_or("LeftOwner", false), r.bool_or("HasBeenShot", false))
}

/// A compound being written.
pub struct Output(pub Vec<(String, Tag)>);

impl Output {
    pub fn put(&mut self, key: &str, value: Tag) {
        self.0.push((key.to_owned(), value));
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }
}

fn doubles(v: [f64; 3]) -> Tag {
    Tag::List(v.iter().map(|&d| Tag::Double(d)).collect())
}

fn item_tag(name: &str) -> Tag {
    Tag::Compound(vec![("id".into(), Tag::String(name.to_owned())), ("count".into(), Tag::Int(1))])
}

/// `Entity.save`: the entity's compound with its `id`, as stored in an entity chunk.
/// `owner_uuid` resolves the network id of a projectile's or TNT's owner.
pub fn save(e: &Entity, owner_uuid: &dyn Fn(i32) -> Option<u128>) -> Tag {
    save_with(e, owner_uuid, &|_| None)
}

/// `Entity.saveAsPassenger`: [`save`] of an entity with the passengers it carries (`lookup`
/// finds an entity by its network id: the level's) saved inside it as the `Passengers` list,
/// each with its own. A rider's `Pos` is its vehicle's x and z with its own y
/// (`saveWithoutId`). Passengers `lookup` does not find (players, entities gone) are left
/// out; vanilla saves a stack by its root, whose chunk it is in.
pub fn save_with<'a>(e: &'a Entity, owner_uuid: &dyn Fn(i32) -> Option<u128>, lookup: &dyn Fn(i32) -> Option<&'a Entity>) -> Tag {
    let mut o = Output(Vec::new());
    let extra = |key: &str| e.extra.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    o.put("id", Tag::String(e.type_name.to_owned()));
    let p = match e.vehicle.and_then(lookup) {
        Some(v) => Vec3::new(v.x(), e.y(), v.z()),
        None => e.position(),
    };
    o.put("Pos", doubles([p.x, p.y, p.z]));
    o.put("Motion", doubles([e.delta.x, e.delta.y, e.delta.z]));
    o.put("Rotation", Tag::List(vec![Tag::Float(e.y_rot), Tag::Float(e.x_rot)]));
    o.put("fall_distance", Tag::Double(e.fall_distance));
    o.put("Fire", Tag::Short(e.remaining_fire_ticks as i16));
    o.put("Air", Tag::Short(e.air_supply as i16));
    o.put("OnGround", Tag::Byte(e.on_ground as i8));
    o.put("Invulnerable", Tag::Byte(e.invulnerable as i8));
    o.put("PortalCooldown", extra("PortalCooldown").unwrap_or(Tag::Int(0)));
    if e.invulnerable_time > 0 {
        o.put("invulnerable_time", Tag::Int(e.invulnerable_time));
    }
    o.put("UUID", uuid_to_tag(e.uuid));
    if e.silent {
        o.put("Silent", Tag::Byte(1));
    }
    if e.no_gravity {
        o.put("NoGravity", Tag::Byte(1));
    }
    if e.ticks_frozen > 0 {
        o.put("TicksFrozen", Tag::Int(e.ticks_frozen));
    }
    let owner = |id: Option<i32>| id.and_then(owner_uuid).map(uuid_to_tag);
    match &e.kind {
        EntityKind::Item(d) => {
            o.put("Health", Tag::Short(d.health as i16));
            o.put("Age", Tag::Short(d.age as i16));
            o.put("PickupDelay", Tag::Short(d.pickup_delay as i16));
            if let Some(t) = d.thrower {
                o.put("Thrower", uuid_to_tag(t));
            }
            if let Some(t) = d.target {
                o.put("Owner", uuid_to_tag(t));
            }
            if !d.stack.is_empty() {
                o.put("Item", d.stack.to_nbt());
            }
        }
        EntityKind::ExperienceOrb(d) => {
            o.put("Health", Tag::Short(d.health as i16));
            o.put("Age", Tag::Short(d.age as i16));
            o.put("Value", Tag::Short(d.value as i16));
            o.put("Count", Tag::Int(d.count));
        }
        EntityKind::FallingBlock(d) => {
            o.put("BlockState", state_to_tag(d.state));
            o.put("Time", Tag::Int(d.time));
            o.put("DropItem", Tag::Byte(d.drop_item as i8));
            o.put("HurtEntities", Tag::Byte(d.hurt_entities as i8));
            o.put("FallHurtAmount", Tag::Float(d.fall_damage_per_distance));
            o.put("FallHurtMax", Tag::Int(d.fall_damage_max));
            if let Some(t) = extra("TileEntityData") {
                o.put("TileEntityData", t);
            }
            o.put("CancelDrop", Tag::Byte(d.cancel_drop as i8));
        }
        EntityKind::Tnt(d) => {
            o.put("fuse", Tag::Short(d.fuse as i16));
            o.put("block_state", state_to_tag(d.block_state));
            if d.explosion_power != 4.0 {
                o.put("explosion_power", Tag::Float(d.explosion_power));
            }
            if let Some(t) = owner(d.owner) {
                o.put("owner", t);
            }
        }
        EntityKind::Throwable(d) => {
            write_projectile(&mut o, owner(d.owner), d.left_owner, d.has_been_shot);
            o.put("Item", extra("Item").unwrap_or_else(|| item_tag(d.kind.type_name())));
        }
        EntityKind::Arrow(d) => {
            write_projectile(&mut o, owner(d.owner), d.left_owner, d.has_been_shot);
            o.put("life", Tag::Short(d.life as i16));
            if let Some(s) = d.last_state {
                o.put("inBlockState", state_to_tag(s));
            }
            o.put("shake", Tag::Byte(d.shake_time as i8));
            o.put("inGround", Tag::Byte(d.in_ground as i8));
            o.put("pickup", Tag::Byte(d.pickup as i8));
            o.put("damage", Tag::Double(d.base_damage));
            o.put("crit", Tag::Byte(d.crit as i8));
            o.put("PierceLevel", Tag::Byte(d.pierce_level as i8));
            o.put("SoundEvent", extra("SoundEvent").unwrap_or_else(|| Tag::String("minecraft:entity.arrow.hit".into())));
            let item = d.pickup_item.clone().or_else(|| ItemStack::of(e.type_name, 1));
            o.put("item", item.as_ref().map(ItemStack::to_nbt).unwrap_or_else(|| item_tag(e.type_name)));
            if let Some(w) = &d.weapon {
                o.put("weapon", w.to_nbt());
            }
            if e.type_name == "minecraft:spectral_arrow" {
                o.put("Duration", Tag::Int(d.glowing));
            }
        }
        EntityKind::Mob(m) => crate::mob::persist::save(e, m, &mut o),
        EntityKind::Ext(x) => x.save(e, &mut o),
        EntityKind::Player(_) | EntityKind::MobTicking { .. } | EntityKind::Other { .. } => {}
    }
    if let Some(t) = crate::leash::save(e) {
        o.put("leash", t);
    }
    // `Entity.saveWithoutId`: the passengers, each saved with its own (a rider that cannot be
    // saved is left out, and the list with it when nothing is left).
    let riders: Vec<Tag> = e
        .passengers
        .iter()
        .filter_map(|&id| lookup(id))
        .filter(|p| p.removed.is_none() && p.vehicle == Some(e.id) && !matches!(p.kind, EntityKind::Player(_)))
        .map(|p| save_with(p, owner_uuid, lookup))
        .collect();
    if !riders.is_empty() {
        o.put("Passengers", Tag::List(riders));
    }
    // Everything else as it was loaded (custom name, tags, an unresolved owner).
    for (k, v) in &e.extra {
        if !o.has(k) {
            o.put(k, v.clone());
        }
    }
    Tag::Compound(o.0)
}

/// `Projectile.addAdditionalSaveData` (a loaded `Owner` or `can_break` comes from `extra`).
fn write_projectile(o: &mut Output, owner: Option<Tag>, left_owner: bool, has_been_shot: bool) {
    if let Some(t) = owner {
        o.put("Owner", t);
    }
    if left_owner {
        o.put("LeftOwner", Tag::Byte(1));
    }
    o.put("HasBeenShot", Tag::Byte(has_been_shot as i8));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::BlockPos;

    fn no_owner(_: i32) -> Option<u128> {
        None
    }

    /// Saves, loads and saves again: the two compounds must be equal, and the loaded entity
    /// must carry the same state.
    fn round_trip(e: &Entity) -> Entity {
        let first = save(e, &no_owner);
        let loaded = load(&first, 99, 1).unwrap_or_else(|err| panic!("{} did not load: {err:?}", e.type_name));
        let second = save(&loaded, &no_owner);
        assert_eq!(first, second, "{} changed across a round trip", e.type_name);
        assert_eq!(loaded.uuid, e.uuid);
        assert_eq!(loaded.position(), e.position());
        assert_eq!(loaded.delta, e.delta);
        assert_eq!((loaded.y_rot, loaded.x_rot), (e.y_rot, e.x_rot));
        loaded
    }

    const UUID: u128 = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;

    #[test]
    fn uuid_codec() {
        let t = uuid_to_tag(UUID);
        assert_eq!(t, Tag::IntArray(vec![0x01234567, 0x89abcdefu32 as i32, 0xfedcba98u32 as i32, 0x76543210]));
        assert_eq!(uuid_from_tag(&t), Some(UUID));
    }

    #[test]
    fn item_round_trip() {
        let stack = ItemStack::of("minecraft:diamond", 7).unwrap();
        let mut e = crate::item::new_at(5, UUID, stack.clone(), Vec3::new(10.5, 64.0, -3.25), 42);
        e.on_ground = true;
        e.remaining_fire_ticks = -1;
        if let EntityKind::Item(d) = &mut e.kind {
            d.age = 1234;
            d.pickup_delay = 17;
            d.health = 3;
            d.thrower = Some(7);
            d.target = Some(UUID ^ 1);
        }
        let back = round_trip(&e);
        let EntityKind::Item(d) = &back.kind else { panic!("not an item") };
        assert_eq!((d.age, d.pickup_delay, d.health, d.thrower, d.target), (1234, 17, 3, Some(7), Some(UUID ^ 1)));
        assert_eq!(d.stack, stack);
        assert!(back.on_ground);
        assert_eq!(back.remaining_fire_ticks, -1);
    }

    #[test]
    fn item_without_item_is_discarded() {
        let mut tag = save(&crate::item::new(1, UUID, ItemStack::of("minecraft:stone", 1).unwrap(), 0), &no_owner);
        if let Tag::Compound(f) = &mut tag {
            f.retain(|(k, _)| k != "Item");
        }
        assert_eq!(load(&tag, 1, 0).err(), Some(LoadError::Discarded));
    }

    #[test]
    fn orb_falling_block_tnt_round_trip() {
        let mut orb = crate::xp_orb::new_at(1, UUID, Vec3::new(0.5, 70.0, 0.5), 11, 3);
        if let EntityKind::ExperienceOrb(d) = &mut orb.kind {
            d.count = 4;
            d.age = 99;
        }
        let back = round_trip(&orb);
        let EntityKind::ExperienceOrb(d) = &back.kind else { panic!() };
        assert_eq!((d.value, d.count, d.age, d.health), (11, 4, 99, 5));

        let anvil = kiln_data::blocks_types::block_by_name("minecraft:anvil").unwrap();
        let state = anvil.with_property(anvil.default, "facing", "east").unwrap();
        let mut fb = crate::falling_block::fall(2, UUID, BlockPos::new(3, 80, -9), state, 4);
        if let EntityKind::FallingBlock(d) = &mut fb.kind {
            d.time = 12;
        }
        let back = round_trip(&fb);
        let EntityKind::FallingBlock(d) = &back.kind else { panic!() };
        assert_eq!((d.state, d.time, d.hurt_entities, d.fall_damage_per_distance), (state, 12, true, 2.0));

        let mut tnt = crate::tnt::ignite(3, UUID, Vec3::new(1.5, 65.0, 1.5), None, 5);
        if let EntityKind::Tnt(d) = &mut tnt.kind {
            d.fuse = 33;
        }
        let back = round_trip(&tnt);
        let EntityKind::Tnt(d) = &back.kind else { panic!() };
        assert_eq!((d.fuse, d.explosion_power, d.block_state), (33, 4.0, kiln_data::blocks::default_state::TNT));
    }

    #[test]
    fn projectiles_round_trip() {
        for t in THROWABLES {
            let e = crate::projectile::new(4, UUID, t, Vec3::new(0.0, 100.0, 0.0), Vec3::new(0.1, 0.2, 0.3), None, 6);
            let back = round_trip(&e);
            assert!(matches!(&back.kind, EntityKind::Throwable(d) if d.kind == t));
        }
        let mut arrow = crate::arrow::new(5, UUID, "minecraft:arrow", Vec3::new(2.0, 90.0, 2.0), Vec3::new(0.0, -1.0, 0.5), None, 7);
        if let EntityKind::Arrow(d) = &mut arrow.kind {
            d.in_ground = true;
            d.life = 300;
            d.shake_time = 4;
            d.crit = true;
            d.last_state = Some(kiln_data::blocks::default_state::STONE);
        }
        let back = round_trip(&arrow);
        let EntityKind::Arrow(d) = &back.kind else { panic!() };
        assert_eq!((d.in_ground, d.life, d.shake_time, d.crit), (true, 300, 4, true));
        assert_eq!(d.last_state, Some(kiln_data::blocks::default_state::STONE));
        let spectral = crate::arrow::new(6, UUID, "minecraft:spectral_arrow", Vec3::ZERO, Vec3::ZERO, None, 8);
        let tag = save(&spectral, &no_owner);
        assert_eq!(tag.get("Duration"), Some(&Tag::Int(200)));
        round_trip(&spectral);
    }

    #[test]
    fn owners_resolve_and_unknown_fields_survive() {
        let e = crate::projectile::new(4, UUID, Throwable::Snowball, Vec3::ZERO, Vec3::ZERO, Some(12), 6);
        let tag = save(&e, &|id| (id == 12).then_some(77));
        assert_eq!(tag.get("Owner"), Some(&uuid_to_tag(77)));
        // A vanilla compound with fields Kiln does not model.
        let mut tag = tag;
        if let Tag::Compound(f) = &mut tag {
            f.retain(|(k, _)| k != "PortalCooldown");
            f.push(("CustomName".into(), Tag::String("Bob".into())));
            f.push(("Tags".into(), Tag::List(vec![Tag::String("a".into())])));
            f.push(("PortalCooldown".into(), Tag::Int(5)));
        }
        let loaded = load(&tag, 1, 0).unwrap();
        let again = save(&loaded, &no_owner);
        for key in ["CustomName", "Tags", "Owner"] {
            assert_eq!(again.get(key), tag.get(key), "{key}");
        }
        assert_eq!(again.get("PortalCooldown"), Some(&Tag::Int(5)));
    }

    #[test]
    fn vanilla_defaults_and_limits() {
        // A minimal vanilla-style compound: defaults fill in, motion over 10 is dropped.
        let tag = Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:experience_orb".into())),
            ("Pos".into(), doubles([1.0, 2.0, 3.0])),
            ("Motion".into(), doubles([11.0, -0.5, 0.0])),
            ("Value".into(), Tag::Short(3)),
            ("Count".into(), Tag::Int(0)),
        ]);
        let e = load(&tag, 1, 0).unwrap();
        assert_eq!(e.delta, Vec3::new(0.0, -0.5, 0.0));
        assert_eq!(e.air_supply, 300);
        let EntityKind::ExperienceOrb(d) = &e.kind else { panic!() };
        assert_eq!((d.value, d.count, d.health), (3, 1, 5));
        assert_eq!(e.uuid, 0);
        let pig = Tag::Compound(vec![("id".into(), Tag::String("minecraft:pig".into()))]);
        assert!(matches!(load(&pig, 1, 0).map(|e| e.kind), Ok(EntityKind::Mob(_))));
        let display = Tag::Compound(vec![("id".into(), Tag::String("minecraft:cushion".into()))]);
        assert_eq!(load(&display, 1, 0).err(), Some(LoadError::NotSimulated));
        let nan = Tag::Compound(vec![
            ("id".into(), Tag::String("minecraft:snowball".into())),
            ("Pos".into(), doubles([f64::NAN, 0.0, 0.0])),
        ]);
        assert!(matches!(load(&nan, 1, 0), Err(LoadError::Invalid(_))));
    }
}
