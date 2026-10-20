"""wp87 negative checks (on the VM, in the worktree: ~/kt wp87-gaps python3 tools/wp87/neg87.py [case ...]).

Each case switches one fix off with a one-line text edit, runs the tests that guard it (they must FAIL), and puts the file back with git.
usage: python3 tools/wp87/neg87.py [case ...]    (no case: all)    NEG_JOBS=4 sets the cargo jobs; results in neg87.log
"""
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ENV = dict(os.environ, CARGO_BUILD_JOBS=os.environ.get("NEG_JOBS", "4"), PATH=os.path.expanduser("~/.cargo/bin") + ":" + os.environ["PATH"])

# name: (file, old text, new text, cargo test arguments, extra env)
CASES = {
    "unstepped": (
        "crates/kiln-sim/src/lib.rs",
        "if p.sync_velocity && !p.sync_held && p.phantom_travels() {",
        "if false && p.sync_velocity && !p.sync_held && p.phantom_travels() {",
        "-p kiln-sim --test damage_knockback_wp87",
        {},
    ),
    "periodic": (
        "crates/kiln-loot/src/view.rs",
        "json.as_i32().is_some_and(|period| period != 0 && self.tick_count % period == 0)",
        "{ let _ = json; false }",
        "-p kiln-sim --test mob_enchant_fx_wp87 periodic",
        {},
    ),
    "mob_tick": (
        "crates/kiln-entity/src/mob/mod.rs",
        "    enchant_fx::tick(e, m, level);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 tick",
        {},
    ),
    "mob_location": (
        "crates/kiln-entity/src/mob/mod.rs",
        "        enchant_fx::block_changed(e, m, level);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 attribute_effect",
        {},
    ),
    "mob_equipment": (
        "crates/kiln-entity/src/mob/mod.rs",
        "    enchant_fx::equipment_changed(e, m, level);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 attribute_effect",
        {},
    ),
    "mob_landing": (
        "crates/kiln-entity/src/mob/mod.rs",
        "        enchant_fx::landed(e, m, level);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 replaces_the_block",
        {},
    ),
    "postattack_explode": (
        "crates/kiln-loot/src/view.rs",
        "EntityEffect::Explode { .. } => out.push(MobPostAttack::Explode { effect: EffectRef(std::sync::Arc::new(effect.clone())), level, on_victim }),",
        "EntityEffect::Explode { .. } => {}",
        "-p kiln-sim --test mob_enchant_fx_wp87 exploding_weapon",
        {},
    ),
    "hit_block_blocks": (
        "crates/kiln-entity/src/enchanting.rs",
        "level.emit(Event::BlockFx { effect: crate::level::BlockFx(std::sync::Arc::new(effect)), level: enchant_level, at });",
        "let _ = (effect, enchant_level, at);",
        "-p kiln-sim --test mob_enchant_fx_wp87 enchanted_bow",
        {},
    ),
    "player_explosions": (
        "crates/kiln-sim/src/entities.rs",
        "    player_explosions(&mut sim);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 players_tick_explosion",
        {},
    ),
    "immunity": (
        "crates/kiln-entity/src/mob/mod.rs",
        "if crate::enchanting::damage_immunity(",
        "if false && crate::enchanting::damage_immunity(",
        "-p kiln-sim --test mob_enchant_fx_wp87 damage_immunity",
        {"KILN_DATAPACK": os.path.expanduser("~/kiln/work/generated")},
    ),
    "magma": (
        "crates/kiln-entity/src/inside.rs",
        "                    self.apply_action(level, Action::HotFloor);\n",
        "",
        "-p kiln-sim --test mob_enchant_fx_wp87 damage_immunity",
        {"KILN_DATAPACK": os.path.expanduser("~/kiln/work/generated")},
    ),
    "cactus": (
        "crates/kiln-entity/src/inside.rs",
        "Kind::Cactus => self.apply_action(level, Action::CactusHurt),",
        "Kind::Cactus => {}",
        "-p kiln-sim --test mob_enchant_fx_wp87 cactus",
        {},
    ),
    "interacting": (
        "crates/kiln-sim/src/mobs.rs",
        "kiln_loot::EntityTarget::InteractingEntity => self.extra.interacting.is_some(),",
        "kiln_loot::EntityTarget::InteractingEntity => false,",
        "-p kiln-sim --test loot_calls interacting_entity",
        {},
    ),
    "barter_origin": (
        "crates/kiln-sim/src/mobs.rs",
        "(!self.extra.no_origin).then_some(self.origin)",
        "Some(self.origin)",
        "-p kiln-sim --test loot_calls barter",
        {},
    ),
    "clocks_run": (
        "crates/kiln-sim/src/weather.rs",
        "            c.total += c.run.tick();\n",
        "            let _ = &c;\n",
        "-p kiln-sim --test dp_clocks_wp87 runs_by_itself",
        {},
    ),
    "markers": (
        "crates/kiln-sim/src/timelines.rs",
        "self.of_clock(clock).flat_map(|(_, t)| t.markers.iter().map(move |m| (m.id.clone(), t.period, m.ticks, m.show_in_commands))).collect()",
        "{ let _ = clock; Vec::new() }",
        "-p kiln-sim --test dp_clocks_wp87 markers",
        {},
    ),
    "level_clock": (
        "crates/kiln-sim/src/weather.rs",
        "self.timelines.bool_attr(crate::dim_type(dim).timelines, attribute, base, &total)",
        "self.timelines.bool_attr(&[\"minecraft:day\"], attribute, base, &total)",
        "-p kiln-sim --test dp_clocks_wp87 level_of_the_pack",
        {},
    ),
    "clock_saved": (
        "crates/kiln-storage/src/level.rs",
        "    for (name, run) in running {\n",
        "    for (name, run) in running.iter().take(0) {\n",
        "-p kiln-sim --test dp_clocks_wp87 world_keeps_them",
        {},
    ),
    "quad_leash": (
        "crates/kiln-entity/src/leash.rs",
        "let quad = holder.quad && supports_quad_leash(e, m);",
        "let quad = false && holder.quad && supports_quad_leash(e, m);",
        "-p kiln-entity --test leash_quad_wp87",
        {},
    ),
    "ocean_ruin_save": (
        "crates/kiln-worldgen/src/structure/templated/ocean_ruin.rs",
        "        *self.settled.lock().unwrap() = Some(pos);
",
        "        let _ = pos;
",
        "-p kiln-worldgen --test wp87_order seed_84",
        {},
    ),
    "nether_sky": (
        "crates/kiln-sim/src/blocks.rs",
        "    if !crate::dim_type(env.dim).has_skylight {
        return 0;",
        "    if false && !crate::dim_type(env.dim).has_skylight {
        return 0;",
        "-p kiln-sim --test nether_sky_wp87",
        {},
    ),
    "flat_depth": (
        "crates/kiln-worldgen/src/generator.rs",
        "g.gen_min_y = g.min_y.max(0);",
        "g.gen_min_y = g.min_y;",
        "-p kiln-worldgen --test wp87_flat a_flat_level_with_features",
        {},
    ),
    "flat_structure_depth": (
        "crates/kiln-worldgen/src/structure/mod.rs",
        "if self.generator.is_flat() { 0 } else { self.generator.min_y }",
        "self.generator.min_y",
        "-p kiln-worldgen --test wp87_flat without_overrides",
        {},
    ),
    "flat_pipeline": (
        "crates/kiln-sim/src/lib.rs",
        "StemGenerator::Flat(flat) if flat.needs_pipeline() &&",
        "StemGenerator::Flat(flat) if false && flat.needs_pipeline() &&",
        "-p kiln-sim --test dp_flat_wp87",
        {},
    ),
    "flat_overrides": (
        "crates/kiln-worldgen/src/structure/mod.rs",
        ".filter(|&i| allowed.as_ref().is_none_or(|a| a.contains(&sets[i].name)))",
        ".filter(|&i| allowed.as_ref().is_none_or(|a| a.contains(&sets[i].name)) || true)",
        "-p kiln-worldgen --test wp87_flat structure_sets_it_names",
        {},
    ),
}


def run(name):
    path, old, new, args, extra = CASES[name]
    full = os.path.join(ROOT, path)
    text = open(full, encoding="utf-8").read()
    if old not in text:
        return f"{name}: SETUP BROKEN (the text to switch off is not in {path})"
    open(full, "w", encoding="utf-8").write(text.replace(old, new, 1))
    try:
        proc = subprocess.run(f"cargo test --release {args}", shell=True, cwd=ROOT, env=dict(ENV, **extra), capture_output=True, text=True)
        out = proc.stdout + proc.stderr
        failed = proc.returncode != 0 and "test result: FAILED" in out
        broke = proc.returncode != 0 and "test result: FAILED" not in out
        failing = [l.strip() for l in out.splitlines() if l.startswith("test ") and l.endswith("FAILED")]
        if broke:
            return f"{name}: DID NOT COMPILE / RUN\n" + "\n".join(out.splitlines()[-15:])
        return f"{name}: {'fails (good): ' + ', '.join(failing) if failed else 'STILL PASSES (bad)'}"
    finally:
        open(full, "w", encoding="utf-8").write(text)


if __name__ == "__main__":
    names = sys.argv[1:] or list(CASES)
    results = []
    for n in names:
        r = run(n)
        print(r, flush=True)
        results.append(r)
    open(os.path.join(ROOT, "neg87.log"), "w").write("\n".join(results) + "\n")
    bad = [r for r in results if "(bad)" in r or "BROKEN" in r or "DID NOT" in r]
    print(f"{len(results) - len(bad)} of {len(results)} as they should be")
    sys.exit(1 if bad else 0)
