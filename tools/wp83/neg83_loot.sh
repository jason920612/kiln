#!/bin/bash
# wp83 loot negative checks (on the VM, in the wp83-loot checkout): take a fix out again (the changed files back to the base revision d14de992, or the call
# sed'ed out), run its test, and require it to FAIL; then restore. usage: ~/kt wp83-loot bash tools/wp83/neg83_loot.sh
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3} RUST_TEST_THREADS=${RUST_TEST_THREADS:-4} KILN_DATAPACK=${KILN_DATAPACK:-$HOME/kiln/work/generated}
base=d14de992
fails=0
verdict() { # name
  if grep -q "^error" /tmp/neg83.out && ! grep -q "^test .* FAILED" /tmp/neg83.out; then echo "BUILD ERROR (bad): $1"; fails=$((fails + 1))
  else echo "fails without the fix (good): $1"; fi
}
# neg "<name>" "<test target args>" file...  (files put back to $base)
neg() {
  local name="$1" tests="$2"; shift 2
  for f in "$@"; do git show "$base:$f" > "$f" || { echo "cannot read $base:$f"; exit 2; }; done
  if cargo test --release $tests > /tmp/neg83.out 2>&1; then echo "NOT FAILING (bad): $name"; fails=$((fails + 1)); else verdict "$name"; fi
  git checkout -- "$@"
}
# negsed "<name>" "<test args>" file 'sed expression': edit the file, run, require FAIL, restore
negsed() {
  local name="$1" tests="$2" f="$3" expr="$4"
  sed -i "$expr" "$f"
  if git diff --quiet -- "$f"; then echo "SED CHANGED NOTHING (bad): $name"; fails=$((fails + 1)); git checkout -- "$f"; return; fi
  if cargo test --release $tests > /tmp/neg83.out 2>&1; then echo "NOT FAILING (bad): $name"; fails=$((fails + 1)); else verdict "$name"; fi
  git checkout -- "$f"
}
T="-p kiln-sim --test loot_calls"
neg "carve/pumpkin" "$T carving_a_pumpkin" crates/kiln-sim/src/tools.rs
neg "harvest/sweet_berry_bush" "$T picking_sweet_berries" crates/kiln-sim/src/tools.rs
neg "harvest/cave_vine" "$T picking_glow_berries" crates/kiln-sim/src/tools.rs
neg "till/rooted_dirt" "$T hoeing_rooted_dirt" crates/kiln-sim/src/tools.rs
neg "harvest/beehive" "$T shearing_a_full_beehive" crates/kiln-sim/src/beehive.rs crates/kiln-sim/src/container/dispense_items.rs
negsed "charged_creeper/root" "$T a_charged_creeper" crates/kiln-entity/src/mob/species.rs 's/super::creeper_blast(e, charged, || \(.*\));$/let _ = charged; \1;/'
negsed "spawn data equipment (mob spawner)" "$T a_spawner_s_mobs_wear" crates/kiln-entity/src/spawner.rs '/mob::spawn_equipment::equip_from_table(&mut e, level, equipment);/d'
negsed "shearing tool context" "$T shearing_a_sheep" crates/kiln-sim/src/entities.rs 's/DeathExtra { tool: Some(stack.clone()), no_damage_source: true, ..Default::default() }/DeathExtra::default()/'
negsed "jungle temple dispenser table id" "-p kiln-sim --test loot_table_names" crates/kiln-worldgen/src/structure/legacy/scattered.rs 's|minecraft:chests/jungle_temple_dispenser|minecraft:dispensers/jungle_temple|'
echo "negative checks failing the wrong way: $fails"
exit $fails
