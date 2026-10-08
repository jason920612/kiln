// Differential test vectors for Kiln's container block entities: hoppers moving items, furnaces
// cooking, and comparators reading containers. Runs scenarios in a real vanilla 26.3 dedicated
// server (started in-process): each scenario places its blocks with /setblock (block entity
// data included), then runs whole level ticks (ServerLevel.tick, so scheduled ticks, block
// events and block entity tickers run in vanilla's order) and records after every tick the
// contents of the watched containers (with hopper cooldowns and furnace timers), the watched
// block states and comparator outputs. Commands can run before chosen ticks.
//
// Block entities tick in the order they were created; scenarios place their blocks sorted by
// position, which is the order Kiln ticks them in.
//
// usage (cwd = a scratch server directory, e.g. work/wp15-containers/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/ContainerVectors.java <out.jsonl> [name-filter]
// (tools/container_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.AbstractFurnaceBlockEntity;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ComparatorBlockEntity;
import net.minecraft.world.level.block.entity.HopperBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

public class ContainerVectors {
    static final int[] BASE = {0, 100, 0};

    // ---------------------------------------------------------------- scenario model

    static final class Scenario {
        final String name;
        int ticks = 40;
        // Blocks relative to BASE: {dx, dy, dz, block argument of /setblock}.
        List<Object[]> blocks = new ArrayList<>();
        // Commands (with relative coordinates written as ~x~ ~y~ ~z~ placeholders) before a tick.
        TreeMap<Integer, List<String>> actions = new TreeMap<>();
        List<int[]> containers = new ArrayList<>();
        List<int[]> states = new ArrayList<>();
        List<int[]> comparators = new ArrayList<>();
        /** Container minecarts watched by the block they stand in. */
        List<int[]> carts = new ArrayList<>();
        /** The item entities lying about (by item, with the sum of their counts) are recorded every tick. */
        boolean watchDrops;
        /** The game time is a multiple of 20 when the first tick begins (daylight detectors work on it). */
        boolean align20;

        Scenario(String name, int ticks) {
            this.name = name;
            this.ticks = ticks;
        }

        Scenario block(int x, int y, int z, String block) {
            blocks.add(new Object[] {x, y, z, block});
            return this;
        }

        /** A container block, watched. */
        Scenario container(int x, int y, int z, String block) {
            containers.add(new int[] {x, y, z});
            return block(x, y, z, block);
        }

        Scenario state(int x, int y, int z) {
            states.add(new int[] {x, y, z});
            return this;
        }

        /**
         * A chest or hopper minecart standing in that block, watched. It is summoned at the first
         * tick without gravity, at the middle of the block (`bottom` above the block's floor),
         * with the saved data `nbt`.
         */
        Scenario cart(int x, int y, int z, String type, double bottom, String nbt) {
            carts.add(new int[] {x, y, z});
            // `nbt` is a compound ("{Items:[...]}") or empty.
            String body = nbt.startsWith("{") ? nbt.substring(1, nbt.length() - 1) : nbt;
            return at(1, String.format(Locale.ROOT, "summon minecraft:%s %s %s %s {NoGravity:1b%s}", type, BASE[0] + x + 0.5,
                    BASE[1] + y + bottom, BASE[2] + z + 0.5, body.isEmpty() ? "" : "," + body));
        }

        /** A comparator reading toward `facing` from the block behind it, on a stone block. */
        Scenario comparator(int x, int y, int z, String facing) {
            comparators.add(new int[] {x, y, z});
            block(x, y - 1, z, "minecraft:stone");
            return block(x, y, z, "minecraft:comparator[facing=" + facing + "]");
        }

        Scenario align20() {
            align20 = true;
            return this;
        }

        Scenario drops() {
            watchDrops = true;
            return this;
        }

        Scenario at(int tick, String command) {
            actions.computeIfAbsent(tick, k -> new ArrayList<>()).add(command);
            return this;
        }

        Map<String, Object> json() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", name);
            m.put("ticks", ticks);
            List<Object> bs = new ArrayList<>();
            for (Object[] b : sortedBlocks()) bs.add(List.of(b[0], b[1], b[2], b[3]));
            m.put("blocks", bs);
            Map<String, Object> acts = new LinkedHashMap<>();
            for (var e : actions.entrySet()) acts.put(String.valueOf(e.getKey()), e.getValue());
            m.put("actions", acts);
            m.put("containers", positions(containers));
            m.put("states", positions(states));
            m.put("comparators", positions(comparators));
            m.put("carts", positions(carts));
            m.put("drops", watchDrops);
            m.put("align20", align20);
            return m;
        }

        /** Blocks in position order (x, then y, then z): block entities tick in this order. */
        List<Object[]> sortedBlocks() {
            List<Object[]> out = new ArrayList<>(blocks);
            out.sort(Comparator.<Object[]>comparingInt(b -> (Integer) b[0]).thenComparingInt(b -> (Integer) b[1])
                    .thenComparingInt(b -> (Integer) b[2]));
            return out;
        }
    }

    static List<Object> positions(List<int[]> ps) {
        List<Object> out = new ArrayList<>();
        for (int[] p : ps) out.add(List.of(p[0], p[1], p[2]));
        return out;
    }

    static String items(String... entries) {
        StringBuilder b = new StringBuilder("{Items:[");
        for (int i = 0; i < entries.length; i++) b.append(i > 0 ? "," : "").append(entries[i]);
        return b.append("]}").toString();
    }

    static String slot(int slot, String id, int count) {
        return String.format(Locale.ROOT, "{Slot:%db,id:\"minecraft:%s\",count:%d}", slot, id, count);
    }

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        // ---- hoppers
        out.add(new Scenario("hopper_chain_down", 90)
                .container(0, 3, 0, "minecraft:chest" + items(slot(0, "oak_log", 5), slot(13, "dirt", 2)))
                .container(0, 2, 0, "minecraft:hopper[facing=down]")
                .container(0, 1, 0, "minecraft:hopper[facing=down]")
                .container(0, 0, 0, "minecraft:chest"));
        out.add(new Scenario("hopper_line_east", 80)
                .container(0, 1, 0, "minecraft:chest" + items(slot(0, "cobblestone", 7)))
                .container(0, 0, 0, "minecraft:hopper[facing=east]")
                .container(1, 0, 0, "minecraft:hopper[facing=east]")
                .container(2, 0, 0, "minecraft:hopper[facing=east]")
                .container(3, 0, 0, "minecraft:hopper[facing=east]")
                .container(4, 0, 0, "minecraft:barrel[facing=up]"));
        out.add(new Scenario("hopper_line_west", 80)
                .container(4, 1, 0, "minecraft:chest" + items(slot(0, "cobblestone", 7)))
                .container(4, 0, 0, "minecraft:hopper[facing=west]")
                .container(3, 0, 0, "minecraft:hopper[facing=west]")
                .container(2, 0, 0, "minecraft:hopper[facing=west]")
                .container(1, 0, 0, "minecraft:hopper[facing=west]")
                .container(0, 0, 0, "minecraft:barrel[facing=up]"));
        out.add(new Scenario("hopper_fills_a_nearly_full_chest", 60)
                .container(0, 2, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 10), slot(2, "sand", 3)))
                .container(0, 1, 0, "minecraft:chest" + items(fullChest("stone", 64, 26, 60))));
        out.add(new Scenario("hopper_feeds_a_furnace", 260)
                .container(0, 2, 0, "minecraft:hopper[facing=down]" + items(slot(0, "raw_iron", 3)))
                .container(-1, 1, 0, "minecraft:hopper[facing=east]" + items(slot(0, "coal", 2)))
                .container(0, 1, 0, "minecraft:furnace[facing=north]")
                .container(0, 0, 0, "minecraft:hopper[facing=down]")
                .state(0, 1, 0));
        out.add(new Scenario("hopper_from_double_chest", 70)
                .container(0, 1, 0, "minecraft:chest[facing=east,type=left]" + items(slot(26, "gold_ingot", 2)))
                .container(0, 1, 1, "minecraft:chest[facing=east,type=right]" + items(slot(0, "iron_ingot", 2), slot(5, "iron_ingot", 1)))
                .container(0, 0, 0, "minecraft:hopper[facing=down]"));
        out.add(new Scenario("hopper_locked_by_redstone", 60)
                .container(0, 2, 0, "minecraft:chest" + items(slot(0, "glass", 6)))
                .container(0, 1, 0, "minecraft:hopper[facing=down]")
                .container(0, 0, 0, "minecraft:chest")
                .state(0, 1, 0)
                .at(12, "setblock ~1 ~1 ~0 minecraft:redstone_block")
                .at(40, "setblock ~1 ~1 ~0 minecraft:air"));
        out.add(new Scenario("hopper_into_shulker_box", 40)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "white_shulker_box", 1), slot(1, "stick", 3)))
                .container(0, 0, 0, "minecraft:shulker_box[facing=up]"));
        out.add(new Scenario("hopper_pulls_furnace_output", 30)
                .container(0, 1, 0, "minecraft:furnace[facing=north]" + items(slot(1, "bucket", 1), slot(2, "iron_ingot", 3)))
                .container(0, 0, 0, "minecraft:hopper[facing=down]"));
        // ---- furnaces
        out.add(new Scenario("furnace_iron_with_coal", 700)
                .container(0, 0, 0, "minecraft:furnace[facing=north]" + items(slot(0, "raw_iron", 3), slot(1, "coal", 1)))
                .state(0, 0, 0));
        out.add(new Scenario("smoker_beef", 260)
                .container(0, 0, 0, "minecraft:smoker[facing=north]" + items(slot(0, "beef", 2), slot(1, "coal", 1)))
                .state(0, 0, 0));
        out.add(new Scenario("blast_furnace_gold_planks", 330)
                .container(0, 0, 0, "minecraft:blast_furnace[facing=north]" + items(slot(0, "raw_gold", 3), slot(1, "oak_planks", 2)))
                .state(0, 0, 0));
        out.add(new Scenario("furnace_runs_out_of_fuel", 260)
                .container(0, 0, 0, "minecraft:furnace[facing=north]" + items(slot(0, "potato", 2), slot(1, "stick", 1)))
                .state(0, 0, 0));
        out.add(new Scenario("furnace_lava_bucket", 220)
                .container(0, 0, 0, "minecraft:furnace[facing=north]" + items(slot(0, "cobblestone", 1), slot(1, "lava_bucket", 1)))
                .state(0, 0, 0));
        out.add(new Scenario("furnace_full_output", 60)
                .container(0, 0, 0, "minecraft:furnace[facing=north]" + items(slot(0, "sand", 4), slot(1, "coal", 1), slot(2, "glass", 64)))
                .state(0, 0, 0));
        out.add(new Scenario("furnace_wet_sponge_fills_bucket", 230)
                .container(0, 0, 0, "minecraft:furnace[facing=north]" + items(slot(0, "wet_sponge", 1), slot(1, "coal", 1)))
                .container(-1, 0, 0, "minecraft:hopper[facing=east]" + items(slot(0, "bucket", 1)))
                .state(0, 0, 0));
        // ---- comparators
        comparator(out, "comparator_chest_one_item", "minecraft:chest" + items(slot(0, "stone", 1)));
        comparator(out, "comparator_chest_half", "minecraft:chest" + items(fullChest("stone", 64, 13, 0)));
        comparator(out, "comparator_chest_full", "minecraft:chest" + items(fullChest("stone", 64, 27, 0)));
        comparator(out, "comparator_chest_pearls", "minecraft:chest" + items(slot(0, "ender_pearl", 16), slot(1, "ender_pearl", 8)));
        comparator(out, "comparator_chest_swords", "minecraft:chest" + items(slot(0, "iron_sword", 1), slot(1, "iron_sword", 1), slot(2, "iron_sword", 1)));
        comparator(out, "comparator_hopper", "minecraft:hopper[facing=down,enabled=false]" + items(slot(0, "stone", 64), slot(1, "stone", 1)));
        comparator(out, "comparator_furnace", "minecraft:furnace[facing=south]" + items(slot(2, "iron_ingot", 64)));
        comparator(out, "comparator_dispenser", "minecraft:dispenser[facing=up]" + items(slot(4, "arrow", 40)));
        comparator(out, "comparator_barrel", "minecraft:barrel[facing=up]" + items(slot(8, "dirt", 50), slot(9, "dirt", 50)));
        comparator(out, "comparator_shulker", "minecraft:shulker_box[facing=up]" + items(slot(0, "stone", 64), slot(1, "stone", 64)));
        // Comparators are placed before the containers (position order), which then update them.
        out.add(new Scenario("comparator_double_chest", 8)
                .container(0, 0, 1, "minecraft:chest[facing=north,type=left]" + items(slot(0, "stone", 64), slot(1, "stone", 64)))
                .container(1, 0, 1, "minecraft:chest[facing=north,type=right]" + items(slot(0, "stone", 64)))
                .comparator(0, 0, 0, "south"));
        out.add(new Scenario("comparator_blocked_chest", 8)
                .container(0, 0, 1, "minecraft:chest" + items(slot(0, "stone", 64)))
                .block(0, 1, 1, "minecraft:stone")
                .comparator(0, 0, 0, "south"));
        out.add(new Scenario("comparator_chest_unblocked", 12)
                .container(0, 0, 1, "minecraft:chest" + items(slot(0, "stone", 64)))
                .block(0, 1, 1, "minecraft:stone")
                .comparator(0, 0, 0, "south")
                .at(4, "setblock ~0 ~1 ~1 minecraft:air"));
        out.add(new Scenario("comparator_follows_hopper", 60)
                .container(0, 1, 0, "minecraft:chest" + items(slot(0, "stone", 20)))
                .container(0, 0, 0, "minecraft:hopper[facing=down]")
                .container(0, -1, 0, "minecraft:chest")
                .comparator(0, -1, 1, "north"));
        cartScenarios(out);
        jukeboxScenarios(out);
        campfireScenarios(out);
        daylightScenarios(out);
        return out;
    }

    /** wp36: jukeboxes: songs ending by their length, comparators, redstone power, hoppers and dispensers. */
    static void jukeboxScenarios(List<Scenario> out) {
        // The jukebox at (0, 0, 1) is read by a comparator at (0, 0, 0) and powers a lamp at (-1, 0, 1)
        // (placed before it: the jukebox's placement tells the lamp).
        String disc = "{RecordItem:{id:\"minecraft:music_disc_%s\",count:1},ticks_since_song_started:%dL}";
        String lamp = "minecraft:redstone_lamp";
        // A song near its end finishes by itself: the disc stays, the power and the music go.
        out.add(new Scenario("jukebox_song_ends", 50)
                .block(-1, 0, 1, lamp).state(-1, 0, 1)
                .container(0, 0, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "11", 1400))
                .state(0, 0, 1)
                .comparator(0, 0, 0, "south"));
        // A song with plenty left: playing, powering, ticking.
        out.add(new Scenario("jukebox_song_plays", 30)
                .block(-1, 0, 1, lamp).state(-1, 0, 1)
                .container(0, 0, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "13", 100))
                .state(0, 0, 1)
                .comparator(0, 0, 0, "south"));
        // A song that is already over when the block entity loads is not started.
        out.add(new Scenario("jukebox_song_over_at_load", 20)
                .block(-1, 0, 1, lamp).state(-1, 0, 1)
                .container(0, 0, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "11", 1500))
                .state(0, 0, 1)
                .comparator(0, 0, 0, "south"));
        // The comparator reads the disc's song.
        for (String d : new String[] {"5", "cat", "pigstep", "otherside"}) {
            out.add(new Scenario("jukebox_comparator_" + d, 8)
                    .container(0, 0, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, d, 0))
                    .comparator(0, 0, 0, "south"));
        }
        // A hopper above puts a disc in: it starts playing at once (and, powered by the jukebox, locks).
        out.add(new Scenario("jukebox_hopper_inserts_disc", 40)
                .container(0, 1, 1, "minecraft:hopper[facing=down]" + items(slot(0, "stick", 2), slot(1, "music_disc_13", 1)))
                .state(0, 1, 1)
                .container(0, 0, 1, "minecraft:jukebox")
                .block(1, 0, 1, lamp).state(1, 0, 1).state(0, 0, 1)
                .comparator(0, 0, 0, "south"));
        // A hopper facing into the side of a jukebox that holds a (finished) disc leaves its disc be.
        out.add(new Scenario("jukebox_full_refuses_a_second_disc", 40)
                .container(0, 0, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "13", 3700))
                .container(-1, 0, 1, "minecraft:hopper[facing=east]" + items(slot(0, "music_disc_5", 1)))
                .state(0, 0, 1));
        // A hopper below takes the disc out (only where it has room): has_record goes, the comparator follows.
        out.add(new Scenario("jukebox_hopper_takes_disc", 40)
                .container(0, 1, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "13", 3700))
                .container(0, 0, 1, "minecraft:hopper[facing=down]")
                .container(0, -1, 1, "minecraft:chest")
                .state(0, 1, 1)
                .comparator(0, 1, 0, "south"));
        // While a song plays the jukebox powers the hopper under it: it takes nothing (locked).
        out.add(new Scenario("jukebox_playing_locks_the_hopper_under_it", 30)
                .container(0, 1, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "13", 100))
                .container(0, 0, 1, "minecraft:hopper[facing=down]")
                .state(0, 0, 1).state(0, 1, 1));
        // A full hopper cannot take it.
        out.add(new Scenario("jukebox_full_hopper_takes_nothing", 30)
                .container(0, 1, 1, "minecraft:jukebox[has_record=true]" + String.format(Locale.ROOT, disc, "13", 3700))
                .container(0, 0, 1, "minecraft:hopper[facing=down]"
                        + items(slot(0, "stone", 64), slot(1, "stone", 64), slot(2, "stone", 64), slot(3, "stone", 64), slot(4, "stone", 64)))
                .state(0, 1, 1));
        // A dispenser puts its disc in (the first slot with an item, as it always does).
        out.add(new Scenario("jukebox_dispenser_inserts_disc", 30)
                .container(-1, 0, 1, "minecraft:dispenser[facing=east]" + items(slot(3, "music_disc_cat", 1)))
                .container(0, 0, 1, "minecraft:jukebox")
                .block(1, 0, 1, lamp).state(1, 0, 1).state(0, 0, 1)
                .comparator(0, 0, 0, "south")
                .at(5, "setblock ~-1 ~1 ~1 minecraft:redstone_block"));
        out.add(new Scenario("jukebox_empty_comparator", 8)
                .container(0, 0, 1, "minecraft:jukebox")
                .comparator(0, 0, 0, "south"));
    }

    /** wp49: daylight detectors through a day, under a roof, inverted, in the rain. */
    static void daylightScenarios(List<Scenario> out) {
        String sensor = "minecraft:daylight_detector[inverted=false,power=0]";
        String inverted = "minecraft:daylight_detector[inverted=true,power=0]";
        Scenario s = new Scenario("daylight_day", 500).align20()
                .block(0, 0, 0, sensor).state(0, 0, 0).block(2, 0, 0, inverted).state(2, 0, 0);
        for (int i = 0; i < 25; i++) s.at(1 + 20 * i, "time set " + (i * 1000));
        out.add(s);
        // Between the hours too, a tick short of each sample.
        s = new Scenario("daylight_dawn_dusk", 300).align20()
                .block(0, 0, 0, sensor).state(0, 0, 0).block(2, 0, 0, inverted).state(2, 0, 0);
        int[] times = {23000, 23400, 23800, 100, 200, 400, 600, 11400, 11800, 12200, 12600, 13000, 13400, 13800, 14200};
        for (int i = 0; i < times.length; i++) s.at(1 + 20 * i, "time set " + times[i]);
        out.add(s);
        // A roof: no sky light under it.
        s = new Scenario("daylight_roof", 100).align20()
                .block(-1, 1, -1, "minecraft:stone").block(0, 1, 0, "minecraft:stone").block(1, 1, 1, "minecraft:stone")
                .block(0, 0, 0, sensor).state(0, 0, 0).block(2, 0, 0, inverted).state(2, 0, 0)
                .at(1, "time set 6000");
        out.add(s);
        // Rain and thunder darken the sky.
        s = new Scenario("daylight_rain", 300).align20()
                .block(0, 0, 0, sensor).state(0, 0, 0).block(2, 0, 0, inverted).state(2, 0, 0)
                .at(1, "time set 6000").at(1, "weather rain 100000").at(160, "weather thunder 100000");
        out.add(s);
    }

    /** wp49: campfires cooking food, cooling down, going out, dropping what they hold. */
    static void campfireScenarios(List<Scenario> out) {
        // Four foods on a lit fire, each with its own time left; what is done drops as the recipe's result.
        String cf = "minecraft:campfire[facing=north,lit=true,waterlogged=false,signal_fire=false]";
        String four = "{Items:[{Slot:0b,id:\"minecraft:beef\",count:1},{Slot:1b,id:\"minecraft:potato\",count:1},{Slot:2b,id:\"minecraft:porkchop\",count:1},{Slot:3b,id:\"minecraft:kelp\",count:1}],"
                + "CookingTimes:[I;0,5,10,15],CookingTotalTimes:[I;30,30,30,30]}";
        out.add(new Scenario("campfire_cooks_four", 45).container(0, 0, 0, cf + four).state(0, 0, 0).drops());
        // A food without a campfire recipe comes back out as it is.
        out.add(new Scenario("campfire_unknown_food_comes_back", 20).container(0, 0, 0, cf
                + "{Items:[{Slot:0b,id:\"minecraft:stone\",count:1}],CookingTimes:[I;8,0,0,0],CookingTotalTimes:[I;10,0,0,0]}").drops());
        // An unlit campfire lets the cooking go back two ticks at a time (never below nothing).
        out.add(new Scenario("campfire_unlit_cools_down", 14).container(0, 0, 0, "minecraft:campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]"
                + "{Items:[{Slot:0b,id:\"minecraft:beef\",count:1},{Slot:2b,id:\"minecraft:potato\",count:1}],CookingTimes:[I;9,0,4,0],CookingTotalTimes:[I;100,100,100,100]}"));
        // Lit, then put out half way, then the same food on a lit fire again.
        out.add(new Scenario("campfire_put_out_and_relit", 40).container(0, 0, 0, cf
                + "{Items:[{Slot:1b,id:\"minecraft:chicken\",count:1}],CookingTimes:[I;0,0,0,0],CookingTotalTimes:[I;0,30,0,0]}").state(0, 0, 0)
                .at(10, "setblock ~0 ~0 ~0 minecraft:campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]{Items:[{Slot:1b,id:\"minecraft:chicken\",count:1}],CookingTimes:[I;0,10,0,0],CookingTotalTimes:[I;0,30,0,0]}")
                .at(22, "setblock ~0 ~0 ~0 minecraft:campfire[facing=north,lit=true,waterlogged=false,signal_fire=false]{Items:[{Slot:1b,id:\"minecraft:chicken\",count:1}],CookingTimes:[I;0,20,0,0],CookingTotalTimes:[I;0,30,0,0]}")
                .drops());
        // Soul campfires cook too.
        out.add(new Scenario("soul_campfire_cooks", 20).container(0, 0, 0, "minecraft:soul_campfire[facing=east,lit=true,waterlogged=false,signal_fire=false]"
                + "{Items:[{Slot:3b,id:\"minecraft:cod\",count:1}],CookingTimes:[I;0,0,0,15],CookingTotalTimes:[I;0,0,0,18]}").drops());
        // A campfire that is broken drops its food.
        out.add(new Scenario("campfire_broken_drops_food", 10).container(0, 0, 0, cf
                + "{Items:[{Slot:0b,id:\"minecraft:salmon\",count:1},{Slot:2b,id:\"minecraft:mutton\",count:1}],CookingTimes:[I;1,1,1,1],CookingTotalTimes:[I;600,600,600,600]}")
                .at(3, "setblock ~0 ~0 ~0 minecraft:air").drops());
    }

    /** Hoppers and container minecarts exchanging items (the minecarts hang in the air, at rest). */
    static void cartScenarios(List<Scenario> out) {
        // A hopper block pulls from a chest minecart in the block above and feeds a chest.
        out.add(new Scenario("cart_hopper_block_pulls_from_chest_minecart", 60)
                .container(0, 0, 0, "minecraft:hopper[facing=east]")
                .container(1, 0, 0, "minecraft:chest")
                .cart(0, 1, 0, "chest_minecart", 0.0, items(slot(0, "apple", 3), slot(9, "coal", 40), slot(26, "stick", 5))));
        // A hopper block pushes into a hopper minecart in the block it faces.
        out.add(new Scenario("cart_hopper_block_fills_hopper_minecart", 60)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 30), slot(2, "sand", 3)))
                .cart(0, 0, 0, "hopper_minecart", 0.0, items(slot(0, "stone", 40))));
        // A hopper minecart pulls from a chest above it (one item per tick at most).
        out.add(new Scenario("cart_hopper_minecart_pulls_from_chest", 60)
                .container(0, 1, 0, "minecraft:chest" + items(slot(0, "apple", 7), slot(1, "coal", 3), slot(20, "apple", 60)))
                .cart(0, 0, 0, "hopper_minecart", 0.0, items(slot(0, "apple", 60))));
        // ... and from a hopper block above it, which also pushes into it.
        out.add(new Scenario("cart_hopper_minecart_and_hopper_block", 60)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "coal", 20)))
                .cart(0, 0, 0, "hopper_minecart", 0.0, ""));
        // A hopper minecart pulls from a chest minecart above it.
        out.add(new Scenario("cart_hopper_minecart_pulls_from_chest_minecart", 40)
                .cart(0, 1, 0, "chest_minecart", 0.0, items(slot(0, "apple", 4), slot(5, "stick", 2)))
                .cart(0, 0, 0, "hopper_minecart", 0.0, ""));
        // A full chest minecart takes nothing; a nearly full one takes what fits.
        out.add(new Scenario("cart_chest_minecart_full", 40)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 64), slot(1, "stone", 10)))
                .cart(0, 0, 0, "chest_minecart", 0.0, items(fullChest("stone", 64, 26, 0))));
        out.add(new Scenario("cart_chest_minecart_nearly_full", 40)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 64), slot(1, "stone", 10)))
                .cart(0, 0, 0, "chest_minecart", 0.0, items(fullChest("stone", 64, 26, 62))));
        // A hopper block beside nothing: a switched-off hopper minecart still gives up items.
        out.add(new Scenario("cart_hopper_block_pulls_from_hopper_minecart_disabled", 40)
                .container(0, 0, 0, "minecraft:hopper[facing=east]")
                .container(1, 0, 0, "minecraft:chest")
                .cart(0, 1, 0, "hopper_minecart", 0.0, "{Enabled:0b,Items:[" + slot(0, "apple", 2) + "," + slot(3, "coal", 2) + "]}"));
    }

    static void comparator(List<Scenario> out, String name, String block) {
        out.add(new Scenario(name, 8).container(0, 0, 1, block).comparator(0, 0, 0, "south"));
    }

    /** `n` stacks of `count`, then (if `last` > 0) one stack of `last`. */
    static String[] fullChest(String id, int count, int n, int last) {
        List<String> s = new ArrayList<>();
        for (int i = 0; i < n; i++) s.add(slot(i, id, count));
        if (last > 0) s.add(slot(n, id, last));
        return s.toArray(new String[0]);
    }

    // ---------------------------------------------------------------- runner

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static void run(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "ContainerVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : scenarios()) {
            if (filter == null || s.name.contains(filter)) selected.add(s);
        }
        System.out.println("ContainerVectors: " + selected.size() + " scenarios");
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -2; cx <= 2; cx++)
                for (int cz = -2; cz <= 2; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(2000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            for (Scenario s : selected) {
                try {
                    lines.add(run(server, s));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + s.name + "\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("ContainerVectors: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + harnessPort(),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:the_void\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "enable-rcon=false",
                "enable-query=false",
                "spawn-monsters=false",
                "generate-structures=false",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 600; i++) {
            for (Thread t : Thread.getAllStackTraces().keySet()) {
                if (!t.getName().equals("Server thread")) continue;
                Field holderField = Thread.class.getDeclaredField("holder");
                holderField.setAccessible(true);
                Object holder = holderField.get(t);
                Field taskField = holder.getClass().getDeclaredField("task");
                taskField.setAccessible(true);
                Object task = taskField.get(holder);
                for (Field f : task.getClass().getDeclaredFields()) {
                    f.setAccessible(true);
                    if (f.get(task) instanceof AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) {
                        while (!s.isReady()) Thread.sleep(100);
                        return s;
                    }
                }
            }
            Thread.sleep(100);
        }
        throw new IllegalStateException("server did not start");
    }

    // ---------------------------------------------------------------- one scenario

    static void command(MinecraftServer server, String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
    }

    /** Relative `~dx ~dy ~dz` in a scenario command become absolute coordinates. */
    static String absolute(String cmd) {
        StringBuilder out = new StringBuilder();
        int axis = 0;
        for (String part : cmd.split(" ")) {
            if (out.length() > 0) out.append(' ');
            if (part.startsWith("~")) {
                int d = part.length() > 1 ? Integer.parseInt(part.substring(1)) : 0;
                out.append(BASE[axis % 3] + d);
                axis++;
            } else {
                out.append(part);
            }
        }
        return out.toString();
    }

    static BlockPos pos(int[] p) {
        return new BlockPos(BASE[0] + p[0], BASE[1] + p[1], BASE[2] + p[2]);
    }

    static String stateString(BlockState s) {
        String name = BuiltInRegistries.BLOCK.getKey(s.getBlock()).toString();
        List<String> props = new ArrayList<>();
        for (Property<?> p : s.getProperties()) props.add(p.getName() + "=" + value(s, p));
        props.sort(null);
        return props.isEmpty() ? name : name + "[" + String.join(",", props) + "]";
    }

    static <T extends Comparable<T>> String value(BlockState s, Property<T> p) {
        return p.getName(s.getValue(p));
    }

    static Object field(Object o, String name) throws Exception {
        for (Class<?> k = o.getClass(); k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException e) {
                // superclass
            }
        }
        throw new NoSuchFieldException(name);
    }

    static void setRunsNormally(ServerLevel level, boolean runs) throws Exception {
        Field f = net.minecraft.world.TickRateManager.class.getDeclaredField("runGameElements");
        f.setAccessible(true);
        f.set(level.tickRateManager(), runs);
    }

    static Map<String, Object> containerState(BlockEntity be) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        // wp49: a campfire is no Container: its four spots and their timers.
        if (be instanceof net.minecraft.world.level.block.entity.CampfireBlockEntity cf) {
            List<Object> items = new ArrayList<>();
            var list = cf.getItems();
            for (int i = 0; i < list.size(); i++) {
                ItemStack st = list.get(i);
                if (!st.isEmpty()) items.add(List.of(i, BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount()));
            }
            m.put("items", items);
            int[] times = (int[]) field(be, "cookingProgress"), totals = (int[]) field(be, "cookingTime");
            m.put("campfire", List.of(times[0], times[1], times[2], times[3], totals[0], totals[1], totals[2], totals[3]));
            return m;
        }
        if (!(be instanceof Container c)) return m;
        List<Object> items = new ArrayList<>();
        for (int i = 0; i < c.getContainerSize(); i++) {
            ItemStack st = c.getItem(i);
            if (!st.isEmpty()) items.add(List.of(i, BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount()));
        }
        m.put("items", items);
        if (be instanceof HopperBlockEntity) m.put("cooldown", field(be, "cooldownTime"));
        // wp36: a jukebox's song player (playing, ticks since the song started).
        if (be instanceof net.minecraft.world.level.block.entity.JukeboxBlockEntity j) {
            m.put("jukebox", List.of(j.getSongPlayer().isPlaying() ? 1 : 0, j.getSongPlayer().getTicksSinceSongStarted()));
        }
        if (be instanceof AbstractFurnaceBlockEntity) {
            m.put("furnace", List.of(field(be, "litTimeRemaining"), field(be, "litTotalTime"), field(be, "cookingTimer"),
                    field(be, "cookingTotalTime")));
        }
        return m;
    }

    /** The item entities in the scenario's area: [item, total count], sorted by item. */
    static List<Object> dropsState(ServerLevel level) {
        TreeMap<String, Integer> sums = new TreeMap<>();
        var box = new net.minecraft.world.phys.AABB(BASE[0] - 3, BASE[1] - 3, BASE[2] - 3, BASE[0] + 7, BASE[1] + 7, BASE[2] + 7);
        for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.item.ItemEntity.class, box)) {
            sums.merge(BuiltInRegistries.ITEM.getKey(e.getItem().getItem()).toString(), e.getItem().getCount(), Integer::sum);
        }
        List<Object> out = new ArrayList<>();
        for (var en : sums.entrySet()) out.add(List.of(en.getKey(), en.getValue()));
        return out;
    }

    /** The slots of the container minecart standing in the block (empty when there is none). */
    static Map<String, Object> cartState(ServerLevel level, int[] p) {
        Map<String, Object> m = new LinkedHashMap<>();
        BlockPos b = pos(p);
        var carts = level.getEntitiesOfClass(net.minecraft.world.entity.vehicle.minecart.AbstractMinecartContainer.class,
                new net.minecraft.world.phys.AABB(b.getX() + 0.01, b.getY() + 0.01, b.getZ() + 0.01, b.getX() + 0.99, b.getY() + 0.99, b.getZ() + 0.99));
        if (carts.isEmpty()) return m;
        List<Object> items = new ArrayList<>();
        var c = carts.get(0);
        for (int i = 0; i < c.getContainerSize(); i++) {
            ItemStack st = c.getItemStacks().get(i);
            if (!st.isEmpty()) items.add(List.of(i, BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount()));
        }
        m.put("items", items);
        return m;
    }

    static String run(MinecraftServer server, Scenario s) throws Exception {
        ServerLevel level = server.overworld();
        if (s.align20) {
            setRunsNormally(level, true);
            while (level.getGameTime() % 20 != 0) level.tick(() -> true);
        }
        for (Object[] b : s.sortedBlocks()) {
            command(server, String.format(Locale.ROOT, "setblock %d %d %d %s", BASE[0] + (int) b[0], BASE[1] + (int) b[1],
                    BASE[2] + (int) b[2], b[3]));
        }
        List<Object> ticks = new ArrayList<>();
        // Whole level ticks while the server's own ticking stays frozen: `runsNormally` is what
        // `ServerLevel.tick` checks (the manager only updates it in its own tick).
        setRunsNormally(level, true);
        try {
            for (int t = 1; t <= s.ticks; t++) {
                for (String cmd : s.actions.getOrDefault(t, List.of())) command(server, absolute(cmd));
                level.tick(() -> true);
                Map<String, Object> tick = new LinkedHashMap<>();
                List<Object> cs = new ArrayList<>();
                for (int[] p : s.containers) cs.add(containerState(level.getBlockEntity(pos(p))));
                tick.put("containers", cs);
                List<Object> st = new ArrayList<>();
                for (int[] p : s.states) st.add(stateString(level.getBlockState(pos(p))));
                tick.put("states", st);
                List<Object> carts = new ArrayList<>();
                for (int[] p : s.carts) carts.add(cartState(level, p));
                tick.put("carts", carts);
                if (s.watchDrops) {
                    tick.put("drops", dropsState(level));
                    // What lies about burns, merges or is picked up in its own ways: only what each tick makes is compared.
                    command(server, "kill @e[type=item]");
                }
                List<Object> cmp = new ArrayList<>();
                for (int[] p : s.comparators) {
                    cmp.add(level.getBlockEntity(pos(p)) instanceof ComparatorBlockEntity c ? c.getOutputSignal() : -1);
                }
                tick.put("comparators", cmp);
                ticks.add(tick);
            }
        } finally {
            setRunsNormally(level, false);
        }
        // Clear the area (without drops) for the next scenario.
        command(server, String.format(Locale.ROOT, "fill %d %d %d %d %d %d air strict", BASE[0] - 3, BASE[1] - 3, BASE[2] - 3,
                BASE[0] + 6, BASE[1] + 6, BASE[2] + 6));
        // Killed container minecarts drop what they hold: the carts first, then the items.
        command(server, "kill @e[type=chest_minecart]");
        command(server, "kill @e[type=hopper_minecart]");
        command(server, "kill @e[type=item]");
        Map<String, Object> line = s.json();
        line.put("result", ticks);
        return toJson(line);
    }

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof Float f) return Float.toString(f);
        if (o instanceof Double d) return Double.toString(d);
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            boolean first = true;
            for (var e : m.entrySet()) {
                b.append(first ? "" : ",").append(toJson(String.valueOf(e.getKey()))).append(":").append(toJson(e.getValue()));
                first = false;
            }
            return b.append("}").toString();
        }
        return toJson(o.toString());
    }

    /// $KILN_HARNESS_PORT, else the first free port of 25581-25583 (wp44's; waits while all are busy).
    static String harnessPort() {
        String env = System.getenv("KILN_HARNESS_PORT");
        if (env != null) return env;
        for (int i = 0; i < 900; i++) {
            for (int p = 25581; p <= 25583; p++) {
                try (var s = new java.net.ServerSocket(p)) {
                    return Integer.toString(p);
                } catch (java.io.IOException e) {
                    // busy
                }
            }
            try {
                Thread.sleep(2000);
            } catch (InterruptedException e) {
                throw new IllegalStateException(e);
            }
        }
        throw new IllegalStateException("no free harness port");
    }
}
