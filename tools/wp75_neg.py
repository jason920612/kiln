"""wp75: negative checks. On the VM, in ~/wt/wp75-gaps: each fix is turned off by a small text edit and the test that guards it must FAIL;
then the tree is restored. usage: python3 tools/wp75_neg.py [case ...]   (log of each run: /tmp/neg75_<case>.log)"""
import os
import re
import subprocess
import sys

ROOT = os.path.expanduser("~/wt/wp75-gaps")
ENV = dict(os.environ, CARGO_BUILD_JOBS=os.environ.get("CARGO_BUILD_JOBS", "3"), KILN_WORK=os.path.expanduser("~/kiln/work"))
ENV["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + ENV["PATH"]
ENV.setdefault("KILN_DATAPACK", os.path.expanduser("~/kiln/work/generated"))

SIM = ["test", "--release", "-p", "kiln-sim"]
VEC = os.path.expanduser("~/kiln/work/wp75")
# name: ([(file, old, new)], [cargo args])
CASES = {
    "tick_order": ([("crates/kiln-sim/src/container/mod.rs", "self.ticking.values().map(|p| (*p, self.index[p]))", "self.index.iter().filter(|(_, k)| ticks(**k)).map(|(p, k)| (*p, *k))")], SIM + ["--test", "hopper_order"]),
    "java_hash_order": ([("crates/kiln-sim/src/container/mod.rs", "    while keys.len() > size / 4 * 3 {", "    while keys.len() > size {")], SIM + ["--lib", "java_hash_map_order_of_block_positions"]),
    "pot_decorations": ([("crates/kiln-sim/src/container/mod.rs", "            normalize_pot_decorations(&mut extra);", "")], SIM + ["--lib", "a_decorated_pot_loads_sherds_in_the_codec_form"]),
    "piston_old_progress": ([("crates/kiln-sim/src/entities/piston.rs", "            m.progress = push.piston.progress;", "            m.progress = push.progress;")], SIM + ["--test", "piston_items"]),
    "summon_order": ([("crates/kiln-sim/src/lib.rs", "            batch.append(&mut d.serial_spawns);", "            batch.append(&mut d.serial_spawns);\n            batch = entities::canonical(batch);")], SIM + ["--test", "summon_order"]),
    "lenient_count": ([("crates/kiln-item/src/stack.rs", "        let count = if (1..=99).contains(&count) { count } else { 1 };", "")], ["test", "--release", "-p", "kiln-item", "--lib", "saved_items_load_leniently"]),
    "lenient_components": ([("crates/kiln-item/src/patch.rs", "            if let Ok((id, value)) = decoded {\n                patch.put(id, value);\n            }", "            match decoded {\n                Ok((id, value)) => patch.put(id, value),\n                Err(_) => patch.unparsed.push((key.to_owned(), v.clone())),\n            }")], ["test", "--release", "-p", "kiln-item", "--lib", "saved_items_load_leniently"]),
    "items_dropped": ([("crates/kiln-inventory/src/persist.rs", "Ok(stack) if (slot as usize) < size => out.stacks[slot as usize] = stack,", "Ok(stack) => out.stacks[(slot as usize) % size] = stack,")], ["test", "--release", "-p", "kiln-inventory", "--lib", "container_items_drop_what_vanilla_drops"]),
    "anger_target": ([("crates/kiln-entity/src/mob/kinds/anger.rs", "        self.target = angry_at.map(|uuid| Reference { id: None, uuid });", "        self.target = None;")], SIM + ["--test", "anger"]),
    "anger_time": ([("crates/kiln-entity/src/mob/kinds/anger.rs", "            self.end = now + left;", "            self.end = left;")], SIM + ["--test", "anger"]),
    "mob_arrows": ([("crates/kiln-sim/src/entities.rs", "            m.stick_in_body(stinger);\n        }\n    }", "            let _ = (m, stinger);\n        }\n    }")], SIM + ["--test", "player_sync", "an_arrow_sticks_in_a_mobs_body"]),
    "attribute_instances": ([("crates/kiln-entity/src/mob/attributes.rs", "        i.created = true;\n        Some(i)", "        i.created = false;\n        Some(i)")], ["test", "--release", "-p", "kiln-entity", "--lib", "an_attribute_has_an_instance_once_it_is_asked_for"]),
    "player_attributes": ([("crates/kiln-sim/src/combat.rs", "                let made = MADE.contains(&a.name) || self.attrs_touched.contains(&a.name) || !mods.is_empty() || base != a.base;", "                let made = true;")], SIM + ["--test", "player_sync", "a_player_is_told_of_the_attributes_that_exist"]),
    "damage_event_order": ([("crates/kiln-sim/src/players.rs", "    packets.append(&mut target.pending_sounds);\n", "")], SIM + ["--test", "player_sync", "the_damage_event_comes_before_the_hurt_sound"]),
    "same_tick_projectile": ([("crates/kiln-sim/src/entities.rs", "let never_looked_at = e.removed && e.section.is_none() && e.kind.tracking_range != 0;", "let never_looked_at = false;")], SIM + ["--test", "player_sync", "a_projectile_gone_in_its_first_tick"]),
    "raid_bar": ([("crates/kiln-sim/src/raid.rs", "            if loaded {\n                self.update_bar_progress(dim, id);\n            }", "")], SIM + ["--test", "raids", "a_raid_in_progress"]),
    "removed_in_tick": ([("crates/kiln-sim/src/entities.rs", "        if !e.removed && e.phys.as_deref().is_some_and(|p| p.is_removed()) {\n            e.sync();", "        if false {\n            e.sync();")], SIM + ["--test", "removed_in_tick"]),
    "spawner_rewrite": ([("crates/kiln-sim/src/mob_spawner.rs", "        e.dirty = !is_saved_as(&e.save(), nbt);", "        e.dirty = false;")], SIM + ["--lib", "a_spawner_missing_its_defaults_is_rewritten"]),
    # With the vanilla vectors (they print how many scenarios still agree):
    "tick_order_vectors": ([("crates/kiln-sim/src/container/mod.rs", "self.ticking.values().map(|p| (*p, self.index[p]))", "self.index.iter().filter(|(_, k)| ticks(**k)).map(|(p, k)| (*p, *k))")], SIM + ["--lib", "container_parity", "--", "--nocapture"], {"KILN_CONTAINER_VECTORS": VEC + "/containers/hc.jsonl"}),
    "piston_old_progress_vectors": ([("crates/kiln-sim/src/entities/piston.rs", "            m.progress = push.piston.progress;", "            m.progress = push.progress;")], SIM + ["--lib", "container_parity", "--", "--nocapture"], {"KILN_CONTAINER_VECTORS": VEC + "/containers/pi.jsonl"}),
    "summon_order_vectors": ([("crates/kiln-sim/src/lib.rs", "            batch.append(&mut d.serial_spawns);", "            batch.append(&mut d.serial_spawns);\n            batch = entities::canonical(batch);")], SIM + ["--lib", "container_parity", "--", "--nocapture"], {"KILN_CONTAINER_VECTORS": VEC + "/containers/pi.jsonl"}),
    "load_vectors_blocks": ([("crates/kiln-item/src/stack.rs", "        let count = if (1..=99).contains(&count) { count } else { 1 };", "")], SIM + ["--lib", "load_parity", "--", "--nocapture"], {"KILN_LOAD_VECTORS": VEC + "/load/ld.jsonl"}),
    "attr_vectors": ([("crates/kiln-entity/src/mob/attributes.rs", "        i.created = true;\n        Some(i)", "        i.created = false;\n        Some(i)")], SIM + ["--lib", "attr_parity", "--", "--nocapture"], {"KILN_ATTR_VECTORS": VEC + "/attrs/at.jsonl"}),
}


def sh(*args, **kw):
    return subprocess.run(list(args), cwd=ROOT, env=ENV, **kw)


def main():
    names = sys.argv[1:] or list(CASES)
    sh("git", "checkout", "-q", "-f", ".")
    for name in names:
        edits, cargo, *extra = CASES[name]
        extra_env = extra[0] if extra else {}
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
            log = f"/tmp/neg75_{name}.log"
            with open(log, "w") as f:
                r = subprocess.run(["cargo", *cargo], cwd=ROOT, env=dict(ENV, **extra_env), stdout=f, stderr=subprocess.STDOUT)
            body = open(log, encoding="utf-8", errors="replace").read()
            for line in body.splitlines():
                if re.search(r"scenarios pass|match vanilla|instances: ", line):
                    print("    " + line.strip()[:200])
            if r.returncode == 0:
                print(f"NEGATIVE {name}: the test PASSED without the fix (BAD)")
            elif re.search(r"test result: FAILED|panicked", body):
                print(f"NEGATIVE {name}: the test fails without the fix (good)")
            else:
                print(f"NEGATIVE {name}: did not run (build error?), see {log}")
        sh("git", "checkout", "-q", "-f", ".")
        sys.stdout.flush()


main()
