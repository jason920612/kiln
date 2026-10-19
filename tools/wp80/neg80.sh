#!/bin/bash
# wp80 negative checks (on the VM, in the wp80-playthrough10 checkout): put the files a fix changed back to what they were before the fix, run the fix's test, and
# require it to FAIL; then restore. usage: ~/kt wp80-playthrough10 bash tools/wp80/neg80.sh
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
fails=0
# neg "<name>" "<test target args>" "<rev>" file...
neg() {
  local name="$1" tests="$2" rev="$3"; shift 3
  for f in "$@"; do git show "$rev:$f" > "$f" || { echo "cannot read $rev:$f"; exit 2; }; done
  if cargo test --release $tests > /tmp/neg80.out 2>&1; then
    echo "NOT FAILING (bad): $name"; fails=$((fails + 1))
  else
    if grep -q "^error" /tmp/neg80.out && ! grep -q "^test .* FAILED" /tmp/neg80.out; then echo "BUILD ERROR (bad): $name"; fails=$((fails + 1)); else echo "fails without the fix (good): $name"; fi
  fi
  git checkout -- "$@"
}
neg "players are pushed and tilted by projectiles" "-p kiln-sim --test player_hurt_animation" 3ff235c6 crates/kiln-sim/src/health.rs crates/kiln-sim/src/entities.rs
neg "landing dust" "-p kiln-sim --test landing_dust" 1b3391db crates/kiln-entity/src/entity.rs crates/kiln-entity/src/mob/mod.rs
neg "eyeblossoms follow the night" "-p kiln-sim --test eyeblossom" b5e1cb23 crates/kiln-blocks/src/behaviour/mod.rs
neg "bone meal (dispenser)" "-p kiln-sim --test bone_meal_dispenser" 1b3391db crates/kiln-sim/src/tools.rs
neg "beacon levels in data get block" "-p kiln-sim --test beacon_pyramids data_get_block_shows" 0849149f crates/kiln-sim/src/container/mod.rs
neg "crafter fields from a command-placed block" "-p kiln-sim --test beacon_pyramids crafter" bf89e37e crates/kiln-sim/src/container/mod.rs
neg "data merge keeps the beacon's levels" "-p kiln-sim --test beacon_pyramids merge" 6aeea9ef crates/kiln-sim/src/container/mod.rs
neg "gateway entities" "-p kiln-sim --test gateway_entities" ed4a2cd6 crates/kiln-sim/src/portal.rs
neg "lodestone compass" "-p kiln-sim --test lodestone_compass" ed18b83e crates/kiln-sim/src/tools.rs
neg "LoveCause" "-p kiln-sim --test love_cause" af637e1f crates/kiln-entity/src/mob/persist.rs
echo "negative checks failing the wrong way: $fails"
exit $fails
