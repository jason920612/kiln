#!/bin/bash
# wp81 (on the VM, in ~/wt/wp81-gaps): neg81.sh [CASE...]   the negative checks: each case switches one fix off with a perl edit of one file, the named test must FAIL, then the file is restored.
# Without arguments all cases run. Logs in ~/wp81/neg_<case>.log.
export PATH="$HOME/.cargo/bin:$PATH" KILN_WORK="$HOME/kiln/work" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
cd "$HOME/wt/wp81-gaps" || exit 2
declare -A FILE EDIT TEST
add() { FILE[$1]=$2; EDIT[$1]=$3; TEST[$1]=$4; ORDER+=("$1"); }
add rings      crates/kiln-sim/src/lib.rs        's~rings\.extend\(tickets::ring_chunks\(c, r\)\);~rings.extend(tickets::ring_chunks(c, 0));~' "-p kiln-sim --test tickets a_forced_chunk_loads_two_rings"
add blockring  crates/kiln-sim/src/blocks.rs     's~\(&mut self\.block, radius - 1\)~(&mut self.block, radius - 2)~' "-p kiln-sim --test tickets water_flows_from_the_block"
add vplus2     crates/kiln-sim/src/region.rs     's~const LOADED_BEYOND_VIEW: i32 = 2;~const LOADED_BEYOND_VIEW: i32 = 0;~' "-p kiln-sim --test tickets the_server_loads_two_chunks"
add pearl      crates/kiln-entity/src/projectile.rs 's~pearl_ticket\(e, level, chunk_before\);~();~' "-p kiln-sim --test tickets a_pearl_keeps_its_chunk"
add pearlsave  crates/kiln-sim/src/persist.rs    's~fields\.push\(\("ender_pearls"\.to_owned\(\), Tag::List\(pearls\)\)\);~();~' "-p kiln-sim --test tickets a_players_pearl_leaves"
add landing    crates/kiln-sim/src/entities.rs   's~level\.blocks\.pearl_landings\.push\(\(owner, to\)\);~();~' "-p kiln-sim --test tickets a_pearl_keeps_its_chunk"
add frozen     crates/kiln-sim/src/lib.rs        's~ && self\.world\.tick_rate\.runs_normally\(\) \{~ {~' "-p kiln-sim --test tickets a_frozen_game"
add ifloaded   crates/kiln-sim/src/commands.rs   's~near\(p\.center, view\)~near(p.center, view + 2)~' "-p kiln-sim --test tickets execute_if_loaded"
add wither     crates/kiln-entity/src/mob/kinds/wither.rs 's~let arrow = matches!~let arrow = direct.is_none() || matches!~' "-p kiln-sim --test wither a_powered_wither"
add portalcd   crates/kiln-sim/src/portal.rs     's~standing\.push\(\(dim, e\.uuid\.as_u128\(\)\)\);~();~' "-p kiln-sim --test dimensions a_mob_standing"
add portalticket crates/kiln-sim/src/portal.rs   's~(fn place_portal_ticket\(&mut self, dim: DimId, at: \[f64; 3\]\) \{)~$1 if true { return; }~' "-p kiln-sim --test dimensions a_mob_that_went"
add mapreload  crates/kiln-sim/src/datapacks.rs  's~let explorer = self\.map_explorer\.clone\(\);~let explorer: Option<kiln_loot::ExplorerHandle> = None;~' "-p kiln-sim --test map_reload"
add gateway    crates/kiln-sim/src/portal.rs     's~\} else if first == block::END_GATEWAY \{~} else if false \&\& first == block::END_GATEWAY {~' "-p kiln-sim --test dimensions a_mob_in_an_end_gateway"
add spawnring  crates/kiln-sim/src/spawner.rs    's~if ticking\.contains_visible\(c\) && s\.any_close\(c\) \{~if ticking.contains_block(c) \&\& ticking.contains_visible(c) \&\& s.any_close(c) {~' "-p kiln-sim --test spawn_ring"
add statuslayer crates/kiln-sim/src/blocks.rs    's~loading_block: layer_around\(&centers, view \+ 1\),~loading_block: layer_around(\&centers, view),~' "-p kiln-sim --lib blocks::tests::blocks_tick_one"
add randomlayer crates/kiln-sim/src/blocks.rs    's~random: layer_around\(&centers, simulation\.min\(view \+ 1\)\),~random: layer_around(\&centers, simulation.min(view)),~' "-p kiln-sim --lib blocks::tests::blocks_tick_one"
run() {
  local name=$1 file=${FILE[$1]} edit=${EDIT[$1]} test=${TEST[$1]}
  git checkout -q -- "$file"
  perl -0pi -e "$edit" "$file"
  if git diff --quiet -- "$file"; then echo "NEG $name: the edit changed nothing"; return 3; fi
  if cargo test --release $test > "$HOME/wp81/neg_$name.log" 2>&1; then
    echo "NEG $name: PASSED without the fix (bad)"; r=1
  elif grep -q "error\[E\|could not compile" "$HOME/wp81/neg_$name.log"; then echo "NEG $name: does not compile (invalid case)"; r=3
  else echo "NEG $name: failed (good)"; r=0; fi
  git checkout -q -- "$file"
  return $r
}
cases=("$@"); [ ${#cases[@]} -eq 0 ] && cases=("${ORDER[@]}")
bad=0
for c in "${cases[@]}"; do run "$c" || bad=1; done
exit $bad
