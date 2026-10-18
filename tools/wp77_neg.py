"""wp77: negative checks. On the VM, in ~/wt/wp77-gaps: each fix is turned off by a small text edit and the test that guards it must FAIL;
then the tree is restored. usage: python3 tools/wp77_neg.py [case ...]   (log of each run: /tmp/neg77_<case>.log)"""
import os
import re
import subprocess
import sys

ROOT = os.path.expanduser("~/wt/wp77-gaps")
ENV = dict(os.environ, CARGO_BUILD_JOBS=os.environ.get("CARGO_BUILD_JOBS", "3"), KILN_WORK=os.path.expanduser("~/kiln/work"))
ENV["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + ENV["PATH"]
ENV.setdefault("KILN_DATAPACK", os.path.expanduser("~/kiln/work/generated"))

SIM = ["test", "--release", "-p", "kiln-sim"]
ENT = ["test", "--release", "-p", "kiln-entity"]
VEC = os.path.expanduser("~/kiln/work/wp77")
# name: ([(file, old, new)], [cargo args], {extra env})
CASES = {
    "attributes_saved": ([("crates/kiln-entity/src/mob/persist.rs", "        .filter(|i| i.created)", "        .filter(|_| true)")], SIM + ["--test", "saved_parity", "--", "--nocapture"], {"KILN_SAVED_VECTORS": VEC + "/saved.jsonl"}),
    "attribute_transient": ([("crates/kiln-entity/src/mob/persist.rs", "!md.transient", "true")], SIM + ["--test", "saved_parity", "--", "--nocapture"], {"KILN_SAVED_VECTORS": VEC + "/saved.jsonl"}),
    "player_attributes": ([("crates/kiln-sim/src/persist.rs", '("attributes", p.attributes_nbt())', '("attributes_off", p.attributes_nbt())')], SIM + ["--test", "player_attributes"], {}),
    "brain_unit_compound": ([("crates/kiln-entity/src/mob/brain/persist.rs", "matches!(t, Tag::Compound(_)).then_some(Val::Unit)?", "Val::Unit")], SIM + ["--test", "brain_parity", "--", "--nocapture"], {"KILN_BRAIN_VECTORS": VEC + "/brt.jsonl"}),
    "mob_cap": ([("crates/kiln-sim/src/lib.rs", "env.blocks.spawn_totals = Some(std::sync::Arc::new(totals));", "let _ = totals;")], SIM + ["--test", "spawn_cap"], {}),
    "wolf_pack": ([("crates/kiln-entity/src/mob/kinds/wolf.rs", "    fn max_spawn_cluster(&self) -> i32 {\n        8", "    fn max_spawn_cluster(&self) -> i32 {\n        4")], ENT + ["--test", "spawn_cluster"], {}),
    "fishing_bite_dip": ([("crates/kiln-entity/src/ext_entity/fishing_hook.rs", "                if !self.biting {", "                if false {")], ENT + ["--test", "fishing_parity", "--", "--nocapture"], {"KILN_FISHING_VECTORS": VEC + "/fishing.jsonl"}),
    "gossip_order": ([("crates/kiln-entity/src/mob/gossip.rs", "std::cmp::Ordering::Greater => high = mid - 1,", "std::cmp::Ordering::Greater => low = mid + 1,")], ENT + ["--test", "villager_parity", "--", "--nocapture"], {"KILN_VILLAGER_VECTORS": VEC + "/villagers.jsonl"}),
    "cat_gift": ([("crates/kiln-entity/src/mob/kinds/cat.rs", "        if !tame::is_tame(m) || tame::ordered_to_sit(m) {\n            return false;\n        }\n        let Some(owner) = tame::owner(m, level) else { return false };\n        self.owner = Some(owner.id);", "        return false;\n        #[allow(unreachable_code)]\n        {\n        let Some(owner) = tame::owner(m, level) else { return false };\n        self.owner = Some(owner.id);")], SIM + ["--test", "cat_gift"], {}),
    "damage_event_order": ([("crates/kiln-sim/src/entities.rs", "                p.hurt(amount, &source, &mut ctx);\n                self.events.push(Event::PlayerHurt { target: id });", "                p.hurt(amount, &source, &mut ctx);")], SIM + ["--test", "player_sync", "the_damage_event_comes_before_a_level_event"], {}),
    "zombie_breaks_doors": ([("crates/kiln-entity/src/mob/kinds/zombie.rs", "        m.goals.add(1, Goal::Custom(Box::new(super::break_door::BreakDoorGoal::new(super::break_door::Who::Zombie))));", "")], SIM + ["--test", "door_break", "a_zombie_breaks_a_door"], {}),
    "zombie_village": ([("crates/kiln-entity/src/mob/kinds/zombie.rs", "    g.add(6, Goal::Custom(Box::new(super::move_through_village::MoveThroughVillageGoal::new(1.0, true, 4, can_break_doors))));", "    g.add(6, Goal::Never);")], ENT + ["--test", "village_goals", "a_zombie"], {}),
    "fox_village": ([("crates/kiln-entity/src/mob/kinds/fox.rs", "        g.add(9, Goal::Custom(Box::new(FoxStrollThroughVillageGoal { interval: mth::reduced_tick_delay(200), wanted: None })));", "        g.add(9, Goal::Never);")], ENT + ["--test", "village_goals", "a_fox"], {}),
    "start_height": ([("crates/kiln-sim/src/spawner.rs", "    chunk.column_height((x & 15) as usize, (z & 15) as usize, |b| !kiln_data::blocks_types::is_air(b))\n}\n\nfn spawn_category_for_chunk(", "    chunk.column_height((x & 15) as usize, (z & 15) as usize, |b| !kiln_data::blocks_types::is_air(b)) + 1\n}\n\nfn spawn_category_for_chunk(")], SIM + ["--lib", "a_start_reaches_no_higher"], {}),
    "portal_piglins": ([("crates/kiln-blocks/src/behaviour/mod.rs", "            C::NetherPortalBlock => portal::random_tick(level, pos),", "")], SIM + ["--test", "portal_piglins"], {}),
    "raid_enchant": ([("crates/kiln-entity/src/mob/kinds/vindicator.rs", '        crate::enchanting::enchant_from_provider(&mut axe, provider, 0.0, &mut e.random);', "        let _ = provider;")], SIM + ["--lib", "raiders_get_their_enchanted_weapons"], {}),
}


def sh(*args, **kw):
    return subprocess.run(list(args), cwd=ROOT, env=ENV, **kw)


def main():
    names = sys.argv[1:] or list(CASES)
    sh("git", "checkout", "-q", "-f", ".")
    for name in names:
        edits, cargo, extra_env = CASES[name]
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
            log = f"/tmp/neg77_{name}.log"
            with open(log, "w") as f:
                r = subprocess.run(["cargo", *cargo], cwd=ROOT, env=dict(ENV, **extra_env), stdout=f, stderr=subprocess.STDOUT)
            body = open(log, encoding="utf-8", errors="replace").read()
            for line in body.splitlines():
                if re.search(r"cases, .* differ|saved: |brain: |villagers: |fishing: ", line):
                    print("    " + line.strip()[:200])
            if r.returncode == 0:
                print(f"NEGATIVE {name}: the test PASSED without the fix (BAD)")
            elif re.search(r"test result: FAILED|panicked", body):
                print(f"NEGATIVE {name}: the test fails without the fix (good)")
            else:
                print(f"NEGATIVE {name}: did not run (build error?), see {log}")
        sh("git", "checkout", "-q", "-f", ".")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
