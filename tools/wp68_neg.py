"""wp68: negative checks. On the VM, in ~/wt/wp68-playthrough6: each fix is turned off by a small text edit and the test that guards it must FAIL;
then the tree is restored. usage: python3 tools/wp68_neg.py [case ...]   (log of each run: /tmp/neg68_<case>.log)"""
import os
import re
import subprocess
import sys

ROOT = os.path.expanduser("~/wt/wp68-playthrough6")
ENV = dict(os.environ, CARGO_BUILD_JOBS=os.environ.get("CARGO_BUILD_JOBS", "5"), KILN_WORK=os.path.expanduser("~/kiln/work"))
ENV["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + ENV["PATH"]

# name: ([(file, old, new)], [cargo args], expected substring in the failing test names (optional))
CASES = {
    "age_lock": ([("crates/kiln-entity/src/mob/interact.rs", '    if !is(stack, "minecraft:golden_dandelion") || !m.baby()', '    if true { return None; }\n    if !is(stack, "minecraft:golden_dandelion") || !m.baby()')], ["test", "--release", "-p", "kiln-entity", "--test", "age_lock"]),
    "sulfur_bucket": ([("crates/kiln-entity/src/mob/kinds/sulfur_cube.rs", '    if stack.is_empty() || mob::item_name(stack) != "minecraft:bucket"', '    if true || stack.is_empty() || mob::item_name(stack) != "minecraft:bucket"')], ["test", "--release", "-p", "kiln-entity", "--test", "sulfur_bucket"]),
    "dirty_entity_data_unit": ([("crates/kiln-proto/src/packets/entity/metadata.rs", "&& let Some(default) = default_value(index, serializer)", "&& let Some(default) = default_value(index, serializer).filter(|_| false)")], ["test", "--release", "-p", "kiln-proto", "a_value_back_at_its_default"]),
    "dirty_entity_data_sim": ([("crates/kiln-proto/src/packets/entity/metadata.rs", "&& let Some(default) = default_value(index, serializer)", "&& let Some(default) = default_value(index, serializer).filter(|_| false)")], ["test", "--release", "-p", "kiln-sim", "--test", "entity_data_reset"]),
    "data_merge_riders": ([("crates/kiln-sim/src/command_data.rs", "                    loaded.passengers = passengers;", "                    let _ = passengers;")], ["test", "--release", "-p", "kiln-sim", "--test", "data_merge_riders"]),
    "item_replace_mount_slots": ([("crates/kiln-sim/src/command_data.rs", "        105 => Some(6),\n        106 => Some(7),\n", "")], ["test", "--release", "-p", "kiln-sim", "--test", "item_replace_mount_slots"]),
    "sound_ids": ([("crates/kiln-blocks/src/behaviour/misc2.rs", '"minecraft:entity.turtle.egg_crack"', '"minecraft:block.turtle_egg.crack_old"')], ["test", "--release", "-p", "kiln-sim", "--test", "sound_ids"]),
    "death_loot_specific": ([("crates/kiln-sim/src/mobs.rs", "P::Sheep { sheared } => self.specific.sheared.is_some_and(|s| sheared.is_none_or(|w| w == s)),", "P::Sheep { .. } => false,")], ["test", "--release", "-p", "kiln-sim", "--test", "death_loot_specific"]),
    "sniffer_egg": ([("crates/kiln-blocks/src/behaviour/mod.rs", "        C::SnifferEggBlock => misc::sniffer_egg_on_place(level, s, pos),", "        C::SnifferEggBlock if !state::same_block(old, s) => misc::sniffer_egg_on_place(level, s, pos),")], ["test", "--release", "-p", "kiln-sim", "--test", "sniffer_egg"]),
    "bundle_selection": ([("crates/kiln-inventory/src/bundle.rs", "let index = if self.outside(self.selected) { 0 } else { self.selected as usize };", "let index = 0usize;")], ["test", "--release", "-p", "kiln-inventory", "the_selected_stack_is_the_one_removed"]),
    "camel_flag": ([("crates/kiln-entity/src/mob/kinds/camel.rs", "        if s.eating {\n            d.set(data::abstract_horse::ID_FLAGS", "        if true || s.eating {\n            d.set(data::abstract_horse::ID_FLAGS")], ["test", "--release", "-p", "kiln-sim", "--lib", "a_camel_does_not_send_the_tame_flag"]),
    "mannequin_flags": ([("crates/kiln-sim/src/mobs.rs", "    if m.kind.is_mob() {", "    if true {")], ["test", "--release", "-p", "kiln-sim", "--lib", "every_mob_sends_only_fields_of_its_entity_type"]),
    "ghastling": ([("crates/kiln-sim/src/blocks.rs", "let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::HappyGhast, 0, 0, h as i64);", "let mut e = kiln_entity::mob::new(kiln_entity::mob::MobKind::Pig, 0, 0, h as i64);")], ["test", "--release", "-p", "kiln-sim", "--test", "ghastling"]),
    "bees_rain": ([("crates/kiln-sim/src/beehive.rs", " || env.weather.weather.weather_layer", "")], ["test", "--release", "-p", "kiln-sim", "--test", "bees_rain"]),
    "shelves": ([("crates/kiln-sim/src/tools.rs", "        C::ShelfBlock => crate::shelf::use_item_on(p, level, pos, s, face, cursor, off_hand),\n", "")], ["test", "--release", "-p", "kiln-sim", "--test", "shelves"]),
    "explosion_packet": ([("crates/kiln-entity/src/explosion.rs", "            hit_players.push((id, push));", "            let _ = (id, push);")], ["test", "--release", "-p", "kiln-sim", "--test", "explosion_packet"]),
}


def sh(*args, **kw):
    return subprocess.run(list(args), cwd=ROOT, env=ENV, **kw)


def main():
    names = sys.argv[1:] or list(CASES)
    sh("git", "checkout", "-q", "-f", ".")
    for name in names:
        edits, cargo = CASES[name]
        ok = True
        for rel, old, new in edits:
            path = os.path.join(ROOT, rel)
            text = open(path, encoding="utf-8", newline="").read()
            if old not in text:
                print(f"NEGATIVE {name}: the text to edit is not in {rel} (case is stale)")
                ok = False
                break
            open(path, "w", encoding="utf-8", newline="").write(text.replace(old, new, 1))
        if ok:
            log = f"/tmp/neg68_{name}.log"
            with open(log, "w") as f:
                r = sh("cargo", *cargo, stdout=f, stderr=subprocess.STDOUT)
            body = open(log, encoding="utf-8", errors="replace").read()
            if r.returncode == 0:
                print(f"NEGATIVE {name}: the test PASSED without the fix (BAD)")
            elif re.search(r"test result: FAILED|panicked", body):
                print(f"NEGATIVE {name}: the test fails without the fix (good)")
            else:
                print(f"NEGATIVE {name}: did not run (build error?), see {log}")
        sh("git", "checkout", "-q", "-f", ".")
        sys.stdout.flush()


main()
