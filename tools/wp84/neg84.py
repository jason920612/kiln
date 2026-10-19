#!/usr/bin/env python3
"""wp84 negative checks (run on the VM from the checkout of the branch: `~/kt wp84-playthrough11 python3 tools/wp84/neg84.py [name ...]`).
Each fix is switched off by a text edit; the test that belongs to it must FAIL; the file is put back (git checkout) afterwards.
Prints one line per case: `FAILS (good)` / `PASSES (bad: the test does not see the fix)`."""
import subprocess
import sys

CASES = [
    # name, file, old text, new text, cargo test arguments
    ("ghast: an adult takes the snowball", "crates/kiln-entity/src/mob/kinds/happy_ghast.rs",
     "        Some(Outcome::PASS)\n    }\n\n    /// Whether a player riding first steers it",
     "        Some(interact::animal_interact(e, m, level, who, stack))\n    }\n\n    /// Whether a player riding first steers it",
     "-p kiln-sim --test happy_ghast_wp84 a_grown_happy_ghast_does_not_take_the_snowball"),
    ("ghast: a loaded ghastling is set up as a grown one", "crates/kiln-entity/src/mob/kinds/happy_ghast.rs",
     "        if m.baby() {\n            self.baby_setup(m);\n        }\n        let still",
     "        let still",
     "-p kiln-sim --test happy_ghast_wp84 a_ghastling_from_saved_data_follows_a_snowball_like_a_baby"),
    ("ghast: no goggles sound", "crates/kiln-entity/src/mob/kinds/happy_ghast.rs",
     "if before == 0 {\n                    e.play_sound(level, \"minecraft:entity.happy_ghast.harness_goggles_down\"",
     "if false {\n                    e.play_sound(level, \"minecraft:entity.happy_ghast.harness_goggles_down\"",
     "-p kiln-sim --test happy_ghast_wp84 a_harnessed_happy_ghast_plays_its_goggles"),
    ("ghast: the lead snaps at 12", "crates/kiln-entity/src/leash.rs",
     "if matches!(e.type_name, \"minecraft:happy_ghast\" | \"minecraft:ghast\") { 16.0 } else { SNAP_DISTANCE }",
     "SNAP_DISTANCE",
     "-p kiln-sim --test happy_ghast_wp84 a_leashed_happy_ghast_keeps_its_lead"),
    ("consume: the play_sound effect is not heard by the drinker", "crates/kiln-sim/src/consume.rs",
     "self.queue_sound_for_all_at(at, name, kiln_proto::packets::world_fx::SoundSource::Players, 1.0, 1.0);",
     "let _ = at; self.queue_sound(name, 1.0, 1.0);",
     "-p kiln-sim --test ominous_bottle_sound"),
    ("knockback: the packet is not delayed one tick", "crates/kiln-sim/src/players.rs",
     "if target.sync_velocity && target.phantom_travels() && !target.sync_deferred {",
     "if false {",
     "-p kiln-sim --test player_knockback_wp84 a_zombie_hit_reaches_the_client_after_one_step_of_travel"),
    ("pot: the comparator is not updated", "crates/kiln-sim/src/decorated_pot.rs",
     "    kiln_blocks::update::update_neighbour_for_output_signal(level, pos, BlockId::of(s));\n    level.effect(Effect::Sound { pos, sound: \"minecraft:block.decorated_pot.insert\"",
     "    level.effect(Effect::Sound { pos, sound: \"minecraft:block.decorated_pot.insert\"",
     "-p kiln-sim --test pot_comparator"),
    ("matrix: the undead mounts take no lead", "crates/kiln-entity/src/leash.rs",
     "        // Not `Enemy` in vanilla (`Mob.canBeLeashed`): the undead mounts.\n        ZombieHorse | CamelHusk | ZombieNautilus => true,\n",
     "",
     "-p kiln-sim --test interact_wp84 a_lead_goes_on_the_undead_mounts"),
    ("matrix: a wild skeleton horse takes no saddle", "crates/kiln-entity/src/mob/kinds/horse.rs",
     "            if st(m).saddle.is_empty()\n                && crate::mob::is_alive(e, m)\n                && let Some(one)",
     "            if false\n                && crate::mob::is_alive(e, m)\n                && let Some(one)",
     "-p kiln-sim --test interact_wp84 a_saddle_goes_on_an_untamed_skeleton_horse"),
    ("matrix: the allay sound goes to the giver too", "crates/kiln-entity/src/mob/kinds/allay.rs",
     "Event::EntitySoundExcept { except: who_id, entity: e.id,",
     "Event::EntitySound { entity: e.id,",
     "-p kiln-sim --test interact_wp84 the_allay_item_given_sound"),
    ("matrix: no explosion particle for a sheared mooshroom", "crates/kiln-entity/src/mob/kinds/mooshroom.rs",
     "                level.particle(\"minecraft:explosion\", Vec3::new(pos.x, pos.y + height * 0.5, pos.z));\n",
     "",
     "-p kiln-sim --test interact_wp84 a_sheared_mooshroom"),
    ("advancement: a predicate reference as the player condition", "crates/kiln-sim/src/advancements/criteria.rs",
     "Json::Arr(_) | Json::Str(_) => Condition::parse_list(p, j),",
     "Json::Arr(_) => Condition::parse_list(p, j),",
     "-p kiln-sim --test advancement_reference"),
    ("knockback: the server body does not travel", "crates/kiln-sim/src/phantom.rs",
     "        if !self.phantom_travels() {\n            return;\n        }",
     "        if true {\n            return;\n        }",
     "-p kiln-sim --test player_knockback_wp84"),
]


def run(args):
    return subprocess.run(["cargo", "test", "--release"] + args.split(), capture_output=True, text=True)


def main():
    only = sys.argv[1:]
    bad = 0
    for name, path, old, new, args in CASES:
        if only and not any(o in name for o in only):
            continue
        text = open(path, encoding="utf-8").read()
        if old not in text:
            print(f"{name}: PATTERN NOT FOUND in {path}")
            bad += 1
            continue
        open(path, "w", encoding="utf-8", newline="\n").write(text.replace(old, new, 1))
        try:
            r = run(args)
        finally:
            subprocess.run(["git", "checkout", "--", path])
        failed = r.returncode != 0 and "test result: FAILED" in r.stdout
        built = "could not compile" not in r.stderr
        verdict = "FAILS (good)" if failed and built else ("DOES NOT COMPILE" if not built else "PASSES (bad: the test does not see the fix)")
        bad += 0 if failed and built else 1
        print(f"{name}: {verdict}", flush=True)
    sys.exit(1 if bad else 0)


main()
