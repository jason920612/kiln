#!/bin/bash
# wp83 negative checks (on the VM, in the wp83-datapacks checkout): switch off one piece of the change, run the test that covers it, and require
# the test to FAIL; then restore the file. usage: ~/kt wp83-datapacks bash tools/wp83/neg83.sh [name-filter]
# (the loot and enchantment parts have their own scripts, neg83_loot.sh and neg83_enchant.sh, run at the end.)
cd "$(dirname "$0")/../.."
export PATH="$HOME/.cargo/bin:$PATH" CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3} RUST_TEST_THREADS=${RUST_TEST_THREADS:-4}
export KILN_DATAPACK=${KILN_DATAPACK:-$HOME/kiln/work/generated}
fails=0
only="$1"
# negsed "<name>" "<cargo test args>" file 'sed expression': edit the file, run, require FAIL (not a build error), restore
negsed() {
  local name="$1" tests="$2" f="$3" expr="$4"
  if [ -n "$only" ] && [[ "$name" != *$only* ]]; then return; fi
  sed -i "$expr" "$f"
  if git diff --quiet -- "$f"; then echo "SED CHANGED NOTHING (bad): $name"; fails=$((fails + 1)); git checkout -- "$f"; return; fi
  if cargo test $tests > /tmp/neg83.out 2>&1; then
    echo "NOT FAILING (bad): $name"; fails=$((fails + 1))
  elif grep -q "^error" /tmp/neg83.out && ! grep -q "^test .* FAILED" /tmp/neg83.out; then
    echo "BUILD ERROR (bad): $name"; fails=$((fails + 1))
  else
    echo "fails without the change (good): $name"
  fi
  git checkout -- "$f"
}
DP="-p kiln-sim --test dp_registries"
S=crates/kiln-sim/src
negsed "registries are built from the packs" "$DP entries_are_numbered" $S/datapacks.rs '/kiln_data::live::install_synced(loaded.names, loaded.data);/d'
negsed "paintings follow the registry numbering" "$DP paintings_take" $S/datapacks.rs '/kiln_data::live::install_synced(loaded.names, loaded.data);/d'
negsed "jukebox songs are registry entries" "$DP a_disc_of_a_song" $S/datapacks.rs '/kiln_data::live::install_synced(loaded.names, loaded.data);/d'
negsed "dialogs are registry entries" "$DP dialogs_of_the_pack" $S/datapacks.rs '/kiln_data::live::install_synced(loaded.names, loaded.data);/d'
negsed "trim patterns are registry entries" "$DP trims_of_the_pack" $S/datapacks.rs '/kiln_data::live::install_synced(loaded.names, loaded.data);/d'
negsed "clients get the data of the pack's entries" "$DP clients_get_the_data" $S/datapacks.rs 's|sync.set_registry_packets((!loaded.packets.is_empty()).then(.*|sync.set_registry_packets(None);|'
negsed "registries sorted by path then namespace" "-p kiln-sim --lib registry_data" $S/registry_data.rs 's|split(a).cmp(&split(b))|split(a).1.cmp(\&split(b).1).then(split(a).0.cmp(\&split(b).0))|'
negsed "tags reach the block logic" "$DP the_tags_of_the_packs" crates/kiln-blocks/src/tags.rs 's|kiln_data::live::tags_of("minecraft:block")|kiln_data::registries::TAGS.iter().find(\|(r, _)\| *r == "minecraft:block").map_or(\&[][..], \|(_, t)\| *t)|'
negsed "a shovel mines what the pack tags" "$DP a_shovel" crates/kiln-blocks/src/tags.rs 's|kiln_data::live::tags_of("minecraft:block")|kiln_data::registries::TAGS.iter().find(\|(r, _)\| *r == "minecraft:block").map_or(\&[][..], \|(_, t)\| *t)|'
negsed "tags are installed for the game logic" "$DP reload_rereads" $S/datapacks.rs '/kiln_data::live::install_tags(tags.clone());/d'
negsed "tags of the packs reach the inventory crate" "$DP the_tags_of_the_packs" crates/kiln-inventory/src/tags.rs 's|for (registry, tags) in kiln_data::live::tags() {|for (registry, tags) in kiln_data::registries::TAGS {|'
negsed "animal variants follow spawn conditions" "$DP animals_spawn" crates/kiln-entity/src/mob/variants.rs 's|^    select(registry, &Where { biome: ctx.biome.*|    let _ = (registry, ctx, group, r);\n    None|'
negsed "variant selection picks the best priority" "-p kiln-entity --lib variants" crates/kiln-entity/src/mob/variants.rs 's|        if priority < best {|        if priority > best {|'
negsed "paintings are placed from the placeable tag" "$DP paintings_take" crates/kiln-entity/src/ext_entity/painting_variants.rs 's|kiln_data::live::tag("minecraft:painting_variant", "minecraft:placeable").unwrap_or(&\[\])|\&[]|'
negsed "painting sizes come from the entries" "$DP paintings_take" crates/kiln-entity/src/ext_entity/painting_variants.rs 's|let packed = kiln_data::live::entry_data("minecraft:painting_variant", name).and_then(|let packed = None::<\&str>.and_then(|'
negsed "damage types come from the entries" "$DP damage_types_of_the_pack" $S/health.rs 's|for entry in kiln_data::live::custom_entries("minecraft:damage_type") {|for entry in Vec::<\&'"'"'static str>::new() {|'
negsed "goat horns play the instrument's own sound and length" "$DP a_goat_horn" $S/ranged.rs 's|kiln_data::live::entry_data("minecraft:instrument", name).and_then(|None::<\&str>.and_then(|'
negsed "looms offer the pack's patterns" "$DP looms_offer" crates/kiln-inventory/src/stations.rs 's|        if kiln_data::live::is_custom()|        if false \&\& kiln_data::live::is_custom()|'
negsed "sound variants of packs" "$DP sound_variants" crates/kiln-entity/src/mob/variants.rs 's|    let json = Json::parse(kiln_data::live::entry_data(registry, name)?).ok()?;|    let json = Json::parse("{}").ok()?;\n    let _ = name;|'
negsed "item defaults follow the registries" "$DP the_default_components" crates/kiln-item/src/defaults.rs '/^                c.renumber();$/d'
negsed "lenient tags (a bad tag does not stop the others)" "-p kiln-sim --lib tags::tests::later_packs" crates/kiln-loot/src/tags.rs 's|        errors.push(format!("{}/{}: missing references {}", key.0, key.1, missing.join(",")));|        errors.push(format!("{}/{}: missing references {}", key.0, key.1, missing.join(",")));\n        return Some(out);|'
negsed "live tables replace the static ones" "-p kiln-data --lib installed_tags" crates/kiln-data/src/live.rs 's|^    publish(Tables::new(old.synced, tags, old.data.clone(), old.synced_custom, custom), false);|    let _ = (tags, custom);|'
negsed "the vanilla tables are the default" "-p kiln-data --lib static_tables" crates/kiln-data/src/live.rs 's|    assert_eq!(synced().len(), registries::SYNCHRONIZED.len());|    assert_eq!(synced().len(), 0);|'
# vanilla's own answer: the Java check must fail when the numbering is wrong
if [ -z "$only" ] || [[ "numbering against vanilla" == *$only* ]]; then
  sed -i 's|split(a).cmp(&split(b))|split(a).1.cmp(\&split(b).1).then(split(a).0.cmp(\&split(b).0))|' $S/registry_data.rs
  if bash tools/wp83/check_registries.sh > /tmp/neg83-java.out 2>&1; then echo "NOT FAILING (bad): numbering against vanilla"; fails=$((fails + 1)); else echo "fails without the change (good): numbering against vanilla (CheckRegistries.java)"; fi
  git checkout -- $S/registry_data.rs
fi
[ -z "$only" ] && for s in tools/wp83/neg83_loot.sh tools/wp83/neg83_enchant.sh; do
  if [ -f "$s" ]; then bash "$s" || fails=$((fails + 1)); fi
done
echo "negative checks failing the wrong way: $fails"
exit $fails
