#!/bin/bash
# on the VM, in the wp78-playthrough9 checkout: negative checks — each fix is switched off with a one-line text edit, the test for it must FAIL, then the file is restored.
# usage (from the Windows side): ssh test@192.168.1.169 "CARGO_BUILD_JOBS=4 ~/kt wp78-playthrough9 bash tools/wp78/neg78.sh"
set -u
check() {  # name file perl-edit test-args...
  local name=$1 file=$2 edit=$3; shift 3
  perl -0pi -e "$edit" "$file"
  if git diff --quiet -- "$file"; then echo "NEG $name: the edit did not apply"; return; fi
  if cargo test --release -q -p kiln-sim "$@" > /tmp/neg_$name.txt 2>&1; then echo "NEG $name: tests still PASS without the fix  <-- bad"; else echo "NEG $name: tests FAIL without the fix (good)"; fi
  git checkout -q -- "$file"
}
check wither_rose crates/kiln-entity/src/mob/mod.rs 's/    if credit_is_wither \{\n        create_wither_rose/    if false \&\& credit_is_wither {\n        create_wither_rose/' --test wither
check power_punch crates/kiln-sim/src/enchant.rs 's/self\.0\.projectile_modify_damage\(hit\.weapon, hit\.attacker, hit\.direct, hit\.victim, hit\.damage_type, damage, random\)/{ let _ = (hit, random); damage }/; s/self\.0\.projectile_modify_knockback\(hit\.weapon, hit\.attacker, hit\.direct, hit\.victim, hit\.damage_type, value, random\)/{ let _ = (hit, random); value }/' --test arrow_enchants
check boss_bar crates/kiln-sim/src/players.rs 's/bars\.push\(\(\*v, b\)\);/let _ = b;/' --test wither
