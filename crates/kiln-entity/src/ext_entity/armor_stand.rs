//! Armor stands (`ArmorStand`): a living entity that wears and holds what players put on it.
//! A click dresses it (an item goes where it belongs, or into a hand when the stand has arms) or
//! undresses it (by the height clicked); two hits within five ticks break it (a creative player
//! at once); explosions and the like break it; fire burns it down. It does not think, but it falls.

use crate::entity::{Entity, EntityKind, RemovalReason};
use crate::ext_entity::EntityExt;
use crate::level::{DamageKind, EntityLevel, Event};
use crate::math::{BlockPos, Vec3};
use crate::mob::interact::{HeldChange, Interactor, Outcome};
use crate::persist::{Input, Output};
use kiln_item::component::EquipmentSlot;
use kiln_item::{ItemStack, keys};
use kiln_javamath::random::RandomSource;
use kiln_proto::nbt::Tag;
use kiln_proto::packets::entity::{DataValue, EntityData};

pub const ARMOR_STAND: &str = "minecraft:armor_stand";

/// The slots a stand has, in `EquipmentSlot.VALUES` order (also the equipment packet's ids).
pub const SLOTS: [EquipmentSlot; 6] =
    [EquipmentSlot::MainHand, EquipmentSlot::OffHand, EquipmentSlot::Feet, EquipmentSlot::Legs, EquipmentSlot::Chest, EquipmentSlot::Head];

/// The names of the slots in the saved `equipment` compound.
const SLOT_KEYS: [&str; 8] = ["mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle"];

/// `EquipmentSlot.getFilterBit(0)`.
fn filter_bit(slot: EquipmentSlot) -> i32 {
    match slot {
        EquipmentSlot::MainHand => 0,
        EquipmentSlot::OffHand => 5,
        EquipmentSlot::Feet => 1,
        EquipmentSlot::Legs => 2,
        EquipmentSlot::Chest => 3,
        EquipmentSlot::Head => 4,
        EquipmentSlot::Body => 6,
        EquipmentSlot::Saddle => 7,
    }
}

fn is_hand(slot: EquipmentSlot) -> bool {
    matches!(slot, EquipmentSlot::MainHand | EquipmentSlot::OffHand)
}

fn index_of(slot: EquipmentSlot) -> Option<usize> {
    SLOTS.iter().position(|&s| s == slot)
}

/// `Rotations` of the six body parts, in the order head, body, left arm, right arm, left leg,
/// right leg, and their defaults.
pub const DEFAULT_POSES: [[f32; 3]; 6] = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0], [-10.0, 0.0, -10.0], [-15.0, 0.0, 10.0], [-1.0, 0.0, -1.0], [1.0, 0.0, 1.0]];
const POSE_KEYS: [&str; 6] = ["Head", "Body", "LeftArm", "RightArm", "LeftLeg", "RightLeg"];

#[derive(Clone, Debug)]
pub struct ArmorStand {
    pub equipment: [ItemStack; 6],
    pub small: bool,
    pub show_arms: bool,
    pub no_base_plate: bool,
    pub marker: bool,
    pub invisible: bool,
    pub disabled_slots: i32,
    pub poses: [[f32; 3]; 6],
    pub health: f32,
    /// The game time of the last hit that did not break it.
    pub last_hit: i64,
}

impl ArmorStand {
    fn new() -> Self {
        ArmorStand {
            equipment: Default::default(),
            small: false,
            show_arms: false,
            no_base_plate: false,
            marker: false,
            invisible: false,
            disabled_slots: 0,
            poses: DEFAULT_POSES,
            health: 20.0,
            last_hit: 0,
        }
    }

    /// `ArmorStand.getDimensionsMarker`: `(width, height)`.
    fn dimensions(&self) -> (f32, f32) {
        if self.marker {
            (0.0, 0.0)
        } else if self.small {
            (0.25, 0.9875)
        } else {
            (0.5, 1.975)
        }
    }

    fn item(&self, slot: EquipmentSlot) -> &ItemStack {
        static EMPTY: std::sync::OnceLock<ItemStack> = std::sync::OnceLock::new();
        match index_of(slot) {
            Some(i) => &self.equipment[i],
            None => EMPTY.get_or_init(ItemStack::empty),
        }
    }

    /// `ArmorStand.isDisabled`.
    fn is_disabled(&self, slot: EquipmentSlot) -> bool {
        self.disabled_slots & (1 << filter_bit(slot)) != 0 || (is_hand(slot) && !self.show_arms)
    }

    /// `ArmorStand.canUseSlot`.
    fn can_use_slot(&self, slot: EquipmentSlot) -> bool {
        !matches!(slot, EquipmentSlot::Body | EquipmentSlot::Saddle) && !self.is_disabled(slot)
    }

    /// `LivingEntity.getEquipmentSlotForItem`.
    fn slot_for_item(&self, stack: &ItemStack) -> EquipmentSlot {
        match stack.get(keys::EQUIPPABLE) {
            Some(eq) if self.can_use_slot(eq.slot) => eq.slot,
            _ => EquipmentSlot::MainHand,
        }
    }

    /// `ArmorStand.getClickedSlot`: the part of the stand the click (a height above its feet) is on.
    fn clicked_slot(&self, y: f64) -> EquipmentSlot {
        let small = self.small;
        // (`getScale() * getAgeScale()`: a small stand counts as a baby.)
        let y = y / if small { 0.5 } else { 1.0 };
        if y >= 0.1 && y < 0.1 + if small { 0.8 } else { 0.45 } && !self.item(EquipmentSlot::Feet).is_empty() {
            EquipmentSlot::Feet
        } else if y >= 0.9 + if small { 0.3 } else { 0.0 } && y < 0.9 + if small { 1.0 } else { 0.7 } && !self.item(EquipmentSlot::Chest).is_empty() {
            EquipmentSlot::Chest
        } else if y >= 0.4 && y < 0.4 + if small { 1.0 } else { 0.8 } && !self.item(EquipmentSlot::Legs).is_empty() {
            EquipmentSlot::Legs
        } else if y >= 1.6 && !self.item(EquipmentSlot::Head).is_empty() {
            EquipmentSlot::Head
        } else if self.item(EquipmentSlot::MainHand).is_empty() && !self.item(EquipmentSlot::OffHand).is_empty() {
            EquipmentSlot::OffHand
        } else {
            EquipmentSlot::MainHand
        }
    }

    /// `LivingEntity.setItemSlot` (+ `onEquipItem`): the stack, the sound and the game event of
    /// the new piece.
    fn set_item_slot(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, slot: EquipmentSlot, stack: ItemStack) {
        let Some(i) = index_of(slot) else { return };
        let old = std::mem::replace(&mut self.equipment[i], stack.clone());
        if old.is_same_item_same_components(&stack) || e.first_tick {
            return;
        }
        let equippable = stack.get(keys::EQUIPPABLE).cloned();
        if !e.silent
            && let Some(eq) = &equippable
            && eq.slot == slot
        {
            let sound = match &eq.equip_sound {
                kiln_item::Holder::Reference(id) => kiln_item::registry::SOUND_EVENT.name(*id),
                kiln_item::Holder::Direct(_) => None,
            }
            .unwrap_or("minecraft:item.armor.equip_generic");
            level.emit(Event::Sound { pos: e.position(), sound, source: "neutral", volume: 1.0, pitch: 1.0 });
        }
        let event = if equippable.is_some() { "minecraft:equip" } else { "minecraft:unequip" };
        level.emit(Event::GameEvent { event, pos: e.position(), entity: Some(e.id) });
    }

    /// `ArmorStand.swapItem`: `Some(change to the held stack)` when the exchange happened.
    fn swap_item(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, slot: EquipmentSlot, held: &ItemStack) -> Option<HeldChange> {
        let current = self.item(slot).clone();
        if !current.is_empty() && self.disabled_slots & (1 << (filter_bit(slot) + 8)) != 0 {
            return None;
        }
        if current.is_empty() && self.disabled_slots & (1 << (filter_bit(slot) + 16)) != 0 {
            return None;
        }
        if who.creative && current.is_empty() && !held.is_empty() {
            let mut one = held.clone();
            one.set_count(1);
            self.set_item_slot(e, level, slot, one);
            return Some(HeldChange::None);
        }
        if !held.is_empty() && held.count() > 1 {
            if !current.is_empty() {
                return None;
            }
            let mut one = held.clone();
            one.set_count(1);
            self.set_item_slot(e, level, slot, one);
            return Some(HeldChange::Shrink(1));
        }
        self.set_item_slot(e, level, slot, held.clone());
        Some(HeldChange::Replace(current))
    }

    /// Re-places the stand's box after its size changed (`refreshDimensions`).
    fn refresh_dimensions(&self, e: &mut Entity) {
        let (w, h) = self.dimensions();
        if e.width != w || e.height != h {
            e.width = w;
            e.height = h;
            let p = e.position();
            e.set_pos(p);
        }
    }

    /// `Block.popResource` at a block: an item a little off its middle.
    fn pop_resource(&self, level: &mut dyn EntityLevel, p: BlockPos, stack: ItemStack) {
        if stack.is_empty() {
            return;
        }
        let r = level.random();
        let x = p.x as f64 + 0.5 + (r.next_double() * 0.5 - 0.25);
        let y = p.y as f64 + 0.5 + (r.next_double() * 0.5 - 0.25) - 0.125;
        let z = p.z as f64 + 0.5 + (r.next_double() * 0.5 - 0.25);
        let id = level.next_entity_id();
        let seed = level.fresh_seed();
        let mut item = crate::item::new(id, 0, stack, seed);
        item.set_pos(Vec3::new(x, y, z));
        let dx = item.random.next_double() * 0.2 - 0.1;
        let dz = item.random.next_double() * 0.2 - 0.1;
        item.delta = Vec3::new(dx, 0.2, dz);
        if let EntityKind::Item(d) = &mut item.kind {
            d.pickup_delay = 10;
        }
        item.set_old_pos_and_rot();
        level.add_entity(item);
    }

    fn block_pos(e: &Entity) -> BlockPos {
        let p = e.position();
        BlockPos::containing(p.x, p.y, p.z)
    }

    fn play_broken_sound(&self, e: &mut Entity, level: &mut dyn EntityLevel) {
        e.play_sound(level, "minecraft:entity.armor_stand.break", 1.0, 1.0);
    }

    fn show_breaking_particles(&self, e: &Entity, level: &mut dyn EntityLevel) {
        // `sendParticles(BlockParticleOption(BLOCK, oak_planks), x, getY(2/3), z, 10, w/4, h/4, w/4, 0.05)`.
        let p = e.position();
        let (w, h) = (e.width, e.height);
        let at = Vec3::new(p.x, p.y + h as f64 * 0.6666666666666666, p.z);
        level.block_particles("minecraft:block", at, kiln_data::blocks::default_state::OAK_PLANKS, 10, Vec3::new((w / 4.0) as f64, (h / 4.0) as f64, (w / 4.0) as f64), 0.05);
    }

    /// `brokenByAnything`: the break sound, the death loot, everything worn falls out.
    fn broken_by_anything(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        self.play_broken_sound(e, level);
        // (`dropAllDeathLoot`: the armor stand's loot table is empty.)
        let above = Self::block_pos(e).offset(0, 1, 0);
        for i in 0..SLOTS.len() {
            let stack = std::mem::take(&mut self.equipment[i]);
            if !stack.is_empty() && level.entity_drops() {
                self.pop_resource(level, above, stack);
            }
        }
    }

    /// `brokenByPlayer`: the stand itself drops, with its name, then everything else.
    fn broken_by_player(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        let mut stack = ItemStack::of("minecraft:armor_stand", 1).unwrap_or_default();
        if let Some(name) = e.extra.iter().find(|(k, _)| k == "CustomName").and_then(|(_, t)| kiln_item::Text::from_nbt(t.clone())) {
            stack.set(kiln_item::component::Component::CustomName(name));
        }
        if level.entity_drops() {
            self.pop_resource(level, Self::block_pos(e), stack);
        }
        self.broken_by_anything(e, level);
    }

    /// `LivingEntity.kill(level, source)`.
    fn kill(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, by: Option<i32>) {
        if !e.is_removed() {
            e.removed = Some(RemovalReason::Killed);
            level.emit(Event::GameEvent { event: "minecraft:entity_die", pos: e.position(), entity: by });
        }
    }

    /// `causeDamage`: the stand loses health and breaks when little remains.
    fn cause_damage(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, amount: f32, by: Option<i32>) {
        let health = self.health - amount;
        if health <= 0.5 {
            self.broken_by_anything(e, level);
            self.kill(e, level, by);
        } else {
            self.health = health;
            level.emit(Event::GameEvent { event: "minecraft:entity_damage", pos: e.position(), entity: by });
        }
    }
}

pub fn new(id: i32, pos: Vec3, yaw: f32, seed: i64) -> Entity {
    let stand = ArmorStand::new();
    let mut e = Entity::new(ARMOR_STAND, id, 0, EntityKind::Other { type_name: ARMOR_STAND }, seed);
    let (w, h) = stand.dimensions();
    e.width = w;
    e.height = h;
    e.kind = EntityKind::Ext(Box::new(stand));
    e.set_pos(pos);
    e.y_rot = yaw;
    e.y_rot_o = yaw;
    e.set_old_pos_and_rot();
    e
}

/// The things the stand's saved data names that its `entity_data` item component adds.
pub fn apply_saved(e: &mut Entity, data: &Tag) {
    let Tag::Compound(fields) = data else { return };
    let mut r = Input { fields, used: Vec::new() };
    let Some(x) = crate::ext_entity::get_mut::<ArmorStand>(e) else { return };
    read_stand(x, &mut r);
    let x = x.clone();
    x.refresh_dimensions(e);
}

fn read_stand(x: &mut ArmorStand, r: &mut Input) {
    x.invisible = r.bool_or("Invisible", false);
    x.small = r.bool_or("Small", false);
    x.show_arms = r.bool_or("ShowArms", false);
    x.disabled_slots = r.int_or("DisabledSlots", 0);
    x.no_base_plate = r.bool_or("NoBasePlate", false);
    x.marker = r.bool_or("Marker", false);
    if let Some(Tag::Compound(pose)) = r.get("Pose") {
        for (i, key) in POSE_KEYS.iter().enumerate() {
            if let Some(Tag::List(l)) = pose.iter().find(|(k, _)| k == key).map(|(_, v)| v)
                && l.len() == 3
            {
                for (j, t) in l.iter().enumerate() {
                    x.poses[i][j] = t.as_f64().unwrap_or(0.0) as f32;
                }
            }
        }
    }
}

pub fn load(r: &mut Input) -> Option<Box<dyn EntityExt>> {
    let mut x = ArmorStand::new();
    read_stand(&mut x, r);
    x.health = r.float_or("Health", 20.0);
    if let Some(Tag::Compound(eq)) = r.get("equipment") {
        for (i, key) in SLOT_KEYS.iter().enumerate().take(6) {
            if let Some(t) = eq.iter().find(|(k, _)| k == key).map(|(_, v)| v)
                && let Ok(s) = ItemStack::from_nbt(t)
            {
                x.equipment[i] = s;
            }
        }
    }
    Some(Box::new(x))
}

/// After the entity's fields are read: the box for the stand's size, and what its type does
/// not model (`HurtTime` and the like) stays in `extra`.
pub fn after_load(e: &mut Entity) {
    let Some(x) = crate::ext_entity::get::<ArmorStand>(e).cloned() else { return };
    x.refresh_dimensions(e);
}

impl EntityExt for ArmorStand {
    crate::entity_ext_boilerplate!();

    fn gravity(&self) -> f64 {
        0.08
    }

    fn tick(&mut self, e: &mut Entity, level: &mut dyn EntityLevel) {
        // `Entity.baseTick` (fire, fluids) then `aiStep`'s travel; a marker or a stand without
        // gravity does not move (`hasPhysics`).
        e.base_tick(level);
        if e.is_removed() {
            return;
        }
        // `LivingEntity.aiStep`: tiny speeds are dropped.
        let mut d = e.delta;
        if d.x.abs() < 0.003 {
            d.x = 0.0;
        }
        if d.y.abs() < 0.003 {
            d.y = 0.0;
        }
        if d.z.abs() < 0.003 {
            d.z = 0.0;
        }
        e.delta = d;
        if self.marker || e.no_gravity {
            return;
        }
        // `travel` (no input): drag by what it stands on, then gravity.
        let friction = if e.on_ground {
            let below = e.block_pos_below_that_affects_movement(&*level);
            crate::physics::block_factors(level.block(below)).friction * 0.91
        } else {
            0.91
        };
        if e.is_in_water() {
            e.do_move(level, crate::entity::MoverType::SelfMove, e.delta);
            let d = e.delta.multiply(0.8, 0.8, 0.8);
            e.delta = Vec3::new(d.x, d.y - 0.08 / 16.0, d.z);
        } else if e.is_in_lava() {
            e.do_move(level, crate::entity::MoverType::SelfMove, e.delta);
            let d = e.delta.multiply(0.5, 0.5, 0.5);
            e.delta = Vec3::new(d.x, d.y - 0.02, d.z);
        } else {
            e.do_move(level, crate::entity::MoverType::SelfMove, e.delta);
            let d = e.delta;
            e.delta = Vec3::new(d.x * friction as f64, (d.y - 0.08) * 0.98f32 as f64, d.z * friction as f64);
        }
        e.apply_effects_from_blocks(level);
    }

    fn save(&self, _e: &Entity, o: &mut Output) {
        o.put("Invisible", Tag::Byte(self.invisible as i8));
        o.put("Small", Tag::Byte(self.small as i8));
        o.put("ShowArms", Tag::Byte(self.show_arms as i8));
        o.put("DisabledSlots", Tag::Int(self.disabled_slots));
        o.put("NoBasePlate", Tag::Byte(self.no_base_plate as i8));
        if self.marker {
            o.put("Marker", Tag::Byte(1));
        }
        // `ArmorStandPose.CODEC`: only the parts that are not as the stand has them by default.
        let mut pose = Vec::new();
        for (i, key) in POSE_KEYS.iter().enumerate() {
            if self.poses[i] != DEFAULT_POSES[i] {
                pose.push(((*key).to_owned(), Tag::List(self.poses[i].iter().map(|&f| Tag::Float(f)).collect())));
            }
        }
        o.put("Pose", Tag::Compound(pose));
        // `LivingEntity.addAdditionalSaveData`.
        o.put("Health", Tag::Float(self.health));
        o.put("HurtTime", Tag::Short(0));
        o.put("DeathTime", Tag::Short(0));
        o.put("AbsorptionAmount", Tag::Float(0.0));
        o.put("FallFlying", Tag::Byte(0));
        o.put("attributes", Tag::List(Vec::new()));
        o.put("Brain", Tag::Compound(vec![("memories".into(), Tag::Compound(Vec::new()))]));
        o.put("current_impulse_context_reset_grace_time", Tag::Int(0));
        let mut eq = Vec::new();
        for (i, key) in SLOT_KEYS.iter().enumerate().take(6) {
            if !self.equipment[i].is_empty() {
                eq.push(((*key).to_owned(), self.equipment[i].to_nbt()));
            }
        }
        if !eq.is_empty() {
            o.put("equipment", Tag::Compound(eq));
        }
    }

    fn entity_data(&self, e: &Entity, d: &mut EntityData) {
        use kiln_data::entities::data::{armor_stand as a, entity, living_entity};
        let mut flags = 0u8;
        if e.remaining_fire_ticks > 0 {
            flags |= kiln_proto::packets::entity::metadata::shared_flags::ON_FIRE;
        }
        if self.invisible {
            flags |= kiln_proto::packets::entity::metadata::shared_flags::INVISIBLE;
        }
        if flags != 0 {
            d.set(entity::SHARED_FLAGS, &DataValue::Byte(flags as i8));
        }
        d.set(living_entity::HEALTH, &DataValue::Float(self.health));
        let client = self.small as i8 | (self.show_arms as i8) << 2 | (self.no_base_plate as i8) << 3 | (self.marker as i8) << 4;
        if client != 0 {
            d.set(a::CLIENT_FLAGS, &DataValue::Byte(client));
        }
        let fields = [a::HEAD_POSE, a::BODY_POSE, a::LEFT_ARM_POSE, a::RIGHT_ARM_POSE, a::LEFT_LEG_POSE, a::RIGHT_LEG_POSE];
        for (i, f) in fields.iter().enumerate() {
            if self.poses[i] != DEFAULT_POSES[i] {
                d.set(*f, &DataValue::Rotations(self.poses[i]));
            }
        }
    }

    fn equipment_shown(&self) -> Vec<(u8, ItemStack)> {
        SLOTS.iter().enumerate().filter(|(i, _)| !self.equipment[*i].is_empty()).map(|(i, _)| (i as u8, self.equipment[i].clone())).collect()
    }

    fn attackable(&self) -> bool {
        // (`Entity.isAttackable`: players can hit it; `ArmorStand.attackable`, the mobs' targeting,
        // is false, which no mob here asks.)
        true
    }

    /// `ArmorStand.interact`.
    fn interact(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, who: &Interactor, stack: &ItemStack) -> Option<Outcome> {
        if self.marker || (!stack.is_empty() && stack.item_name() == "minecraft:name_tag") {
            // `NameTagItem.interactLivingEntity`: the stand takes the name.
            if !stack.is_empty()
                && let Some(name) = stack.get(keys::CUSTOM_NAME)
                && !e.is_removed()
            {
                e.extra.retain(|(k, _)| k != "CustomName");
                e.extra.push(("CustomName".into(), name.nbt().clone()));
                return Some(Outcome::success(HeldChange::Consume(1)));
            }
            return None;
        }
        if who.spectator {
            return Some(Outcome::success(HeldChange::None));
        }
        let slot = self.slot_for_item(stack);
        if stack.is_empty() {
            let clicked = self.clicked_slot(who.hit.y);
            let slot = if self.is_disabled(clicked) { slot } else { clicked };
            if !self.item(slot).is_empty()
                && let Some(change) = self.swap_item(e, level, who, slot, stack)
            {
                return Some(Outcome::success(change));
            }
        } else {
            if self.is_disabled(slot) {
                return Some(Outcome::PASS);
            }
            if is_hand(slot) && !self.show_arms {
                return Some(Outcome::PASS);
            }
            if let Some(change) = self.swap_item(e, level, who, slot, stack) {
                return Some(Outcome::success(change));
            }
        }
        None
    }

    /// `ArmorStand.hurtServer`.
    fn hurt(&mut self, e: &mut Entity, level: &mut dyn EntityLevel, kind: DamageKind, _amount: f32, attacker: Option<i32>) -> bool {
        if e.is_removed() {
            return false;
        }
        let attacker_mob = attacker.and_then(|a| level.entity(a)).is_some_and(|a| crate::mob::data(a).is_some());
        if !level.mob_griefing() && attacker_mob {
            return false;
        }
        if kind.is_tag("minecraft:bypasses_invulnerability") {
            self.kill(e, level, attacker);
            return false;
        }
        if e.is_invulnerable_to_base(kind) || self.invisible || self.marker {
            return false;
        }
        if kind.is_tag("minecraft:is_explosion") {
            self.broken_by_anything(e, level);
            self.kill(e, level, attacker);
            return false;
        }
        if kind.is_tag("minecraft:ignites_armor_stands") {
            if e.remaining_fire_ticks > 0 {
                self.cause_damage(e, level, 0.15, attacker);
            } else {
                e.ignite_for_ticks(100);
            }
            return false;
        }
        if kind.is_tag("minecraft:burns_armor_stands") && self.health > 0.5 {
            self.cause_damage(e, level, 4.0, attacker);
            return false;
        }
        let can_break = kind.is_tag("minecraft:can_break_armor_stand");
        let always_kills = kind.is_tag("minecraft:always_kills_armor_stands");
        if !can_break && !always_kills {
            return false;
        }
        let player = attacker.and_then(|a| level.player(a));
        if let Some(p) = &player
            && !p.may_build
        {
            return false;
        }
        // `isCreativePlayer`: a creative player's own hit.
        if player.as_ref().is_some_and(|p| p.creative) && kind == DamageKind::PlayerAttack {
            self.play_broken_sound(e, level);
            self.show_breaking_particles(e, level);
            self.kill(e, level, attacker);
            return true;
        }
        let now = level.game_time();
        if now - self.last_hit > 5 && !always_kills {
            level.emit(Event::EntityEvent { entity: e.id, event: 32 });
            level.emit(Event::GameEvent { event: "minecraft:entity_damage", pos: e.position(), entity: attacker });
            self.last_hit = now;
        } else {
            self.broken_by_player(e, level);
            self.show_breaking_particles(e, level);
            self.kill(e, level, attacker);
        }
        true
    }
}
