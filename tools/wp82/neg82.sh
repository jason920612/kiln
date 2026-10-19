#!/bin/bash
# wp82 (on the VM, in ~/wt/wp82-gaps): neg82.sh [CASE...]   the negative checks: each case switches one fix off with a perl edit of one file, the named test must FAIL, then the file is restored.
# Without arguments all cases run. Logs in ~/wp82/neg_<case>.log.
export PATH="$HOME/.cargo/bin:$PATH" KILN_WORK="$HOME/kiln/work" KILN_DATAPACK="$HOME/kiln/work/generated" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3} RUST_TEST_THREADS=3
cd "$HOME/wt/wp82-gaps" || exit 2
declare -A FILE EDIT TEST
add() { FILE[$1]=$2; EDIT[$1]=$3; TEST[$1]=$4; ORDER+=("$1"); }
add raidtick   crates/kiln-sim/src/raid.rs        's~if !loaded \|\| !ticking\.contains\(ChunkPos::of_block\(x, z\)\) \{~if !loaded {~' "-p kiln-sim --test raids a_wave_spawns_where"
add raidfrozen crates/kiln-sim/src/lib.rs         's~self\.tick_raids\(normal\);~self.tick_raids(true);~' "-p kiln-sim --test raids a_frozen_game"
add carrots    crates/kiln-sim/src/tools.rs       's~C::CropBlock \| C::CarrotBlock \| C::PotatoBlock \|~C::CropBlock |~g' "-p kiln-sim --test bone_meal_wp82 bone_meal_ages_carrots"
add shelf      crates/kiln-blocks/src/behaviour/bonemeal.rs 's~(C::ShelfMushroomBlock => \{)~$1 return None;~' "-p kiln-sim --test bone_meal_wp82 bone_meal_ripens_a_shelf"
add lichen     crates/kiln-blocks/src/behaviour/bonemeal.rs 's~(C::GlowLichenBlock => \{)~$1 return None;~' "-p kiln-sim --test bone_meal_wp82 bone_meal_spreads_glow_lichen"
add combat     crates/kiln-sim/src/health.rs      's~self\.send\(packets::player::player_combat_enter\(\)\);~();~' "-p kiln-sim --test combat_packets a_hit_from"
add combatend  crates/kiln-sim/src/health.rs      's~if \(self\.tick_count \+ 1\) % 20 == 0~if (self.tick_count + 1) % 20 == 7~' "-p kiln-sim --test combat_packets a_hit_from"
add projpower  crates/kiln-sim/src/entities.rs    's~match e\.phys\.as_deref\(\)\.and_then\(kiln_entity::ext_entity::acceleration_power\)~match None::<f64>~' "-p kiln-sim --test combat_packets a_fireballs"
add packs      crates/kiln-sim/src/entities.rs    's~let mut shared = f\.group\.as_ref\(\)\.map\(\|g\| g\.0\.lock\(\)\.expect\("group"\)\);~let mut shared: Option<std::sync::MutexGuard<'"'"'_, kiln_entity::mob::GroupData>> = None;~' "-p kiln-sim --lib mobs::tests"
add portaltime crates/kiln-sim/src/portal.rs      's~(                self\.portal_trips_end\.push\(trip\);\n            \}\n        \}\n)(    \}\n\n    /// `Entity\.handlePortal`)~$1        self.run_portal_trips(true);\n        self.run_portal_trips(false);\n$2~' "-p kiln-sim --test dimensions a_mob_in_a_portal_goes_through"
add gwmotion   crates/kiln-sim/src/portal.rs      's~if !pearl \{~if false {~' "-p kiln-sim --test gateway_entities a_gateway_keeps"
add befields   crates/kiln-sim/src/container/mod.rs 's~ && KEPT\.contains\(&k\.as_str\(\)\)~~' "-p kiln-sim --test be_fields a_block_entity_drops"
add lectern    crates/kiln-sim/src/container/mod.rs 's~int\("Page", 0\)\.max\(0\)\.min\(page_count\(&book\) - 1\)~int("Page", 0).clamp(0, (page_count(\&book) - 1).max(0))~' "-p kiln-sim --test be_fields a_lectern_page"
add maprefs    crates/kiln-sim/src/maps.rs        's~maps\.lock\(\)\.unwrap_or_else\(\|e\| e\.into_inner\(\)\)\.try_add_reference\(chunk, name\)~true~' "-p kiln-sim --lib maps::tests::a_second_map"
run() {
  local name=$1 file=${FILE[$1]} edit=${EDIT[$1]} test=${TEST[$1]}
  git checkout -q -- "$file"
  perl -0pi -e "$edit" "$file"
  if git diff --quiet -- "$file"; then echo "NEG $name: the edit changed nothing"; return 3; fi
  if cargo test --release $test > "$HOME/wp82/neg_$name.log" 2>&1; then
    echo "NEG $name: PASSED without the fix (bad)"; r=1
  elif grep -q "error\[E\|could not compile" "$HOME/wp82/neg_$name.log"; then echo "NEG $name: does not compile (invalid case)"; r=3
  else echo "NEG $name: failed (good)"; r=0; fi
  git checkout -q -- "$file"
  return $r
}
cases=("$@"); [ ${#cases[@]} -eq 0 ] && cases=("${ORDER[@]}")
bad=0
for c in "${cases[@]}"; do run "$c" || bad=1; done
exit $bad
