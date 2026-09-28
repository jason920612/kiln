//! Player health, damage, death and respawn (vanilla `ServerPlayer.hurtServer`,
//! `Player.hurtServer`/`actuallyHurt`, `LivingEntity.hurtServer`, `CombatTracker`,
//! `ServerPlayer.die`, `PlayerList.respawn`).
//!
//! Damage goes through vanilla's pipeline: invulnerability (game mode, damage type tags, the
//! game rules), difficulty scaling, the 20-tick hurt cooldown with its "only the excess over
//! the last hit" rule, armor and toughness from equipment attributes (`CombatRules`), armor
//! durability, absorption and the damage type's exhaustion. Enchantments take part through
//! `EnchantmentHelper` (see [`crate::enchant`]): damage immunity (frost walker), protection,
//! armor effectiveness (breach) and unbreaking on armor. Mob effects take part too: fire
//! resistance makes fire damage miss, resistance takes 20% per level after armor. Totems of
//! undying (`death_protection` in a hand) save a dying player. Not modelled yet: shields.
//!
//! Food follows `FoodData`: exhaustion from sprinting, jumping, fighting and breaking blocks
//! uses up saturation then food; a well-fed player heals, a starving one takes damage.

use crate::{Player, combat, entities};
use kiln_entity::level::DamageKind;
use kiln_proto::nbt::Tag;
use kiln_proto::packets;
use kiln_proto::packets::entity;

/// A new player's health (`Attributes.MAX_HEALTH`'s base).
pub(crate) const MAX_HEALTH: f32 = 20.0;
/// `LivingEntity.onBelowWorld`: damage per tick below the world.
const VOID_DAMAGE: f32 = 4.0;
/// Players take void damage this far below the dimension's bottom.
pub(crate) const VOID_DEPTH: f64 = 64.0;
/// `Attributes.SAFE_FALL_DISTANCE` base value.
const SAFE_FALL_DISTANCE: f64 = 3.0;
/// `FoodData.addExhaustion` cap.
const MAX_EXHAUSTION: f32 = 40.0;
/// `LivingEntity.damageCooldownTime` after a full hit; hits while it is above half only deal
/// what exceeds the last one.
const HURT_COOLDOWN: i32 = 20;
/// `Player.getLastHurtByPlayerMemoryTime`: ticks a player attacker gets the kill credit.
const KILL_CREDIT_TICKS: i32 = 100;

/// The game rules and difficulty damage depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DamageRules {
    /// `minecraft:pvp`.
    pub pvp: bool,
    /// `minecraft:fall_damage`, `fire_damage`, `freeze_damage`, `drowning_damage`.
    pub fall: bool,
    pub fire: bool,
    pub freeze: bool,
    pub drowning: bool,
    /// 0 (peaceful) to 3 (hard).
    pub difficulty: u8,
}

impl Default for DamageRules {
    fn default() -> Self {
        DamageRules { pvp: true, fall: true, fire: true, freeze: true, drowning: true, difficulty: 2 }
    }
}

/// Where damage's side effects go.
pub(crate) struct DamageCtx<'a> {
    pub rules: DamageRules,
    pub game_time: i64,
    /// Items dropped by players that died.
    pub spawns: &'a mut Vec<entities::Spawn>,
    /// Deaths to announce in a serial phase.
    pub deaths: &'a mut Vec<Death>,
    /// The random enchantment effects draw from while an attack is carried out (the
    /// attacker's; see [`crate::enchant`]); `None` uses the hurt player's own.
    pub level_rng: Option<kiln_javamath::random::LegacyRandom>,
}

/// What hurt a player: a damage type in `minecraft:damage_type`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Cause {
    /// `/kill`: bypasses invulnerability.
    Kill,
    OutOfWorld,
    /// Landing after falling this far.
    Fall(f64),
    Starve,
    /// Damage from an entity's behaviour (explosions, falling blocks, arrows, ...).
    Entity(DamageKind),
    /// A player's melee hit (`player_attack`).
    PlayerAttack,
    /// Any other damage type caused by an entity (`thorns` from an enchantment effect).
    Other(&'static str),
}

/// The entity responsible for damage (`DamageSource.getEntity`), as the victim needs it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Attacker {
    pub id: i32,
    pub name: String,
    pub pos: [f64; 3],
    /// A creative player (`DamageSource.isCreativePlayer`).
    pub creative: bool,
    /// The display name of a custom-named main hand item, for the `.item` death messages.
    pub weapon: Option<Tag>,
    /// The attacker as enchantment requirements see it.
    pub view: crate::enchant::EntityView,
    /// The attacker's entity type when it is a mob (a player otherwise).
    pub mob: Option<&'static str>,
}

impl Attacker {
    /// The attacker's name in death messages: a player's name, or a mob type's translation.
    fn name_tag(&self) -> Tag {
        match self.mob {
            Some(t) => translate_plain(&format!("entity.minecraft.{}", t.trim_start_matches("minecraft:"))),
            None => text(&self.name),
        }
    }

    /// A mob attacker.
    pub(crate) fn mob(id: i32, type_name: &'static str, pos: [f64; 3]) -> Attacker {
        let type_id = kiln_item::registry::ENTITY_TYPE.id(type_name).unwrap_or(-1);
        Attacker {
            id,
            name: type_name.to_owned(),
            pos,
            creative: false,
            weapon: None,
            view: crate::enchant::EntityView { type_id, pos, on_ground: true, on_fire: false, sneaking: false, sprinting: false, flying: false },
            mob: Some(type_name),
        }
    }
}

/// `DamageSource`: a damage type, who caused it and what dealt it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    pub cause: Cause,
    pub attacker: Option<Attacker>,
    /// Network id of the entity that dealt the damage when it is not the attacker (an arrow).
    pub direct: Option<i32>,
    /// `getWeaponItem`: the attacker's main hand item for melee hits.
    pub weapon: Option<kiln_item::ItemStack>,
}

impl From<Cause> for Source {
    fn from(cause: Cause) -> Self {
        Source { cause, attacker: None, direct: None, weapon: None }
    }
}

impl Cause {
    pub(crate) fn damage_type(self) -> &'static str {
        match self {
            Cause::Kill => "minecraft:generic_kill",
            Cause::OutOfWorld => "minecraft:out_of_world",
            Cause::Fall(_) => "minecraft:fall",
            Cause::Starve => "minecraft:starve",
            Cause::Entity(kind) => entities::damage_type(kind).0,
            Cause::PlayerAttack => "minecraft:player_attack",
            Cause::Other(name) => name,
        }
    }
}

impl Source {
    pub(crate) fn melee(attacker: Attacker, weapon: kiln_item::ItemStack) -> Source {
        Source { cause: Cause::PlayerAttack, attacker: Some(attacker), direct: None, weapon: Some(weapon) }
    }

    fn type_name(&self) -> &'static str {
        self.cause.damage_type()
    }

    /// Network id of the damage type.
    pub(crate) fn type_id(&self) -> i32 {
        kiln_data::synced_id("minecraft:damage_type", self.type_name()).unwrap_or(0)
    }

    /// A `minecraft:damage_type` tag such as `minecraft:bypasses_armor`.
    pub(crate) fn is(&self, tag: &str) -> bool {
        damage_type_tag(self.type_id(), tag)
    }

    fn info(&self) -> &'static DamageTypeInfo {
        damage_type_info(self.type_name())
    }

    /// `DamageSource.scalesWithDifficulty`: players hurt by these take more on hard and less on
    /// easy. No living non-player entity (mob) deals damage yet.
    fn scales_with_difficulty(&self) -> bool {
        match self.info().scaling {
            Scaling::Always => true,
            Scaling::WhenCausedByLivingNonPlayer => self.attacker.as_ref().is_some_and(|a| a.mob.is_some()),
            Scaling::Never => false,
        }
    }

    /// `DamageSource.getLocalizedDeathMessage`, with the victim's kill credit (the last player
    /// that hurt it) for sources without an attacker.
    fn death_message(&self, victim: &str, kill_credit: Option<&str>) -> Tag {
        let key = format!("death.attack.{}", self.info().message_id);
        if let Some(a) = &self.attacker {
            return match &a.weapon {
                Some(item) => translate(&format!("{key}.item"), vec![text(victim), a.name_tag(), item.clone()]),
                None => translate(&key, vec![text(victim), a.name_tag()]),
            };
        }
        match kill_credit {
            Some(killer) => translate(&format!("{key}.player"), vec![text(victim), text(killer)]),
            None => translate(&key, vec![text(victim)]),
        }
    }
}

/// A plain string component.
fn text(s: &str) -> Tag {
    Tag::String(s.into())
}

/// `Component.translatable(key, with...)`, keys in the order vanilla writes them (its
/// compounds are hash maps). NBT lists hold one type: with a styled argument, plain ones
/// become `{"text": ...}` compounds.
fn translate(key: &str, with: Vec<Tag>) -> Tag {
    let mixed = with.iter().any(|t| matches!(t, Tag::Compound(_)));
    let with = if mixed {
        with.into_iter()
            .map(|t| match t {
                Tag::String(s) => Tag::Compound(vec![("text".into(), Tag::String(s))]),
                other => other,
            })
            .collect()
    } else {
        with
    };
    Tag::Compound(vec![("with".into(), Tag::List(with)), ("translate".into(), Tag::String(key.into()))])
}

fn translate_plain(key: &str) -> Tag {
    Tag::Compound(vec![("translate".into(), Tag::String(key.into()))])
}

/// Whether a `minecraft:damage_type` network id is in a damage type tag.
pub(crate) fn damage_type_tag(id: i32, tag: &str) -> bool {
    kiln_data::registries::TAGS
        .iter()
        .find(|(r, _)| *r == "minecraft:damage_type")
        .and_then(|(_, tags)| tags.iter().find(|(t, _)| *t == tag))
        .is_some_and(|(_, ids)| ids.contains(&id))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Scaling {
    #[allow(dead_code)]
    Never,
    WhenCausedByLivingNonPlayer,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeathMessageType {
    Default,
    FallVariants,
    IntentionalGameDesign,
}

/// A damage type's definition (`DamageType`, from the vanilla datapack).
struct DamageTypeInfo {
    name: &'static str,
    message_id: &'static str,
    exhaustion: f32,
    scaling: Scaling,
    death_message: DeathMessageType,
}

macro_rules! damage_types {
    ($($name:literal $msg:literal $exh:literal $scaling:ident $death:ident;)*) => {
        &[$(DamageTypeInfo {
            name: concat!("minecraft:", $name),
            message_id: $msg,
            exhaustion: $exh,
            scaling: Scaling::$scaling,
            death_message: DeathMessageType::$death,
        },)*]
    };
}

/// `data/minecraft/damage_type/*.json` of the 26.3 datapack (checked against the extracted
/// datapack by a test when it is present).
const DAMAGE_TYPES: &[DamageTypeInfo] = damage_types! {
    "arrow" "arrow" 0.1 WhenCausedByLivingNonPlayer Default;
    "bad_respawn_point" "badRespawnPoint" 0.1 Always IntentionalGameDesign;
    "cactus" "cactus" 0.1 WhenCausedByLivingNonPlayer Default;
    "campfire" "inFire" 0.1 WhenCausedByLivingNonPlayer Default;
    "cramming" "cramming" 0.0 WhenCausedByLivingNonPlayer Default;
    "dragon_breath" "dragonBreath" 0.0 WhenCausedByLivingNonPlayer Default;
    "drown" "drown" 0.0 WhenCausedByLivingNonPlayer Default;
    "dry_out" "dryout" 0.1 WhenCausedByLivingNonPlayer Default;
    "ender_pearl" "fall" 0.0 WhenCausedByLivingNonPlayer FallVariants;
    "explosion" "explosion" 0.1 Always Default;
    "fall" "fall" 0.0 WhenCausedByLivingNonPlayer FallVariants;
    "falling_anvil" "anvil" 0.1 WhenCausedByLivingNonPlayer Default;
    "falling_block" "fallingBlock" 0.1 WhenCausedByLivingNonPlayer Default;
    "falling_stalactite" "fallingStalactite" 0.1 WhenCausedByLivingNonPlayer Default;
    "fireball" "fireball" 0.1 WhenCausedByLivingNonPlayer Default;
    "fireworks" "fireworks" 0.1 WhenCausedByLivingNonPlayer Default;
    "fly_into_wall" "flyIntoWall" 0.0 WhenCausedByLivingNonPlayer Default;
    "freeze" "freeze" 0.0 WhenCausedByLivingNonPlayer Default;
    "generic" "generic" 0.0 WhenCausedByLivingNonPlayer Default;
    "generic_kill" "genericKill" 0.0 WhenCausedByLivingNonPlayer Default;
    "hot_floor" "hotFloor" 0.1 WhenCausedByLivingNonPlayer Default;
    "in_fire" "inFire" 0.1 WhenCausedByLivingNonPlayer Default;
    "in_wall" "inWall" 0.0 WhenCausedByLivingNonPlayer Default;
    "indirect_magic" "indirectMagic" 0.0 WhenCausedByLivingNonPlayer Default;
    "lava" "lava" 0.1 WhenCausedByLivingNonPlayer Default;
    "lightning_bolt" "lightningBolt" 0.1 WhenCausedByLivingNonPlayer Default;
    "mace_smash" "mace_smash" 0.1 WhenCausedByLivingNonPlayer Default;
    "magic" "magic" 0.0 WhenCausedByLivingNonPlayer Default;
    "mob_attack" "mob" 0.1 WhenCausedByLivingNonPlayer Default;
    "mob_attack_no_aggro" "mob" 0.1 WhenCausedByLivingNonPlayer Default;
    "mob_projectile" "mob" 0.1 WhenCausedByLivingNonPlayer Default;
    "on_fire" "onFire" 0.0 WhenCausedByLivingNonPlayer Default;
    "out_of_world" "outOfWorld" 0.0 WhenCausedByLivingNonPlayer Default;
    "outside_border" "outsideBorder" 0.0 WhenCausedByLivingNonPlayer Default;
    "player_attack" "player" 0.1 WhenCausedByLivingNonPlayer Default;
    "player_explosion" "explosion.player" 0.1 Always Default;
    "sonic_boom" "sonic_boom" 0.0 Always Default;
    "spear" "spear" 0.1 WhenCausedByLivingNonPlayer Default;
    "spit" "mob" 0.1 WhenCausedByLivingNonPlayer Default;
    "stalagmite" "stalagmite" 0.0 WhenCausedByLivingNonPlayer Default;
    "starve" "starve" 0.0 WhenCausedByLivingNonPlayer Default;
    "sting" "sting" 0.1 WhenCausedByLivingNonPlayer Default;
    "sulfur_cube_hot" "sulfurCubeHot" 0.1 WhenCausedByLivingNonPlayer Default;
    "sweet_berry_bush" "sweetBerryBush" 0.1 WhenCausedByLivingNonPlayer Default;
    "thorns" "thorns" 0.1 WhenCausedByLivingNonPlayer Default;
    "thrown" "thrown" 0.1 WhenCausedByLivingNonPlayer Default;
    "trident" "trident" 0.1 WhenCausedByLivingNonPlayer Default;
    "unattributed_fireball" "onFire" 0.1 WhenCausedByLivingNonPlayer Default;
    "wind_charge" "mob" 0.1 WhenCausedByLivingNonPlayer Default;
    "wither" "wither" 0.0 WhenCausedByLivingNonPlayer Default;
    "wither_skull" "witherSkull" 0.1 WhenCausedByLivingNonPlayer Default;
};

/// The `'static` name of a vanilla damage type (`generic` for unknown ones).
pub(crate) fn static_damage_type(name: &str) -> &'static str {
    damage_type_info(name).name
}

fn damage_type_info(name: &str) -> &'static DamageTypeInfo {
    DAMAGE_TYPES.iter().find(|t| t.name == name).unwrap_or_else(|| damage_type_info("minecraft:generic"))
}

/// `CombatEntry`: one hit the combat tracker remembers.
#[derive(Debug, Clone)]
pub(crate) struct CombatEntry {
    source: Source,
    /// The victim's fall distance when it was hit.
    fall_distance: f32,
}

/// `CombatTracker`: recent hits, for the death message. Entries are forgotten 100 ticks (300 in
/// combat, i.e. after a hit from an entity) after the last hit.
#[derive(Debug, Clone, Default)]
pub(crate) struct CombatTracker {
    entries: Vec<CombatEntry>,
    last_damage_time: i64,
    taking_damage: bool,
    in_combat: bool,
}

impl CombatTracker {
    /// `recheckStatus`.
    pub(crate) fn recheck(&mut self, now: i64, alive: bool) {
        let limit = if self.in_combat { 300 } else { 100 };
        if self.taking_damage && (!alive || now - self.last_damage_time > limit) {
            self.taking_damage = false;
            self.in_combat = false;
            self.entries.clear();
        }
    }

    fn record(&mut self, source: &Source, fall_distance: f32, now: i64, alive: bool) {
        self.recheck(now, alive);
        self.entries.push(CombatEntry { source: source.clone(), fall_distance });
        self.last_damage_time = now;
        self.taking_damage = true;
        // `shouldEnterCombat`: the attacker is a living entity.
        if !self.in_combat && alive && source.attacker.is_some() {
            self.in_combat = true;
        }
    }

    /// `getMostSignificantFall`: the hit that led to the longest fall (over 5 blocks). Fall
    /// locations (ladders, vines, water) are not tracked, so only the fall rule applies.
    fn most_significant_fall(&self) -> Option<&CombatEntry> {
        let (mut best, mut best_distance) = (None, 0.0f32);
        for (i, e) in self.entries.iter().enumerate() {
            let always = e.source.is("minecraft:always_most_significant_fall");
            let distance = if always { f32::MAX } else { e.fall_distance };
            if (e.source.is("minecraft:is_fall") || always) && distance > 0.0 && (best.is_none() || distance > best_distance) {
                best = Some(if i > 0 { &self.entries[i - 1] } else { e });
                best_distance = distance;
            }
        }
        best.filter(|_| best_distance > 5.0)
    }

    /// `getDeathMessage`.
    fn death_message(&self, victim: &str, kill_credit: Option<&str>) -> Tag {
        let Some(last) = self.entries.last() else {
            return translate("death.attack.generic", vec![text(victim)]);
        };
        let info = last.source.info();
        if info.death_message == DeathMessageType::FallVariants
            && let Some(fall) = self.most_significant_fall()
        {
            return fall_message(fall, last.source.attacker.as_ref(), victim);
        }
        if info.death_message == DeathMessageType::IntentionalGameDesign {
            let key = format!("death.attack.{}", info.message_id);
            let link = translate(
                "chat.square_brackets",
                vec![translate_plain(&format!("{key}.link"))],
            );
            return translate(&format!("{key}.message"), vec![text(victim), link]);
        }
        last.source.death_message(victim, kill_credit)
    }
}

/// `CombatTracker.getFallMessage`: a fall after a hit credits whoever hit.
fn fall_message(fall: &CombatEntry, killer: Option<&Attacker>, victim: &str) -> Tag {
    let source = &fall.source;
    if source.is("minecraft:is_fall") || source.is("minecraft:always_most_significant_fall") {
        return translate("death.fell.accident.generic", vec![text(victim)]);
    }
    let assisted = |a: &Attacker, item: &str, plain: &str| match &a.weapon {
        Some(w) => translate(item, vec![text(victim), a.name_tag(), w.clone()]),
        None => translate(plain, vec![text(victim), a.name_tag()]),
    };
    match (&source.attacker, killer) {
        (Some(a), k) if k.is_none_or(|k| k.name != a.name) => assisted(a, "death.fell.assist.item", "death.fell.assist"),
        (_, Some(k)) => assisted(k, "death.fell.finish.item", "death.fell.finish"),
        _ => translate("death.fell.killer", vec![text(victim)]),
    }
}

/// A death to announce in a serial phase (the message goes to everyone).
pub(crate) struct Death {
    pub conn: kiln_link::ConnId,
    pub message: Tag,
    /// The player credited with the kill (`awardKillScore` in the serial phase).
    pub killer: Option<String>,
}

impl Player {
    /// Creative and spectator players are invulnerable (`Abilities.invulnerable`).
    pub(crate) fn invulnerable(&self) -> bool {
        matches!(self.game_mode, 1 | 3)
    }

    /// `ServerGamePacketListenerImpl.hasClientLoaded`: the client reported it finished loading
    /// terrain, or the timeout ran out.
    pub(crate) fn client_loaded(&self) -> bool {
        self.load_timeout == 0
    }

    pub(crate) fn health_packet(&self) -> bytes::Bytes {
        packets::player::set_health(self.health, self.food, self.saturation)
    }

    /// `ServerPlayer.isInvulnerableTo`, `Player.isInvulnerableTo` and
    /// `LivingEntity.isInvulnerableTo` (enchantment damage immunity: frost walker).
    fn invulnerable_to(&mut self, source: &Source, ctx: &mut DamageCtx) -> bool {
        let rules = &ctx.rules;
        if !self.client_loaded()
            || (source.is("minecraft:is_drowning") && !rules.drowning)
            || (source.is("minecraft:is_fall") && !rules.fall)
            || (source.is("minecraft:is_fire") && !rules.fire)
            || (source.is("minecraft:is_freezing") && !rules.freeze)
        {
            return true;
        }
        let Some(loot) = self.loot.clone() else { return false };
        let view = self.view();
        let rng = ctx.level_rng.as_mut().unwrap_or(&mut self.level_rng);
        let equipment: Vec<_> = combat::SLOTS.iter().map(|s| (*s, self.inv.equipped(*s))).collect();
        loot.is_immune_to_damage(&equipment, rng, |level| crate::enchant::DamageContext { level, this: &view, source })
    }

    /// `ServerPlayer.hurtServer` down to `LivingEntity.hurtServer`: damages the player and
    /// returns whether the hit landed (it may deal only the excess over the last hit while the
    /// hurt cooldown runs). A death goes to `ctx.deaths`.
    pub(crate) fn hurt(&mut self, amount: f32, source: &Source, ctx: &mut DamageCtx) -> bool {
        let rules = ctx.rules;
        if self.invulnerable_to(source, ctx) {
            return false;
        }
        // `canHarmPlayer`: player attackers (melee or their arrows) need PvP on.
        if source.attacker.is_some() && !rules.pvp && self.hurt_by_player(source) {
            return false;
        }
        if self.invulnerable() && !source.is("minecraft:bypasses_invulnerability") {
            return false;
        }
        if self.dead {
            return false;
        }
        if source.is("minecraft:is_fire") && self.has_effect("minecraft:fire_resistance") {
            return false;
        }
        let health_before = self.health;
        let mut amount = amount;
        if source.scales_with_difficulty() {
            amount = match rules.difficulty {
                0 => 0.0,
                1 => (amount / 2.0 + 1.0).min(amount),
                3 => amount * 3.0 / 2.0,
                _ => amount,
            };
        }
        if amount == 0.0 {
            return false;
        }
        // `LivingEntity.hurtServer`.
        amount = amount.max(0.0);
        if source.is("minecraft:damages_helmet") && !self.inv.equipped(kiln_item::component::EquipmentSlot::Head).is_empty() {
            self.hurt_equipment(source, amount, &[kiln_item::component::EquipmentSlot::Head], ctx);
            amount *= 0.75;
        }
        if !amount.is_finite() {
            amount = f32::MAX;
        }
        let full = if self.hurt_cooldown as f32 > 10.0 && !source.is("minecraft:bypasses_cooldown") {
            if amount <= self.last_hurt {
                return false;
            }
            self.actually_hurt(amount - self.last_hurt, source, ctx);
            self.last_hurt = amount;
            false
        } else {
            self.last_hurt = amount;
            self.hurt_cooldown = HURT_COOLDOWN;
            self.actually_hurt(amount, source, ctx);
            true
        };
        // `resolvePlayerResponsibleForDamage`.
        if let Some(a) = &source.attacker
            && self.hurt_by_player(source)
        {
            self.kill_credit = Some((a.name.clone(), KILL_CREDIT_TICKS));
        }
        // `getKillCredit` falls back to the last mob that hurt the player.
        if let Some(t) = source.attacker.as_ref().and_then(|a| a.mob) {
            self.last_mob_attacker = Some((t, ctx.game_time));
        }
        if full {
            // `broadcastDamageEvent` (to viewers in the movement phase) and `markHurt`.
            let damage_event = (source.type_id(), source.attacker.as_ref().map(|a| a.id), source.direct.or(source.attacker.as_ref().map(|a| a.id)));
            self.send(entity::damage_event(self.entity_id, damage_event.0, damage_event.1, damage_event.2, None));
            self.damaged = Some(damage_event);
            if !source.is("minecraft:no_impact") {
                self.sync_velocity = true;
            }
            // `dealDefaultKnockback` from the source's position (melee: the attacker's).
            if !source.is("minecraft:no_knockback")
                && matches!(source.cause, Cause::PlayerAttack | Cause::Other(_) | Cause::Entity(DamageKind::MobAttack))
                && let Some(a) = &source.attacker
            {
                let (dx, dz) = (a.pos[0] - self.pos[0], a.pos[2] - self.pos[2]);
                self.knockback(0.4000000059604645, dx, dz);
            }
        }
        // `EntityHurtPlayerTrigger` (dealt before armor and effects, taken after).
        let taken = health_before - self.health;
        let attacker_view = source.attacker.as_ref().map(|a| a.view.clone());
        let killer = attacker_view.as_ref().map(|v| crate::advancements::criteria::Subject {
            type_id: v.type_id,
            pos: v.pos,
            dim: crate::DIMENSIONS[self.dim].0,
            on_ground: v.on_ground,
            on_fire: v.on_fire,
            sneaking: v.sneaking,
            sprinting: v.sprinting,
            flying: v.flying,
            baby: false,
            equipment: Vec::new(),
            world: None,
            components: Default::default(),
            effects: Vec::new(),
            vehicle: None,
        });
        self.hurt_trigger("minecraft:entity_hurt_player", killer.as_ref(), amount, taken, source.cause.damage_type());
        if self.health <= 0.0 && !self.check_totem_death_protection(source) {
            let death = self.die(ctx);
            ctx.deaths.push(death);
            // `KilledTrigger` for the killer's side (`entity_killed_player`).
            if let Some(k) = &killer {
                self.killed("minecraft:entity_killed_player", k, source.cause.damage_type(), source.direct.is_none());
            }
        }
        true
    }

    /// Whether the damage's attacker is a player.
    fn hurt_by_player(&self, source: &Source) -> bool {
        source.attacker.as_ref().is_some_and(|a| a.mob.is_none())
    }

    /// `Player.actuallyHurt`: armor, absorption, exhaustion and the combat tracker.
    fn actually_hurt(&mut self, amount: f32, source: &Source, ctx: &mut DamageCtx) {
        if self.invulnerable_to(source, ctx) {
            return;
        }
        let mut damage = self.damage_after_armor(source, amount, ctx);
        damage = self.damage_after_magic(source, damage, ctx);
        let before = damage;
        damage = (damage - self.absorption).max(0.0);
        self.absorption = (self.absorption - (before - damage)).max(0.0);
        let absorbed = before - damage;
        if absorbed > 0.0 && absorbed < 3.4028235e37 {
            self.award_stat(*crate::player_stats::stat::DAMAGE_ABSORBED, (absorbed * 10.0).round() as i32);
        }
        if damage == 0.0 {
            return;
        }
        self.exhaust(source.info().exhaustion);
        let fall = match source.cause {
            Cause::Fall(d) => d as f32,
            _ => self.fall_distance as f32,
        };
        self.combat.record(source, fall, ctx.game_time, self.health > 0.0);
        self.health = (self.health - damage).clamp(0.0, self.max_health());
        if damage < 3.4028235e37 {
            self.award_stat(*crate::player_stats::stat::DAMAGE_TAKEN, (damage * 10.0).round() as i32);
        }
    }

    /// `LivingEntity.getDamageAfterArmorAbsorb`: armor takes durability damage and reduces the
    /// damage unless the type bypasses armor.
    fn damage_after_armor(&mut self, source: &Source, amount: f32, ctx: &mut DamageCtx) -> f32 {
        if source.is("minecraft:bypasses_armor") {
            return amount;
        }
        use kiln_item::component::EquipmentSlot as S;
        self.hurt_equipment(source, amount, &[S::Feet, S::Legs, S::Chest, S::Head], ctx);
        let armor = combat::floor(self.attribute(combat::ARMOR)) as f32;
        let toughness = self.attribute(combat::ARMOR_TOUGHNESS) as f32;
        // `CombatRules.getDamageAfterAbsorb`: the weapon's enchantments change how much the
        // armor counts (breach).
        let loot = self.loot.clone();
        let view = self.view();
        let rng = ctx.level_rng.as_mut().unwrap_or(&mut self.level_rng);
        combat::damage_after_absorb(amount, armor, toughness, |h| match (&source.weapon, &loot) {
            (Some(weapon), Some(loot)) => loot
                .modify_armor_effectiveness(weapon, rng, h, |level| crate::enchant::DamageContext { level, this: &view, source })
                .clamp(0.0, 1.0),
            _ => h,
        })
    }

    /// `LivingEntity.getDamageAfterMagicAbsorb`: resistance takes 20% per level (unless the type
    /// bypasses it), never negative, then the equipment's enchantment protection unless the type
    /// bypasses enchantments.
    fn damage_after_magic(&mut self, source: &Source, damage: f32, ctx: &mut DamageCtx) -> f32 {
        if source.is("minecraft:bypasses_effects") {
            return damage;
        }
        let mut damage = damage;
        if let Some(amplifier) = self.effect_amplifier("minecraft:resistance")
            && !source.is("minecraft:bypasses_resistance")
        {
            let factor = 25 - (amplifier + 1) * 5;
            let before = damage;
            damage = (damage * factor as f32 / 25.0).max(0.0);
            let resisted = before - damage;
            if resisted > 0.0 && resisted < 3.4028235e37 {
                self.award_stat(*crate::player_stats::stat::DAMAGE_RESISTED, (resisted * 10.0).round() as i32);
            }
        }
        if damage <= 0.0 {
            return 0.0;
        }
        if source.is("minecraft:bypasses_enchantments") {
            return damage;
        }
        let Some(loot) = self.loot.clone() else { return damage };
        let view = self.view();
        let rng = ctx.level_rng.as_mut().unwrap_or(&mut self.level_rng);
        let equipment: Vec<_> = combat::SLOTS.iter().map(|s| (*s, self.inv.equipped(*s))).collect();
        let protection =
            loot.damage_protection(&equipment, rng, |level| crate::enchant::DamageContext { level, this: &view, source });
        if protection > 0.0 { damage_after_magic_absorb(damage, protection) } else { damage }
    }

    /// `LivingEntity.doHurtEquipment`: armor worn in `slots` loses `max(1, damage / 4)`
    /// durability.
    fn hurt_equipment(&mut self, source: &Source, damage: f32, slots: &[kiln_item::component::EquipmentSlot], ctx: &mut DamageCtx) {
        if damage <= 0.0 {
            return;
        }
        let amount = (damage / 4.0).max(1.0) as i32;
        for &slot in slots {
            let stack = self.inv.equipped(slot);
            let hurts = stack.get(kiln_item::keys::EQUIPPABLE).is_some_and(|e| e.damage_on_hurt)
                && stack.is_damageable_item()
                && can_be_hurt_by(stack, source);
            if hurts {
                self.hurt_and_break(slot, amount, ctx.level_rng.as_mut());
            }
        }
    }

    /// `ServerPlayer.die`: the death screen, the inventory scattered (no `keepInventory` yet),
    /// the death animation for viewers.
    fn die(&mut self, ctx: &mut DamageCtx) -> Death {
        self.dead = true;
        let credit = self.kill_credit.as_ref().map(|(name, _)| name.clone());
        // `ServerPlayer.die`: `deathCount`, `killed_by`, the deaths statistic and the timers.
        self.update_criterion("deathCount", crate::player_stats::ScoreOp::Add(1));
        let killer_type = if credit.is_some() {
            Some("minecraft:player")
        } else {
            self.last_mob_attacker.filter(|(_, t)| ctx.game_time - t <= 100).map(|(k, _)| k)
        };
        if let Some(id) = killer_type.and_then(|k| kiln_item::registry::ENTITY_TYPE.id(k)) {
            self.award_stat(crate::player_stats::Stat::entity(crate::player_stats::KILLED_BY, id), 1);
        }
        self.award_stat(*crate::player_stats::stat::DEATHS, 1);
        self.reset_stat(*crate::player_stats::stat::TIME_SINCE_DEATH);
        self.reset_stat(*crate::player_stats::stat::TIME_SINCE_REST);
        let message = self.combat.death_message(&self.name, credit.as_deref());
        self.fall_distance = 0.0;
        self.send(packets::player::player_combat_kill(self.entity_id, &message));
        // `ServerPlayer.die`: the fire goes out.
        self.clear_fire();
        self.sync_on_fire_flag();
        self.death_location = Some(self.pos.map(|c| c.floor() as i32));
        self.death_dim = self.dim;
        for i in 0..self.inv.items.len() {
            let stack = std::mem::replace(&mut self.inv.items[i], kiln_item::ItemStack::empty());
            if !stack.is_empty() {
                ctx.spawns.push(self.throw_randomly(stack));
            }
        }
        for i in 0..self.inv.equipment.len() {
            let stack = std::mem::replace(&mut self.inv.equipment[i], kiln_item::ItemStack::empty());
            if !stack.is_empty() {
                ctx.spawns.push(self.throw_randomly(stack));
            }
        }
        self.inv.times_changed += 1;
        // `LivingEntity.dropExperience` (players always drop it).
        let xp = self.death_experience(false);
        if xp > 0 {
            let at = self.pos;
            crate::container::furnace::award_experience(at, xp, &mut self.entity_rng, ctx.spawns);
        }
        self.died = true;
        // `broadcastEntityEvent(DEATH)` reaches the player too.
        self.send(entity::entity_event(self.entity_id, 3));
        Death { conn: self.conn, message, killer: credit }
    }

    /// Per-tick damage bookkeeping (`ServerPlayer.tick`'s cooldown, `LivingEntity.baseTick`'s
    /// kill credit expiry and combat tracker check).
    pub(crate) fn tick_damage(&mut self, game_time: i64) {
        if self.hurt_cooldown > 0 {
            self.hurt_cooldown -= 1;
        }
        if let Some((_, ticks)) = &mut self.kill_credit {
            if *ticks > 0 {
                *ticks -= 1;
            } else {
                self.kill_credit = None;
            }
        }
        self.combat.recheck(game_time, !self.dead && self.health > 0.0);
    }

    /// Sends Set Health when health, food or whether saturation is zero changed since the last
    /// one (`ServerPlayer.doTick`).
    pub(crate) fn sync_health(&mut self) {
        let now = (self.health.to_bits(), self.food, self.saturation == 0.0);
        if self.sent_health != Some(now) {
            self.sent_health = Some(now);
            self.send(self.health_packet());
        }
    }

    /// `LivingEntity.checkTotemDeathProtection`: an item with `death_protection` in a hand (the
    /// main hand first) is used up instead of dying: the `used` statistic and `used_totem`,
    /// health 1, its death effects, and the totem animation (entity event 35).
    pub(crate) fn check_totem_death_protection(&mut self, source: &Source) -> bool {
        use kiln_item::component::{ConsumeEffect, EquipmentSlot};
        if source.is("minecraft:bypasses_invulnerability") {
            return false;
        }
        for slot in [EquipmentSlot::MainHand, EquipmentSlot::OffHand] {
            let index = kiln_inventory::inventory::equipment_index(slot, self.inv.selected);
            let stack = kiln_inventory::Container::item(&self.inv, index);
            let Some(protection) = stack.get(kiln_item::keys::DEATH_PROTECTION).cloned() else { continue };
            let used = stack.clone();
            kiln_inventory::Container::item_mut(&mut self.inv, index).shrink(1);
            self.award_stat(crate::player_stats::Stat::item(crate::player_stats::USED, used.item()), 1);
            self.fire_conds("minecraft:used_totem", None, |c, _, loot| {
                c.item("item").is_none_or(|p| kiln_loot::predicate::item_matches(&loot.tags, p, &used))
            });
            self.health = 1.0;
            // `DeathProtection.applyEffects`: the consume effects that make sense here.
            for effect in &protection.death_effects {
                match effect {
                    ConsumeEffect::ClearAllEffects => {
                        self.remove_all_effects();
                    }
                    ConsumeEffect::ApplyEffects { effects, probability } => {
                        use kiln_javamath::random::RandomSource;
                        if self.entity_rng.next_float() < *probability {
                            for e in effects {
                                self.add_effect(crate::effects::Effect::from_item(e));
                            }
                        }
                    }
                    ConsumeEffect::RemoveEffects(kiln_item::HolderSet::Direct(ids)) => {
                        for &id in ids {
                            self.remove_effect(id);
                        }
                    }
                    _ => {}
                }
            }
            self.entity_events.push(35);
            self.send(entity::entity_event(self.entity_id, 35));
            return true;
        }
        false
    }

    /// `LivingEntity.heal`.
    pub(crate) fn heal(&mut self, amount: f32) {
        if self.health > 0.0 {
            self.health = (self.health + amount).min(self.max_health());
        }
    }

    /// `Player.causeFoodExhaustion`: nothing for invulnerable (creative, spectator) players.
    pub(crate) fn exhaust(&mut self, amount: f32) {
        if !self.invulnerable() {
            self.add_exhaustion(amount);
        }
    }

    /// `FoodData.addExhaustion`.
    fn add_exhaustion(&mut self, amount: f32) {
        self.exhaustion = (self.exhaustion + amount).min(MAX_EXHAUSTION);
    }

    /// `FoodData.tick` and the peaceful regeneration of `Player.aiStep`. `difficulty` is 0
    /// (peaceful) to 3 (hard).
    pub(crate) fn tick_food(&mut self, natural_regen: bool, ctx: &mut DamageCtx) {
        let difficulty = ctx.rules.difficulty;
        if difficulty == 0 && natural_regen {
            if self.health < self.max_health() && ctx.game_time % 20 == 0 {
                self.heal(1.0);
            }
            if self.food < 20 && ctx.game_time % 10 == 0 {
                self.food += 1;
            }
        }
        if self.exhaustion > 4.0 {
            self.exhaustion -= 4.0;
            if self.saturation > 0.0 {
                self.saturation = (self.saturation - 1.0).max(0.0);
            } else if difficulty != 0 {
                self.food = (self.food - 1).max(0);
            }
        }
        let hurt = self.health > 0.0 && self.health < self.max_health();
        if natural_regen && self.saturation > 0.0 && hurt && self.food >= 20 {
            self.food_timer += 1;
            if self.food_timer >= 10 {
                let f = self.saturation.min(6.0);
                self.heal(f / 6.0);
                self.add_exhaustion(f);
                self.food_timer = 0;
            }
        } else if natural_regen && self.food >= 18 && hurt {
            self.food_timer += 1;
            if self.food_timer >= 80 {
                self.heal(1.0);
                self.add_exhaustion(6.0);
                self.food_timer = 0;
            }
        } else if self.food <= 0 {
            self.food_timer += 1;
            if self.food_timer >= 80 {
                self.food_timer = 0;
                if self.health > 10.0 || difficulty == 3 || (self.health > 1.0 && difficulty == 2) {
                    self.hurt(1.0, &Cause::Starve.into(), ctx);
                }
            }
        } else {
            self.food_timer = 0;
        }
    }

    /// Exhaustion from a move by `d` (`Player.checkMovementStatistics`) and from jumping
    /// (`jumpFromGround`: left the ground going up).
    pub(crate) fn exhaust_for_move(&mut self, d: [f64; 3], was_on_ground: bool, in_water: bool) {
        if was_on_ground && !self.on_ground && d[1] > 0.0 {
            self.exhaust(if self.sprinting { 0.2 } else { 0.05 });
        }
        let horizontal = ((d[0] * d[0] + d[2] * d[2]).sqrt() as f32 * 100.0).round();
        if horizontal <= 0.0 {
            return;
        }
        if in_water {
            self.exhaust(0.01 * horizontal * 0.01);
        } else if self.on_ground && self.sprinting {
            self.exhaust(0.1 * horizontal * 0.01);
        }
    }

    /// Vanilla `Entity.checkFallDamage` for a reported move by `dy` ending `on_ground`.
    pub(crate) fn check_fall(&mut self, dy: f64, on_ground: bool, in_fluid: bool, ctx: &mut DamageCtx) {
        if in_fluid || self.game_mode == 3 || self.flying {
            self.reset_fall_distance();
            return;
        }
        // The move itself counts, landing included.
        if dy < 0.0 {
            self.fall_distance -= dy;
        }
        // `trackStartFallingPosition`.
        if self.fall_distance > 0.0 && self.starting_to_fall.is_none() {
            self.starting_to_fall = Some(self.pos);
        }
        if on_ground {
            let fell = self.fall_distance;
            // `Player.causeFallDamage`: falls of two blocks or more count, except with `mayfly`.
            if fell >= 2.0 && !matches!(self.game_mode, 1 | 3) {
                self.award_stat(*crate::player_stats::stat::FALL_ONE_CM, (fell * 100.0).round() as i32);
            }
            // `LivingEntity.calculateFallDamage`; creative players (`mayfly`) take none.
            let damage = (fell - SAFE_FALL_DISTANCE).floor();
            if damage > 0.0 && self.game_mode != 1 {
                self.hurt(damage as f32, &Cause::Fall(fell).into(), ctx);
            }
            // `resetFallDistance` after the damage, which the combat tracker records it with.
            self.reset_fall_distance();
        }
    }

    /// `ServerPlayer.resetFallDistance`: a living player who was falling fires
    /// `fall_from_height` (from where the fall began to here).
    pub(crate) fn reset_fall_distance(&mut self) {
        if let Some(start) = self.starting_to_fall.take()
            && self.health > 0.0
        {
            self.distance_trigger("minecraft:fall_from_height", start);
        }
        self.fall_distance = 0.0;
    }

    /// Void damage every tick below the world (`Entity.checkBelowWorld`).
    pub(crate) fn check_void(&mut self, min_y: i32, ctx: &mut DamageCtx) {
        if self.pos[1] < min_y as f64 - VOID_DEPTH {
            self.hurt(VOID_DAMAGE, &Cause::OutOfWorld.into(), ctx);
        }
    }

    /// An item flung in a random direction (`Player.drop(stack, throwRandomly = true)`).
    fn throw_randomly(&mut self, stack: kiln_item::ItemStack) -> entities::Spawn {
        let f = self.rng.next_f32() * 0.5;
        let a = self.rng.next_f32() * std::f32::consts::TAU;
        let vel = [(-a.sin() * f) as f64, 0.2, (a.cos() * f) as f64];
        entities::Spawn {
            kind: &kiln_data::entities::types::ITEM,
            pos: [self.pos[0], self.pos[1] + 1.62 - 0.3, self.pos[2]],
            vel,
            body: entities::Body::Item { stack, pickup_delay: entities::DROP_PICKUP_DELAY, thrower: None },
        }
    }
}

/// `CombatRules.getDamageAfterMagicAbsorb`: protection points (up to 20) take 4% each.
pub(crate) fn damage_after_magic_absorb(damage: f32, protection: f32) -> f32 {
    let p = protection.clamp(0.0, 20.0);
    damage * (1.0 - p / 25.0)
}

/// `ItemStack.canBeHurtBy`: false when the `damage_resistant` component covers the source.
fn can_be_hurt_by(stack: &kiln_item::ItemStack, source: &Source) -> bool {
    let Some(resistant) = stack.get(kiln_item::keys::DAMAGE_RESISTANT) else { return true };
    match &resistant.types {
        kiln_item::HolderSet::Tag(t) => !source.is(t.as_str()),
        kiln_item::HolderSet::Direct(ids) => !ids.contains(&source.type_id()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_type_table_matches_the_datapack() {
        let Some(dir) = std::env::var_os("KILN_WORK")
            .map(std::path::PathBuf::from)
            .or_else(|| Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../work")))
            .map(|w| w.join("generated/data/minecraft/damage_type"))
            .filter(|d| d.is_dir())
        else {
            eprintln!("skipped: no extracted datapack");
            return;
        };
        let mut seen = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let name = format!("minecraft:{}", path.file_stem().unwrap().to_str().unwrap());
            let t = DAMAGE_TYPES.iter().find(|t| t.name == name).unwrap_or_else(|| panic!("{name} missing"));
            assert_eq!(t.message_id, v["message_id"].as_str().unwrap(), "{name}");
            assert_eq!(t.exhaustion, v["exhaustion"].as_f64().unwrap() as f32, "{name}");
            let scaling = match v["scaling"].as_str().unwrap() {
                "never" => Scaling::Never,
                "always" => Scaling::Always,
                _ => Scaling::WhenCausedByLivingNonPlayer,
            };
            assert_eq!(t.scaling, scaling, "{name}");
            let death = match v.get("death_message_type").and_then(|d| d.as_str()) {
                Some("fall_variants") => DeathMessageType::FallVariants,
                Some("intentional_game_design") => DeathMessageType::IntentionalGameDesign,
                _ => DeathMessageType::Default,
            };
            assert_eq!(t.death_message, death, "{name}");
            seen += 1;
        }
        assert_eq!(seen, DAMAGE_TYPES.len());
    }

    #[test]
    fn damage_type_tags() {
        let id = |n: &str| kiln_data::synced_id("minecraft:damage_type", n).unwrap();
        assert!(damage_type_tag(id("minecraft:fall"), "minecraft:bypasses_armor"));
        assert!(!damage_type_tag(id("minecraft:player_attack"), "minecraft:bypasses_armor"));
        assert!(damage_type_tag(id("minecraft:out_of_world"), "minecraft:bypasses_invulnerability"));
        assert!(damage_type_tag(id("minecraft:generic_kill"), "minecraft:bypasses_invulnerability"));
    }
}
