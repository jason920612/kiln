#!/bin/bash
# wp83 (enchantments) negative checks (on the VM, in the wp83-enchant checkout): break the line a fix is made of (or put back what was
# there before it), run the fix's test, and require it to FAIL; then restore. usage: ~/kt wp83-enchant bash tools/wp83/neg83_enchant.sh
# (needs KILN_DATAPACK=/home/test/kiln/work/generated for the tests that load vanilla's data.)
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
fails=0
# negsed "<name>" "<test args>" file 'sed expression': edit the file, run, require FAIL (a failing test, not a build error), restore
negsed() {
  local name="$1" tests="$2" f="$3" expr="$4"
  sed -i "$expr" "$f"
  if git diff --quiet -- "$f"; then echo "SED CHANGED NOTHING (bad): $name"; fails=$((fails + 1)); git checkout -- "$f"; return; fi
  if cargo test --release $tests > /tmp/neg83e.out 2>&1; then
    echo "NOT FAILING (bad): $name"; fails=$((fails + 1))
  elif grep -q "^error" /tmp/neg83e.out && ! grep -q "^test .* FAILED" /tmp/neg83e.out; then echo "BUILD ERROR (bad): $name"; fails=$((fails + 1))
  else echo "fails without the fix (good): $name"; fi
  git checkout -- "$f"
}
T="-p kiln-sim --test enchant_packs"
# location_changed / tick effects of a pack's enchantment
negsed "location_changed of a pack's enchantment" "$T a_packs_location_changed" crates/kiln-sim/src/enchant_fx.rs 's/if e.effects.location_changed.is_empty() || !e.matching_slot(slot) {/if true {/'
negsed "tick of a pack's enchantment" "$T a_packs_tick_effect" crates/kiln-sim/src/enchant_fx.rs 's/if e.effects.tick.is_empty() || !e.matching_slot(slot) {/if true {/'
# hit_block of an arrow with a pack's enchantment
negsed "hit_block of an arrow" "$T a_packs_hit_block" crates/kiln-entity/src/arrow.rs 's/if let Some(w) = data(e).weapon.clone().filter(|w| !w.is_empty()) {/if false \&\& let Some(w) = data(e).weapon.clone().filter(|w| !w.is_empty()) {/'
# projectile_spawned: any such effect used to mean 100 seconds of fire
negsed "projectile_spawned ignites for the effect's duration" "$T a_packs_projectile_spawned" crates/kiln-sim/src/ranged.rs 's|crate::enchant_fx::projectile_spawned(p, level, &mut e, &ammo, weapon, spawns);|e.ignite_for_seconds(100.0);|'
# soul speed redefined by a pack: the attribute effect of the definition
negsed "redefined soul speed" "$T redefining_soul_speed" crates/kiln-sim/src/enchant_fx.rs 's/                if apply_transient {/                if false \&\& apply_transient {/'
# a spear's stab burns by the weapon's post_attack effects
negsed "spear stab post_attack (fire aspect redefined)" "$T a_spears_stab_burns" crates/kiln-sim/src/spear.rs 's/                    blow.post_attack_entity(a, id, source);/                    let _ = (\&mut blow, id);/'
# ice: the melt, and the tag instead of silk touch by id
negsed "ice melts when broken" "$T ice_melts_unless" crates/kiln-sim/src/digging.rs 's/    if let (true, Some(tool)) = (removed, ice_tool.as_ref()) {/    if let (false, Some(tool)) = (removed, ice_tool.as_ref()) {/'
negsed "silk touch by id instead of #prevents_ice_melting" "$T ice_melts_unless" crates/kiln-sim/src/digging.rs 's|    if tool.has_enchantment_tag("minecraft:prevents_ice_melting") {|    if tool.get(kiln_item::keys::ENCHANTMENTS).is_some_and(\|e\| kiln_item::registry::ENCHANTMENT.id("minecraft:silk_touch").is_some_and(\|id\| e.level(id) > 0)) {|'
# the markers of a pack's enchantments (only vanilla's own were known)
negsed "vanishing marker of a pack's enchantment" "$T a_packs_enchantment_with_the_vanishing_marker" crates/kiln-item/src/enchant_flags.rs 's/    list.0.iter().any(|(e, _)| ids.contains(e))/    list.0.iter().any(|(e, _)| ids.contains(e) \&\& kiln_data::live::synced_name("minecraft:enchantment", *e).is_some_and(|n| n.starts_with("minecraft:")))/'
negsed "binding marker of a pack's enchantment" "$T a_packs_enchantment_with_the_binding_marker" crates/kiln-item/src/enchant_flags.rs 's/    list.0.iter().any(|(e, _)| ids.contains(e))/    list.0.iter().any(|(e, _)| ids.contains(e) \&\& kiln_data::live::synced_name("minecraft:enchantment", *e).is_some_and(|n| n.starts_with("minecraft:")))/'
# /reload keeps the registry entries of the world
negsed "reload keeps the world's enchantments" "$T reload_rereads_the_tags" crates/kiln-sim/src/datapacks.rs 's/let frozen = self.commands.packs.world_defs.clone();/let frozen: Option<std::sync::Arc<kiln_loot::data::WorldDefs>> = None;/'
# the crossbow: every enchantment's charge time, not the highest level's
negsed "crossbow charge time of every enchantment" "$T every_enchantment_with_a_charge_time" crates/kiln-loot/src/effects.rs 's/            if let Some(effect) = \&e.effects.crossbow_charge_time {/            if let (true, Some(effect)) = (level >= 3, \&e.effects.crossbow_charge_time) {/'
echo "negative checks failing the wrong way: $fails"
exit $fails
