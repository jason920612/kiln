# wp83 – enchantments run from the data pack's definitions

Part of wp83 (a server running data packs behaves like vanilla 26.3 running the same data packs): the enchantments.
Vanilla runs whatever effect components the definition of an enchantment (`data/<ns>/enchantment/*.json`) has and reads
the enchantment **tags** for the few rules that were enchantment names before (`#prevents_ice_melting`, ...). Kiln knew a
handful of vanilla enchantments by id (soul speed, frost walker, channeling, flame, silk touch, the curses, fire aspect on a spear,
riptide and loyalty on a trident). Now the definitions decide.

Test packs: `tools/wp83/packs/enchant/*` (written for the tests, not vanilla's files). Tests:
`crates/kiln-sim/tests/enchant_packs.rs` (a world with the packs in `datapacks/`; they hold `kiln_data::live::exclusive()`),
unit tests in `kiln-loot` (`fx.rs`, `tests/vanilla_load.rs`) and `kiln-item` (`enchant_flags.rs`).
`tools/wp83/neg83_enchant.sh` runs each test without its change and requires it to fail.

## The registry is loaded with the world

`RegistryDataLoader` loads `enchantment`, `enchantment_provider`, `jukebox_song`, `trade_set` and `villager_trade` once, when the
world loads; `/reload` reloads only loot tables, predicates, item modifiers, slot sources, number providers, advancements and recipes
(`RegistryDataLoader.RELOADABLE_REGISTRIES`) and the **tags**. Kiln's `LootData` re-read the enchantment files on every `/reload`.
Now `kiln_loot::data::WorldDefs` holds the text of those five directories read at the first load
(`LootData.world`, kept in `Packs.world_defs`); a reload decodes them again from there, against the reloaded tags (a named
`HolderSet` such as `supported_items: "#minecraft:enchantable/sword"` follows the tag in vanilla too: `Registry.bindTags`).
Test: `reload_rereads_the_tags_but_keeps_the_enchantments_the_world_loaded`.

## Effect components (A)

`kiln_loot::effects::Effects` decodes every component of `minecraft:enchantment_effect_component_type`; `Effects.other` stays empty for
all of them (the loader test `enchantment_effects_decode` checks vanilla's). What evaluates what:

| component | evaluated by | notes |
|---|---|---|
| `damage_protection`, `damage_immunity`, `damage`, `smash_damage_per_fallen_block`, `knockback`, `armor_effectiveness`, `item_damage`, `ammo_use`, `projectile_piercing/_spread/_count`, `trident_return_acceleration`, `fishing_*`, `block_experience`, `mob_experience`, `repair_with_xp`, `attributes`, `post_attack`, `post_piercing_attack`, `equipment_drops` (decoded) | as before (`EnchantmentHelper` in `kiln_loot::effects`) | |
| `crossbow_charge_time`, `trident_spin_attack_strength` | `LootData::crossbow_charge_time` / `trident_spin_attack_strength`: **every** enchantment's effect applied in turn (`modifyUnfilteredValue`), not the one of the highest level | fixes two enchantments with the component |
| `crossbow_charging_sounds`, `trident_sound` | `LootData::pick_highest_level` (`EnchantmentHelper.pickHighestLevel`: the enchantment with the highest level, entry `min(level, size) - 1`), with the default loading / `item.trident.throw` sounds | `crossbow.rs`, `trident.rs` |
| `location_changed` | `kiln-sim/src/enchant_fx.rs` (players) | below |
| `tick` | `enchant_fx.rs` (players), first thing of `base_tick` | |
| `hit_block` | `kiln-entity/src/enchanting.rs` `run_projectile_effects` (thrown tridents and arrows that remember their weapon, at the hit location clamped into the block, `blockHitContext`) | lightning / sounds / ignite / impulse; other effects: see below |
| `projectile_spawned` | `enchant_fx::projectile_spawned` (players' shots: the ammunition's enchantments, then the weapon's when it is another item); `run_projectile_effects` for skeletons' and pillagers' arrows (they now carry the bow) | |
| `prevent_equipment_drop`, `prevent_armor_change` | `kiln_item::enchant_flags` (the sim installs the ids of the enchantments that have them when the world loads; vanilla's two curses until then) | players' death (`destroyVanishingCursedItems`), armor slots / `Equippable` swap, mobs' death drops, shearing, mobs picking up armor |
| `post_piercing_attack` | `spear.rs` (lunge kinds as before); any other effect kind through `kiln_loot::fx` | |

### `location_changed` and `tick` (`enchant_fx.rs`)

Vanilla (`LivingEntity.baseTick` → `onChangedBlock` → `EnchantmentHelper.runLocationChangedEffects`; `checkFallDamage`; `collectEquipmentChanges`;
`onEquippedItemBroken`; `Enchantment.runLocationChangedEffects`):

* it runs when the wearer's **block position** changes, when it **lands**, and when a piece of equipment **changes** (put on, taken off,
  durability changed, broken): the old item's effects stop first (`stopLocationBasedEffects`), then the new item's start;
* per enchantment on a slot it applies to (`matchingSlot`), per `location_changed` entry in order: the requirements are tested in
  `locationContext` (`this`, level, origin, **`enchantment_active`** = whether this entry was active before). If they hold the entry becomes
  active; an `attribute` effect adds its transient modifier (`<id>/<slot>`) **only when it becomes active**, every other effect runs each time;
  if they fail and the entry was active it is deactivated (the modifier goes);
* `ActiveEffect { slot, enchantment, index }` and `LocationModifier` on the `Player` replace `soul_speed: Option<i32>`; the modifiers are in
  `attribute_modifiers_with`. `tick` runs the same way with `entityContext`.
* soul speed and frost walker are now just their definitions: the `movement_affected_by` / `stepping_on` / `location` / `movement`
  predicates (`EntityView.matches_with`, new fields `movement`, `movement_block`, `on_pos`), `enchantment_active_check`, `replace_disk` with its
  block predicate (`matching_blocks`, `matching_block_tag`, `matching_fluids`, `solid`, `replaceable`, `unobstructed`, `all_of`, `any_of`, `not`, `true`),
  `spawn_particles` (positions / velocities drawn in `sendParticles`' argument order), `play_sound`, `change_item_damage`.

### Entity effects (`kiln_loot::fx`)

`fx::resolve` turns an `EnchantmentEntityEffect` into `Action`s (all numbers decided: level-based values, the entity's random drawn in vanilla's order,
`apply_impulse` rotated by the entity's look quaternion). Who does an action: `Player::apply_fx` (players: ignite, mob effect, damage, item damage,
exhaustion, impulse, sound, particles; the rest queued as `LevelJob`s for the region's block phase: `replace_block`, `replace_disk`,
`set_block_properties`, `summon_entity`, `run_function` (run by `Sim::run_enchantment_functions` as the player, at the effect position, silent, level 2));
`kiln-entity` for projectiles (sounds, lightning, ignite, impulse).

## Where Kiln hard-coded a vanilla enchantment (B)

| was | now |
|---|---|
| `enchant_loc.rs`: soul speed (`0.0405 + 0.0105 * (level - 1)`, the 4 % wear, the particles) and frost walker (radius, frosted ice) by id | the definitions (`location_changed` / `tick`) |
| `trident.rs`: `riptide` level → sound; `channeling` flag | `trident_sound`, `hit_block` / `post_attack` effects |
| `ext_entity/trident.rs`: loyalty / channeling by id; `call_lightning` | `trident_return_acceleration`, `hit_block`, `post_attack` through `kiln_loot` |
| `ranged.rs`: any `projectile_spawned` meant "100 s of fire" | the effects of the definitions |
| `crossbow.rs`: `quick_charge_<level>` sound names | `crossbow_charging_sounds` |
| `spear.rs`: `fire_aspect` × 4 s on a mob | the weapon's `post_attack` effects (`Work::post_attack_entity`) |
| `beehive.rs`, `blocks.rs` (infested), `decorated_pot.rs`: `minecraft:silk_touch` | `#prevents_bee_spawns_when_mining`, `#prevents_infested_spawns`, `#prevents_decorated_pot_shattering` (`ItemStack::has_enchantment_tag`) |
| (not there at all) breaking ice | `IceBlock.playerDestroy` with `#prevents_ice_melting` (`digging.rs`) |
| `kiln-inventory/rules.rs`: curses by id / the files of `data/minecraft/enchantment` | `kiln_item::enchant_flags` from the loaded definitions (also pack enchantments) |
| `mob/interact.rs`, `mob/pickup.rs`: `binding_curse` | the marker |
| mob death drops ignored `prevent_equipment_drop` | honoured |

## Still hard-coded / not done, and why

* **Soul speed / frost walker on mobs**: only players run `tick` / `location_changed` (Kiln's mobs keep no active-effect state and `mob/mod.rs` applies no
  enchantment attribute effects to them; vanilla does for every `LivingEntity`).
* **`explode`** is only carried out in `post_attack` (the mace's wind burst, `melee.rs`); as a `tick`, `location_changed`, `hit_block` or `projectile_spawned`
  effect it does nothing. **`run_function`**, **`replace_*`**, `set_block_properties` and `summon_entity` of types other than `lightning_bolt` run for
  players' effects but not for a projectile's `hit_block` (needs the server's level from `kiln-entity`); mobs' melee `post_attack`
  (`MobPostAttack`) knows ignite and mob effects only.
* **`equipment_drops`** (looting for equipment): decoded, not applied to a mob's equipment drop chance (`mob/mod.rs` uses the plain chance).
* `prevent_equipment_drop` for an armor stand's, a horse's and an allay's items, and `prevent_armor_change` for zombie villager conversion, dispensers and piglins'
  `canReplaceCurrentItem` (mobs pick up with `compareArmor` ✓): not checked.
* The requirement context of `hit_block` / `post_attack` in `kiln-entity` has no dimension (`location` predicates with a `dimension` fail there); biomes, light, sky and
  blocks work (`LevelProbe`).
* Particle options (`dust` colour ...) of `spawn_particles` are not carried (the type only); `replace_*` take `simple_state_provider` / a bare state only.
* The random for requirements of projectile effects is derived from the projectile id and game time (not the level's random stream).
* `moved_too_quickly`-style teleport staleness: a teleported player's supporting block is the old one until its next move, as in vanilla.

## Custom enchantments everywhere vanilla's work (C)

Checked by reading the code and by the tests with `kz:*` enchantments (`tools/wp83/packs/enchant/kz_*`): `/enchant` (argument from the live
registry, `max_level`, `exclusive_set`, supported items), enchanting table (`#in_enchanting_table`, costs, weights, exclusive sets via
`kiln_loot::enchant::select`), anvil (`enchantment_can_enchant`, `anvil_cost`, `max_level`, incompatible sets), grindstone (`#curse`), `enchant_randomly` /
`enchant_with_levels` (vanilla: no `options` means *every* registered enchantment; the vanilla tables say `#on_random_loot`), villager trades
(`#tradeable`, `#double_trade_price`, `trades/*`), providers (`on_mob_spawn_equipment`, `on_traded_equipment`, `on_random_loot`), `curse` / `treasure` /
`tooltip_order` (sent with the tags). All of them read `LootData` (definitions as loaded with the world) and the live tags (reloaded).
