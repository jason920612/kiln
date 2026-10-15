#!/bin/bash
# wp66: negative checks. On the VM, in ~/wt/wp66-wp65-gaps: each fix is taken out (a sed that turns it off, or the reverse of its commit's
# source changes) and the test that guards it must FAIL; then the tree is restored. usage: tools/wp66_negative.sh [case ...]
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4} KILN_WORK=$HOME/kiln/work
cd ~/wt/wp66-wp65-gaps || exit 1
git checkout -q -f . 
run() { # name, test command...
  local name=$1; shift
  if "$@" > /tmp/neg_$name.log 2>&1; then echo "NEGATIVE $name: the test PASSED without the fix (BAD)"; else
    if grep -q "test result: FAILED\|panicked" /tmp/neg_$name.log; then echo "NEGATIVE $name: the test fails without the fix (good)"; else echo "NEGATIVE $name: did not run (build error?) see /tmp/neg_$name.log"; fi
  fi
  git checkout -q -f .
}
want() { [ $# -eq 0 ] || [[ " $WANT " == *" $1 "* ]]; }
WANT="$*"
sel() { [ -z "$WANT" ] || [[ " $WANT " == *" $1 "* ]]; }

if sel random_tick; then
  sed -i 's/\.and_then(|cell| cell\.rand_value())/.and_then(|_cell| None::<i32>)/' crates/kiln-sim/src/blocks.rs
  run random_tick cargo test --release -p kiln-sim --test blocks random_tick_positions_continue
fi
if sel be_defaults; then
  sed -i '/^fn always_saved/a\    if true { return Vec::new(); }' crates/kiln-world/src/block_entity.rs
  run be_defaults cargo test --release -p kiln-world a_new_block_entity_saves
fi
if sel gateway_age; then
  sed -i 's/^            crate::gateway::tick(&mut level, &ticking);/            \/\/ gateway tick off/' crates/kiln-sim/src/region.rs
  run gateway_age cargo test --release -p kiln-sim --test gateway_trips a_gateway_counts_its_age
fi
if sel tick_function_order; then
  git show b460163f -- crates | git apply -R && run tick_function_order cargo test --release -p kiln-sim --test functions_tick
fi
if sel forceload; then
  git show 33ed974e -- crates/kiln-sim/src | git apply -R && run forceload cargo test --release -p kiln-sim --test forceload
fi
if sel raid_bar; then
  git show 2b28efce -- crates/kiln-entity/src crates/kiln-sim/src | git apply -R && run raid_bar cargo test --release -p kiln-sim --test raids the_raid_bar
fi
if sel fireworks; then
  sed -i '/^pub fn launch_firework/a\    if true { return; }' crates/kiln-entity/src/mob/kinds/villager.rs
  run fireworks cargo test --release -p kiln-sim --test raids villagers_launch
fi
if sel bell; then
  git show 909d0c1d -- crates/kiln-sim/src | git apply -R && run bell cargo test --release -p kiln-sim --test raids a_rung_bell
fi
