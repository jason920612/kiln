//! Brain memories: `MemoryModuleType`, the values they hold, `MemorySlot` (a value with an
//! optional time to live), `MemoryStatus`, `WalkTarget` and the position trackers.

use crate::math::{BlockPos, Vec3};
use crate::mob::DamageSource;

/// `MemoryModuleType` (registry name = the lowercase constant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Mem {
    /// core.GlobalPos
    Home,
    /// core.GlobalPos
    JobSite,
    /// core.GlobalPos
    PotentialJobSite,
    /// core.GlobalPos
    MeetingPoint,
    /// List<core.GlobalPos>
    SecondaryJobSite,
    /// List<LivingEntity>
    NearestLivingEntities,
    /// memory.NearestVisibleLivingEntities
    NearestVisibleLivingEntities,
    /// List<LivingEntity>
    VisibleVillagerBabies,
    /// List<player.Player>
    NearestPlayers,
    /// player.Player
    NearestVisiblePlayer,
    /// player.Player
    NearestVisibleAttackablePlayer,
    /// List<player.Player>
    NearestVisibleAttackablePlayers,
    /// memory.WalkTarget
    WalkTarget,
    /// behavior.PositionTracker
    LookTarget,
    /// LivingEntity
    AttackTarget,
    /// Boolean
    AttackCoolingDown,
    /// LivingEntity
    InteractionTarget,
    /// AgeableMob
    BreedTarget,
    /// Entity
    RideTarget,
    /// pathfinder.Path
    Path,
    /// Set<core.GlobalPos>
    DoorsToClose,
    /// core.BlockPos
    NearestBed,
    /// damagesource.DamageSource
    HurtBy,
    /// LivingEntity
    HurtByEntity,
    /// LivingEntity
    AvoidTarget,
    /// LivingEntity
    NearestHostile,
    /// LivingEntity
    NearestAttackable,
    /// core.GlobalPos
    HidingPlace,
    /// Long
    HeardBellTime,
    /// Long
    CantReachWalkTargetSince,
    /// Boolean
    GolemDetectedRecently,
    /// Boolean
    DangerDetectedRecently,
    /// Long
    LastSlept,
    /// Long
    LastWoken,
    /// Long
    LastWorkedAtPoi,
    /// LivingEntity
    NearestVisibleAdult,
    /// item.ItemEntity
    NearestVisibleWantedItem,
    /// Mob
    NearestVisibleNemesis,
    /// Integer
    PlayDeadTicks,
    /// player.Player
    TemptingPlayer,
    /// Integer
    TemptationCooldownTicks,
    /// Integer
    GazeCooldownTicks,
    /// Integer
    LongJumpCooldownTicks,
    /// Boolean
    LongJumpMidJump,
    /// Boolean
    HasHuntingCooldown,
    /// Integer
    RamCooldownTicks,
    /// phys.Vec3
    RamTarget,
    /// util.Unit
    IsInWater,
    /// util.Unit
    IsPregnant,
    /// Boolean
    IsPanicking,
    /// List<UUID>
    UnreachableTongueTargets,
    /// Set<core.GlobalPos>
    VisitedBlockPositions,
    /// Set<core.GlobalPos>
    UnreachableTransportBlockPositions,
    /// Integer
    TransportItemsCooldownTicks,
    /// Integer
    ChargeCooldownTicks,
    /// Integer
    AttackTargetCooldown,
    /// Integer
    SpearFleeingTime,
    /// phys.Vec3
    SpearFleeingPosition,
    /// phys.Vec3
    SpearChargePosition,
    /// Integer
    SpearEngageTime,
    /// behavior.SpearAttack$SpearStatus
    SpearStatus,
    /// UUID
    AngryAt,
    /// Boolean
    UniversalAnger,
    /// Boolean
    AdmiringItem,
    /// Integer
    TimeTryingToReachAdmireItem,
    /// Boolean
    DisableWalkToAdmireItem,
    /// Boolean
    AdmiringDisabled,
    /// Boolean
    HuntedRecently,
    /// core.BlockPos
    CelebrateLocation,
    /// Boolean
    Dancing,
    /// monster.hoglin.Hoglin
    NearestVisibleHuntableHoglin,
    /// monster.hoglin.Hoglin
    NearestVisibleBabyHoglin,
    /// player.Player
    NearestTargetablePlayerNotWearingGold,
    /// List<monster.piglin.AbstractPiglin>
    NearbyAdultPiglins,
    /// List<monster.piglin.AbstractPiglin>
    NearestVisibleAdultPiglins,
    /// List<monster.hoglin.Hoglin>
    NearestVisibleAdultHoglins,
    /// monster.piglin.AbstractPiglin
    NearestVisibleAdultPiglin,
    /// LivingEntity
    NearestVisibleZombified,
    /// Integer
    VisibleAdultPiglinCount,
    /// Integer
    VisibleAdultHoglinCount,
    /// player.Player
    NearestPlayerHoldingWantedItem,
    /// Boolean
    AteRecently,
    /// core.BlockPos
    NearestRepellent,
    /// Boolean
    Pacified,
    /// LivingEntity
    RoarTarget,
    /// core.BlockPos
    DisturbanceLocation,
    /// util.Unit
    RecentProjectile,
    /// util.Unit
    IsSniffing,
    /// util.Unit
    IsEmerging,
    /// util.Unit
    RoarSoundDelay,
    /// util.Unit
    DigCooldown,
    /// util.Unit
    RoarSoundCooldown,
    /// util.Unit
    SniffCooldown,
    /// util.Unit
    TouchCooldown,
    /// util.Unit
    VibrationCooldown,
    /// util.Unit
    SonicBoomCooldown,
    /// util.Unit
    SonicBoomSoundCooldown,
    /// util.Unit
    SonicBoomSoundDelay,
    /// UUID
    LikedPlayer,
    /// core.GlobalPos
    LikedNoteblockPosition,
    /// Integer
    LikedNoteblockCooldownTicks,
    /// Integer
    ItemPickupCooldownTicks,
    /// List<core.GlobalPos>
    SnifferExploredPositions,
    /// core.BlockPos
    SnifferSniffingTarget,
    /// Boolean
    SnifferDigging,
    /// Boolean
    SnifferHappy,
    /// util.Unit
    BreezeJumpCooldown,
    /// util.Unit
    BreezeShoot,
    /// util.Unit
    BreezeShootCharging,
    /// util.Unit
    BreezeShootRecovering,
    /// util.Unit
    BreezeShootCooldown,
    /// util.Unit
    BreezeJumpInhaling,
    /// core.BlockPos
    BreezeJumpTarget,
    /// util.Unit
    BreezeLeavingWater,
}

impl Mem {
    pub const COUNT: usize = 114;
    pub const ALL: [Mem; 114] = [
        Mem::Home,
        Mem::JobSite,
        Mem::PotentialJobSite,
        Mem::MeetingPoint,
        Mem::SecondaryJobSite,
        Mem::NearestLivingEntities,
        Mem::NearestVisibleLivingEntities,
        Mem::VisibleVillagerBabies,
        Mem::NearestPlayers,
        Mem::NearestVisiblePlayer,
        Mem::NearestVisibleAttackablePlayer,
        Mem::NearestVisibleAttackablePlayers,
        Mem::WalkTarget,
        Mem::LookTarget,
        Mem::AttackTarget,
        Mem::AttackCoolingDown,
        Mem::InteractionTarget,
        Mem::BreedTarget,
        Mem::RideTarget,
        Mem::Path,
        Mem::DoorsToClose,
        Mem::NearestBed,
        Mem::HurtBy,
        Mem::HurtByEntity,
        Mem::AvoidTarget,
        Mem::NearestHostile,
        Mem::NearestAttackable,
        Mem::HidingPlace,
        Mem::HeardBellTime,
        Mem::CantReachWalkTargetSince,
        Mem::GolemDetectedRecently,
        Mem::DangerDetectedRecently,
        Mem::LastSlept,
        Mem::LastWoken,
        Mem::LastWorkedAtPoi,
        Mem::NearestVisibleAdult,
        Mem::NearestVisibleWantedItem,
        Mem::NearestVisibleNemesis,
        Mem::PlayDeadTicks,
        Mem::TemptingPlayer,
        Mem::TemptationCooldownTicks,
        Mem::GazeCooldownTicks,
        Mem::LongJumpCooldownTicks,
        Mem::LongJumpMidJump,
        Mem::HasHuntingCooldown,
        Mem::RamCooldownTicks,
        Mem::RamTarget,
        Mem::IsInWater,
        Mem::IsPregnant,
        Mem::IsPanicking,
        Mem::UnreachableTongueTargets,
        Mem::VisitedBlockPositions,
        Mem::UnreachableTransportBlockPositions,
        Mem::TransportItemsCooldownTicks,
        Mem::ChargeCooldownTicks,
        Mem::AttackTargetCooldown,
        Mem::SpearFleeingTime,
        Mem::SpearFleeingPosition,
        Mem::SpearChargePosition,
        Mem::SpearEngageTime,
        Mem::SpearStatus,
        Mem::AngryAt,
        Mem::UniversalAnger,
        Mem::AdmiringItem,
        Mem::TimeTryingToReachAdmireItem,
        Mem::DisableWalkToAdmireItem,
        Mem::AdmiringDisabled,
        Mem::HuntedRecently,
        Mem::CelebrateLocation,
        Mem::Dancing,
        Mem::NearestVisibleHuntableHoglin,
        Mem::NearestVisibleBabyHoglin,
        Mem::NearestTargetablePlayerNotWearingGold,
        Mem::NearbyAdultPiglins,
        Mem::NearestVisibleAdultPiglins,
        Mem::NearestVisibleAdultHoglins,
        Mem::NearestVisibleAdultPiglin,
        Mem::NearestVisibleZombified,
        Mem::VisibleAdultPiglinCount,
        Mem::VisibleAdultHoglinCount,
        Mem::NearestPlayerHoldingWantedItem,
        Mem::AteRecently,
        Mem::NearestRepellent,
        Mem::Pacified,
        Mem::RoarTarget,
        Mem::DisturbanceLocation,
        Mem::RecentProjectile,
        Mem::IsSniffing,
        Mem::IsEmerging,
        Mem::RoarSoundDelay,
        Mem::DigCooldown,
        Mem::RoarSoundCooldown,
        Mem::SniffCooldown,
        Mem::TouchCooldown,
        Mem::VibrationCooldown,
        Mem::SonicBoomCooldown,
        Mem::SonicBoomSoundCooldown,
        Mem::SonicBoomSoundDelay,
        Mem::LikedPlayer,
        Mem::LikedNoteblockPosition,
        Mem::LikedNoteblockCooldownTicks,
        Mem::ItemPickupCooldownTicks,
        Mem::SnifferExploredPositions,
        Mem::SnifferSniffingTarget,
        Mem::SnifferDigging,
        Mem::SnifferHappy,
        Mem::BreezeJumpCooldown,
        Mem::BreezeShoot,
        Mem::BreezeShootCharging,
        Mem::BreezeShootRecovering,
        Mem::BreezeShootCooldown,
        Mem::BreezeJumpInhaling,
        Mem::BreezeJumpTarget,
        Mem::BreezeLeavingWater,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mem::Home => "minecraft:home",
            Mem::JobSite => "minecraft:job_site",
            Mem::PotentialJobSite => "minecraft:potential_job_site",
            Mem::MeetingPoint => "minecraft:meeting_point",
            Mem::SecondaryJobSite => "minecraft:secondary_job_site",
            Mem::NearestLivingEntities => "minecraft:mobs",
            Mem::NearestVisibleLivingEntities => "minecraft:visible_mobs",
            Mem::VisibleVillagerBabies => "minecraft:visible_villager_babies",
            Mem::NearestPlayers => "minecraft:nearest_players",
            Mem::NearestVisiblePlayer => "minecraft:nearest_visible_player",
            Mem::NearestVisibleAttackablePlayer => "minecraft:nearest_visible_targetable_player",
            Mem::NearestVisibleAttackablePlayers => "minecraft:nearest_visible_targetable_players",
            Mem::WalkTarget => "minecraft:walk_target",
            Mem::LookTarget => "minecraft:look_target",
            Mem::AttackTarget => "minecraft:attack_target",
            Mem::AttackCoolingDown => "minecraft:attack_cooling_down",
            Mem::InteractionTarget => "minecraft:interaction_target",
            Mem::BreedTarget => "minecraft:breed_target",
            Mem::RideTarget => "minecraft:ride_target",
            Mem::Path => "minecraft:path",
            Mem::DoorsToClose => "minecraft:doors_to_close",
            Mem::NearestBed => "minecraft:nearest_bed",
            Mem::HurtBy => "minecraft:hurt_by",
            Mem::HurtByEntity => "minecraft:hurt_by_entity",
            Mem::AvoidTarget => "minecraft:avoid_target",
            Mem::NearestHostile => "minecraft:nearest_hostile",
            Mem::NearestAttackable => "minecraft:nearest_attackable",
            Mem::HidingPlace => "minecraft:hiding_place",
            Mem::HeardBellTime => "minecraft:heard_bell_time",
            Mem::CantReachWalkTargetSince => "minecraft:cant_reach_walk_target_since",
            Mem::GolemDetectedRecently => "minecraft:golem_detected_recently",
            Mem::DangerDetectedRecently => "minecraft:danger_detected_recently",
            Mem::LastSlept => "minecraft:last_slept",
            Mem::LastWoken => "minecraft:last_woken",
            Mem::LastWorkedAtPoi => "minecraft:last_worked_at_poi",
            Mem::NearestVisibleAdult => "minecraft:nearest_visible_adult",
            Mem::NearestVisibleWantedItem => "minecraft:nearest_visible_wanted_item",
            Mem::NearestVisibleNemesis => "minecraft:nearest_visible_nemesis",
            Mem::PlayDeadTicks => "minecraft:play_dead_ticks",
            Mem::TemptingPlayer => "minecraft:tempting_player",
            Mem::TemptationCooldownTicks => "minecraft:temptation_cooldown_ticks",
            Mem::GazeCooldownTicks => "minecraft:gaze_cooldown_ticks",
            Mem::LongJumpCooldownTicks => "minecraft:long_jump_cooling_down",
            Mem::LongJumpMidJump => "minecraft:long_jump_mid_jump",
            Mem::HasHuntingCooldown => "minecraft:has_hunting_cooldown",
            Mem::RamCooldownTicks => "minecraft:ram_cooldown_ticks",
            Mem::RamTarget => "minecraft:ram_target",
            Mem::IsInWater => "minecraft:is_in_water",
            Mem::IsPregnant => "minecraft:is_pregnant",
            Mem::IsPanicking => "minecraft:is_panicking",
            Mem::UnreachableTongueTargets => "minecraft:unreachable_tongue_targets",
            Mem::VisitedBlockPositions => "minecraft:visited_block_positions",
            Mem::UnreachableTransportBlockPositions => "minecraft:unreachable_transport_block_positions",
            Mem::TransportItemsCooldownTicks => "minecraft:transport_items_cooldown_ticks",
            Mem::ChargeCooldownTicks => "minecraft:charge_cooldown_ticks",
            Mem::AttackTargetCooldown => "minecraft:attack_target_cooldown",
            Mem::SpearFleeingTime => "minecraft:spear_fleeing_time",
            Mem::SpearFleeingPosition => "minecraft:spear_fleeing_position",
            Mem::SpearChargePosition => "minecraft:spear_charge_position",
            Mem::SpearEngageTime => "minecraft:spear_engage_time",
            Mem::SpearStatus => "minecraft:spear_status",
            Mem::AngryAt => "minecraft:angry_at",
            Mem::UniversalAnger => "minecraft:universal_anger",
            Mem::AdmiringItem => "minecraft:admiring_item",
            Mem::TimeTryingToReachAdmireItem => "minecraft:time_trying_to_reach_admire_item",
            Mem::DisableWalkToAdmireItem => "minecraft:disable_walk_to_admire_item",
            Mem::AdmiringDisabled => "minecraft:admiring_disabled",
            Mem::HuntedRecently => "minecraft:hunted_recently",
            Mem::CelebrateLocation => "minecraft:celebrate_location",
            Mem::Dancing => "minecraft:dancing",
            Mem::NearestVisibleHuntableHoglin => "minecraft:nearest_visible_huntable_hoglin",
            Mem::NearestVisibleBabyHoglin => "minecraft:nearest_visible_baby_hoglin",
            Mem::NearestTargetablePlayerNotWearingGold => "minecraft:nearest_targetable_player_not_wearing_gold",
            Mem::NearbyAdultPiglins => "minecraft:nearby_adult_piglins",
            Mem::NearestVisibleAdultPiglins => "minecraft:nearest_visible_adult_piglins",
            Mem::NearestVisibleAdultHoglins => "minecraft:nearest_visible_adult_hoglins",
            Mem::NearestVisibleAdultPiglin => "minecraft:nearest_visible_adult_piglin",
            Mem::NearestVisibleZombified => "minecraft:nearest_visible_zombified",
            Mem::VisibleAdultPiglinCount => "minecraft:visible_adult_piglin_count",
            Mem::VisibleAdultHoglinCount => "minecraft:visible_adult_hoglin_count",
            Mem::NearestPlayerHoldingWantedItem => "minecraft:nearest_player_holding_wanted_item",
            Mem::AteRecently => "minecraft:ate_recently",
            Mem::NearestRepellent => "minecraft:nearest_repellent",
            Mem::Pacified => "minecraft:pacified",
            Mem::RoarTarget => "minecraft:roar_target",
            Mem::DisturbanceLocation => "minecraft:disturbance_location",
            Mem::RecentProjectile => "minecraft:recent_projectile",
            Mem::IsSniffing => "minecraft:is_sniffing",
            Mem::IsEmerging => "minecraft:is_emerging",
            Mem::RoarSoundDelay => "minecraft:roar_sound_delay",
            Mem::DigCooldown => "minecraft:dig_cooldown",
            Mem::RoarSoundCooldown => "minecraft:roar_sound_cooldown",
            Mem::SniffCooldown => "minecraft:sniff_cooldown",
            Mem::TouchCooldown => "minecraft:touch_cooldown",
            Mem::VibrationCooldown => "minecraft:vibration_cooldown",
            Mem::SonicBoomCooldown => "minecraft:sonic_boom_cooldown",
            Mem::SonicBoomSoundCooldown => "minecraft:sonic_boom_sound_cooldown",
            Mem::SonicBoomSoundDelay => "minecraft:sonic_boom_sound_delay",
            Mem::LikedPlayer => "minecraft:liked_player",
            Mem::LikedNoteblockPosition => "minecraft:liked_noteblock",
            Mem::LikedNoteblockCooldownTicks => "minecraft:liked_noteblock_cooldown_ticks",
            Mem::ItemPickupCooldownTicks => "minecraft:item_pickup_cooldown_ticks",
            Mem::SnifferExploredPositions => "minecraft:sniffer_explored_positions",
            Mem::SnifferSniffingTarget => "minecraft:sniffer_sniffing_target",
            Mem::SnifferDigging => "minecraft:sniffer_digging",
            Mem::SnifferHappy => "minecraft:sniffer_happy",
            Mem::BreezeJumpCooldown => "minecraft:breeze_jump_cooldown",
            Mem::BreezeShoot => "minecraft:breeze_shoot",
            Mem::BreezeShootCharging => "minecraft:breeze_shoot_charging",
            Mem::BreezeShootRecovering => "minecraft:breeze_shoot_recover",
            Mem::BreezeShootCooldown => "minecraft:breeze_shoot_cooldown",
            Mem::BreezeJumpInhaling => "minecraft:breeze_jump_inhaling",
            Mem::BreezeJumpTarget => "minecraft:breeze_jump_target",
            Mem::BreezeLeavingWater => "minecraft:breeze_leaving_water",
        }
    }

    /// Whether the memory has a codec (`canSerialize`): only those are saved.
    pub fn serializable(self) -> bool {
        matches!(
            self,
            Mem::Home | Mem::JobSite | Mem::PotentialJobSite | Mem::MeetingPoint | Mem::GolemDetectedRecently | Mem::DangerDetectedRecently | Mem::LastSlept | Mem::LastWoken | Mem::LastWorkedAtPoi | Mem::PlayDeadTicks | Mem::TemptationCooldownTicks | Mem::GazeCooldownTicks | Mem::LongJumpCooldownTicks | Mem::HasHuntingCooldown | Mem::RamCooldownTicks | Mem::IsInWater | Mem::IsPregnant | Mem::IsPanicking | Mem::VisitedBlockPositions | Mem::UnreachableTransportBlockPositions | Mem::ChargeCooldownTicks | Mem::AttackTargetCooldown | Mem::AngryAt | Mem::UniversalAnger | Mem::AdmiringItem | Mem::AdmiringDisabled | Mem::HuntedRecently | Mem::RecentProjectile | Mem::IsSniffing | Mem::IsEmerging | Mem::RoarSoundDelay | Mem::DigCooldown | Mem::RoarSoundCooldown | Mem::SniffCooldown | Mem::TouchCooldown | Mem::VibrationCooldown | Mem::SonicBoomCooldown | Mem::SonicBoomSoundCooldown | Mem::SonicBoomSoundDelay | Mem::LikedPlayer | Mem::LikedNoteblockPosition | Mem::LikedNoteblockCooldownTicks | Mem::ItemPickupCooldownTicks | Mem::SnifferExploredPositions | Mem::BreezeJumpCooldown | Mem::BreezeShoot | Mem::BreezeShootCharging | Mem::BreezeShootRecovering | Mem::BreezeShootCooldown | Mem::BreezeJumpInhaling | Mem::BreezeJumpTarget | Mem::BreezeLeavingWater
        )
    }

    pub fn by_name(name: &str) -> Option<Mem> {
        Mem::ALL.iter().copied().find(|m| m.name() == name)
    }
}

/// `MemoryStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    ValuePresent,
    ValueAbsent,
    Registered,
}

/// `GlobalPos`: a block in a dimension (`minecraft:overworld` ...).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlobalPos {
    pub dim: std::sync::Arc<str>,
    pub pos: BlockPos,
}

impl GlobalPos {
    pub fn new(dim: &str, pos: BlockPos) -> GlobalPos {
        GlobalPos { dim: dim.into(), pos }
    }
}

/// `PositionTracker`: `BlockPosTracker` or `EntityTracker`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tracker {
    /// `BlockPosTracker(pos)` (centre of the block) or `(Vec3)`.
    Block { pos: BlockPos, center: Vec3 },
    /// `EntityTracker(entity, trackEyeHeight, targetEyeHeight)`.
    Entity { id: i32, track_eye: bool, target_eye: bool },
}

impl Tracker {
    pub fn block(pos: BlockPos) -> Tracker {
        Tracker::Block { pos, center: pos.center() }
    }

    pub fn vec(v: Vec3) -> Tracker {
        Tracker::Block { pos: BlockPos::containing(v.x, v.y, v.z), center: v }
    }

    /// `new EntityTracker(entity, trackEyeHeight)`.
    pub fn entity(id: i32, track_eye: bool) -> Tracker {
        Tracker::Entity { id, track_eye, target_eye: false }
    }

    /// `new EntityTracker(entity, trackEyeHeight, targetEyeHeight)`.
    pub fn entity3(id: i32, track_eye: bool, target_eye: bool) -> Tracker {
        Tracker::Entity { id, track_eye, target_eye }
    }
}

/// `WalkTarget`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkTarget {
    pub target: Tracker,
    pub speed: f32,
    pub close_enough: i32,
}

impl WalkTarget {
    pub fn block(pos: BlockPos, speed: f32, close_enough: i32) -> WalkTarget {
        WalkTarget { target: Tracker::block(pos), speed, close_enough }
    }

    /// `new WalkTarget(Vec3, speed, closeEnough)`: the tracker is the block containing `v`
    /// (`new BlockPosTracker(BlockPos.containing(v))`, its position the block's center).
    pub fn vec(v: Vec3, speed: f32, close_enough: i32) -> WalkTarget {
        WalkTarget { target: Tracker::block(BlockPos::containing(v.x, v.y, v.z)), speed, close_enough }
    }

    pub fn entity(id: i32, speed: f32, close_enough: i32) -> WalkTarget {
        WalkTarget { target: Tracker::entity(id, true), speed, close_enough }
    }
}

/// `NearestVisibleLivingEntities`: the nearby entities (nearest first) and, once asked, whether
/// each is targetable by the owner (`Sensor.isEntityTargetable`), remembered until the next scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NearestVisible {
    pub nearby: Vec<i32>,
    /// (id, visible), filled on demand.
    pub seen: Vec<(i32, bool)>,
}

impl NearestVisible {
    pub fn new(nearby: Vec<i32>) -> NearestVisible {
        NearestVisible { nearby, seen: Vec::new() }
    }
}

/// A memory's value (the Java type differs per `MemoryModuleType`; see the doc of each [`Mem`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    /// `Unit`.
    Unit,
    Bool(bool),
    Int(i32),
    Long(i64),
    /// An entity or a player, by id.
    Entity(i32),
    Entities(Vec<i32>),
    Pos(GlobalPos),
    Positions(Vec<GlobalPos>),
    Block(BlockPos),
    Vec3(Vec3),
    Walk(WalkTarget),
    Look(Tracker),
    Uuid(u128),
    Uuids(Vec<u128>),
    Damage(DamageSource),
    Visible(NearestVisible),
    /// A value nothing reads (`PATH`: only its presence matters).
    Marker,
}

/// `MemorySlot`: a value that expires `ttl` brain ticks after it was set (`i64::MAX`: never).
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub value: Option<Val>,
    pub ttl: i64,
}

pub const NEVER_EXPIRE: i64 = i64::MAX;

impl Slot {
    pub const EMPTY: Slot = Slot { value: None, ttl: NEVER_EXPIRE };

    /// `MemorySlot.tick`.
    pub fn tick(&mut self) {
        if self.value.is_some() && self.ttl != NEVER_EXPIRE {
            if self.ttl <= 0 {
                self.clear();
            } else {
                self.ttl -= 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.value = None;
        self.ttl = NEVER_EXPIRE;
    }
}

/// `Brain.isEmptyCollection`: setting an empty list or set erases the memory.
fn is_empty_collection(v: &Val) -> bool {
    match v {
        Val::Entities(l) => l.is_empty(),
        Val::Positions(l) => l.is_empty(),
        Val::Uuids(l) => l.is_empty(),
        _ => false,
    }
}

/// The memories of a brain: every registered `MemoryModuleType` has a slot.
#[derive(Clone, Debug)]
pub struct Memories {
    slots: Vec<Slot>,
    registered: [bool; Mem::COUNT],
}

impl Default for Memories {
    fn default() -> Self {
        Memories { slots: vec![Slot::EMPTY; Mem::COUNT], registered: [false; Mem::COUNT] }
    }
}

impl Memories {
    pub fn register(&mut self, m: Mem) {
        self.registered[m as usize] = true;
    }

    pub fn is_registered(&self, m: Mem) -> bool {
        self.registered[m as usize]
    }

    /// `Brain.forgetOutdatedMemories`.
    pub fn tick(&mut self) {
        for (i, s) in self.slots.iter_mut().enumerate() {
            if self.registered[i] {
                s.tick();
            }
        }
    }

    /// `Brain.checkMemory`.
    pub fn check(&self, m: Mem, status: Status) -> bool {
        let i = m as usize;
        if !self.registered[i] {
            return false;
        }
        match status {
            Status::Registered => true,
            Status::ValuePresent => self.slots[i].value.is_some(),
            Status::ValueAbsent => self.slots[i].value.is_none(),
        }
    }

    pub fn has(&self, m: Mem) -> bool {
        self.check(m, Status::ValuePresent)
    }

    /// `Brain.getMemory` (a memory that was never registered has no value).
    pub fn get(&self, m: Mem) -> Option<&Val> {
        if !self.registered[m as usize] { None } else { self.slots[m as usize].value.as_ref() }
    }

    pub fn get_mut(&mut self, m: Mem) -> Option<&mut Val> {
        if !self.registered[m as usize] { None } else { self.slots[m as usize].value.as_mut() }
    }

    /// `Brain.getTimeUntilExpiry`.
    pub fn time_until_expiry(&self, m: Mem) -> i64 {
        self.slots[m as usize].ttl
    }

    /// `Brain.setMemory(type, value)`: unregistered types are ignored; an empty collection erases.
    pub fn set(&mut self, m: Mem, v: Val) {
        self.set_with(m, v, NEVER_EXPIRE);
    }

    /// `Brain.setMemoryWithExpiry`.
    pub fn set_expiring(&mut self, m: Mem, v: Val, ttl: i64) {
        self.set_with(m, v, ttl);
    }

    fn set_with(&mut self, m: Mem, v: Val, ttl: i64) {
        let i = m as usize;
        if !self.registered[i] {
            return;
        }
        if is_empty_collection(&v) {
            self.slots[i].clear();
        } else {
            self.slots[i] = Slot { value: Some(v), ttl };
        }
    }

    /// `Brain.setMemory(type, Optional)`: `None` erases.
    pub fn set_opt(&mut self, m: Mem, v: Option<Val>) {
        match v {
            Some(v) => self.set(m, v),
            None => self.erase(m),
        }
    }

    /// `Brain.eraseMemory`.
    pub fn erase(&mut self, m: Mem) {
        if self.registered[m as usize] {
            self.slots[m as usize].clear();
        }
    }

    /// `Brain.clearMemories`.
    pub fn clear_all(&mut self) {
        for s in self.slots.iter_mut() {
            s.clear();
        }
    }

    pub fn slot(&self, m: Mem) -> &Slot {
        &self.slots[m as usize]
    }

    /// `Brain.isMemoryValue`.
    pub fn is(&self, m: Mem, v: &Val) -> bool {
        self.get(m) == Some(v)
    }

    // ---- typed reads (None when absent or of another shape)

    pub fn entity(&self, m: Mem) -> Option<i32> {
        match self.get(m)? {
            Val::Entity(id) => Some(*id),
            _ => None,
        }
    }

    pub fn entities(&self, m: Mem) -> &[i32] {
        match self.get(m) {
            Some(Val::Entities(l)) => l,
            _ => &[],
        }
    }

    pub fn boolean(&self, m: Mem) -> Option<bool> {
        match self.get(m)? {
            Val::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn int(&self, m: Mem) -> Option<i32> {
        match self.get(m)? {
            Val::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn long(&self, m: Mem) -> Option<i64> {
        match self.get(m)? {
            Val::Long(i) => Some(*i),
            _ => None,
        }
    }

    pub fn block(&self, m: Mem) -> Option<BlockPos> {
        match self.get(m)? {
            Val::Block(p) => Some(*p),
            Val::Pos(g) => Some(g.pos),
            _ => None,
        }
    }

    pub fn global_pos(&self, m: Mem) -> Option<&GlobalPos> {
        match self.get(m)? {
            Val::Pos(g) => Some(g),
            _ => None,
        }
    }

    pub fn vec3(&self, m: Mem) -> Option<Vec3> {
        match self.get(m)? {
            Val::Vec3(v) => Some(*v),
            _ => None,
        }
    }

    pub fn walk_target(&self) -> Option<WalkTarget> {
        match self.get(Mem::WalkTarget)? {
            Val::Walk(w) => Some(*w),
            _ => None,
        }
    }

    pub fn look_target(&self) -> Option<Tracker> {
        match self.get(Mem::LookTarget)? {
            Val::Look(t) => Some(*t),
            _ => None,
        }
    }

    pub fn uuid(&self, m: Mem) -> Option<u128> {
        match self.get(m)? {
            Val::Uuid(u) => Some(*u),
            _ => None,
        }
    }

    pub fn positions(&self, m: Mem) -> &[GlobalPos] {
        match self.get(m) {
            Some(Val::Positions(l)) => l,
            _ => &[],
        }
    }

    pub fn damage(&self, m: Mem) -> Option<DamageSource> {
        match self.get(m)? {
            Val::Damage(d) => Some(*d),
            _ => None,
        }
    }

    pub fn visible(&self) -> Option<&NearestVisible> {
        match self.get(Mem::NearestVisibleLivingEntities)? {
            Val::Visible(v) => Some(v),
            _ => None,
        }
    }

    /// Every memory with a value, as (type, value, ttl) (for persistence and debugging).
    pub fn iter(&self) -> impl Iterator<Item = (Mem, &Val, i64)> {
        Mem::ALL.iter().filter_map(move |&m| {
            let s = &self.slots[m as usize];
            s.value.as_ref().map(|v| (m, v, s.ttl))
        })
    }
}
