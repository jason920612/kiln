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
        /** The items are made by the entity phase (Kiln has them a tick later). */
        boolean dropsLag;
        /** The bees appearing each tick (where) and their number are recorded. */
        boolean watchBees;
        /** wp49: the new entities of each tick are recorded. */
        boolean watchEntities;
        /** wp49: the equipment (and chest) of every living thing about is recorded each tick. */
        boolean watchMobs;
        /** wp49: every entity but items and players is recorded each tick: type, position, motion and health. */
        boolean track;

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

        Scenario dropsLag() {
            dropsLag = true;
            return this;
        }

        Scenario align20() {
            align20 = true;
            return this;
        }

        Scenario drops() {
            watchDrops = true;
            return this;
        }

        Scenario bees() {
            watchBees = true;
            return this;
        }

        /** wp49: the entities (other than items and players) that appear are recorded each tick (type and where). */
        Scenario entities() {
            watchEntities = true;
            return this;
        }

        Scenario track() {
            track = true;
            return this;
        }

        Scenario mobs() {
            watchMobs = true;
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
            m.put("drops_lag", dropsLag);
            m.put("bees", watchBees);
            m.put("entities", watchEntities);
            m.put("mobs", watchMobs);
            m.put("track", track);
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
        targetScenarios(out);
        projectileBlockScenarios(out);
        hiveScenarios(out);
        potScenarios(out);
        lecternScenarios(out);
        dispenserScenarios(out);
        dispenserScenarios2(out);
        windScenarios(out);
        cropProbes(out);
        crafterScenarios(out);
        commandBlockScenarios(out);
        return out;
    }

    /** wp49: a command block of `kind` (facing east, powered from below at tick 2 unless said otherwise) at the origin. */
    static String cb(String kind, boolean conditional, String command, String extra) {
        return String.format(Locale.ROOT, "minecraft:%s[facing=east,conditional=%b]{Command:\"%s\"%s}", kind, conditional, command, extra.isEmpty() ? "" : "," + extra);
    }

    static Scenario cbScenario(String name, int ticks) {
        return new Scenario("cb_" + name, ticks).state(0, 1, 0).state(1, 1, 0).state(2, 1, 0).state(3, 1, 0);
    }

    static void commandBlockScenarios(List<Scenario> out) {
        String gold = "setblock ~ ~1 ~ minecraft:gold_block";
        String iron = "setblock ~ ~1 ~ minecraft:iron_block";
        String power = "setblock ~0 ~-1 ~0 minecraft:redstone_block";
        // A plain command block: power makes it run, once for a rising edge.
        out.add(cbScenario("basic", 14).container(0, 0, 0, cb("command_block", false, gold, "")).at(2, power));
        out.add(cbScenario("two_pulses", 24).container(0, 0, 0, cb("command_block", false, gold, "")).at(2, power).at(5, "setblock ~0 ~-1 ~0 minecraft:air")
                .at(8, power).at(14, "setblock ~0 ~1 ~0 minecraft:air"));
        out.add(cbScenario("not_powered", 10).container(0, 0, 0, cb("command_block", false, gold, "")));
        out.add(cbScenario("held_power", 20).container(0, 0, 0, cb("command_block", false, gold, "")).at(2, power).at(8, "setblock ~0 ~1 ~0 minecraft:air"));
        // The scoreboard counts the runs.
        String count = "scoreboard players add s n 1";
        out.add(cbScenario("repeating_powered", 16).container(0, 0, 0, cb("repeating_command_block", false, count, "")).at(1, "scoreboard objectives remove n").at(1, "scoreboard objectives add n dummy").at(2, power));
        out.add(cbScenario("repeating_idle", 16).container(0, 0, 0, cb("repeating_command_block", false, count, "auto:1b")).at(1, "scoreboard objectives remove n").at(1, "scoreboard objectives add n dummy"));
        out.add(cbScenario("repeating_auto_kick", 16).container(0, 0, 0, cb("repeating_command_block", false, count, "auto:1b")).at(1, "scoreboard objectives remove n").at(1, "scoreboard objectives add n dummy").at(3, power)
                .at(5, "setblock ~0 ~-1 ~0 minecraft:air"));
        // Chains: each block above its own.
        out.add(cbScenario("chain", 16).container(0, 0, 0, cb("command_block", false, gold, "")).container(1, 0, 0, cb("chain_command_block", false, iron, "auto:1b"))
                .container(2, 0, 0, cb("chain_command_block", false, gold, "auto:1b")).at(2, power));
        out.add(cbScenario("chain_not_active", 16).container(0, 0, 0, cb("command_block", false, gold, "")).container(1, 0, 0, cb("chain_command_block", false, iron, ""))
                .container(2, 0, 0, cb("chain_command_block", false, gold, "auto:1b")).at(2, power));
        out.add(cbScenario("chain_conditional_after_failure", 16)
                .container(0, 0, 0, cb("command_block", false, "execute if block ~ ~ ~ minecraft:diamond_block run setblock ~ ~1 ~ minecraft:gold_block", ""))
                .container(1, 0, 0, cb("chain_command_block", true, iron, "auto:1b")).container(2, 0, 0, cb("chain_command_block", false, gold, "auto:1b")).at(2, power));
        out.add(cbScenario("chain_conditional_after_success", 16).container(0, 0, 0, cb("command_block", false, gold, ""))
                .container(1, 0, 0, cb("chain_command_block", true, iron, "auto:1b")).container(2, 0, 0, cb("chain_command_block", true, gold, "auto:1b")).at(2, power));
        out.add(cbScenario("conditional_without_a_block_behind", 12).container(0, 0, 0, cb("command_block", true, gold, "")).at(2, power));
        // The comparator reads the success count.
        out.add(cbScenario("comparator", 14).container(0, 0, 0, cb("command_block", false, "execute as @e[type=!player] run say hi", "")).comparator(0, 0, -2, "south").at(2, power));
        out.add(cbScenario("comparator_forks", 14).container(0, 0, 0, cb("command_block", false, "execute positioned ~ ~ ~ positioned ~1 ~ ~ positioned ~2 ~ ~ run setblock ~ ~2 ~ minecraft:gold_block", ""))
                .comparator(0, 0, -2, "south").at(2, power));
        // What the output says.
        out.add(cbScenario("output_scoreboard", 10).container(0, 0, 0, cb("command_block", false, count, "")).at(1, "scoreboard objectives remove n").at(1, "scoreboard objectives add n dummy").at(2, power));
        out.add(cbScenario("output_failure", 10).container(0, 0, 0, cb("command_block", false, "foo bar", "")).at(2, power));
        out.add(cbScenario("output_failure_in_command", 10).container(0, 0, 0, cb("command_block", false, "setblock ~ ~1 ~ minecraft:not_a_block", "")).at(2, power));
        out.add(cbScenario("output_untracked", 10).container(0, 0, 0, cb("command_block", false, count, "TrackOutput:0b")).at(1, "scoreboard objectives remove n").at(1, "scoreboard objectives add n dummy").at(2, power));
        out.add(cbScenario("output_say", 10).container(0, 0, 0, cb("command_block", false, "say hello", "")).at(2, power));
        out.add(cbScenario("output_leading_slash", 10).container(0, 0, 0, cb("command_block", false, "/setblock ~ ~1 ~ minecraft:gold_block", "")).at(2, power));
        out.add(cbScenario("searge", 10).container(0, 0, 0, cb("command_block", false, "Searge", "")).at(2, power));
        out.add(cbScenario("empty_command", 10).container(0, 0, 0, cb("command_block", false, "", "")).at(2, power));
        // The block's own state: placed powered, the command changed, a rewritten block entity.
        out.add(cbScenario("command_changed", 14).container(0, 0, 0, cb("command_block", false, gold, "")).at(2, power).at(6, "data merge block ~ ~ ~ {Command:\"" + iron + "\"}")
                .at(7, "setblock ~0 ~-1 ~0 minecraft:air").at(8, power));
        out.add(cbScenario("failed_then_works", 14).container(0, 0, 0, cb("command_block", false, "execute if block ~ ~1 ~ minecraft:gold_block run setblock ~1 ~1 ~ minecraft:iron_block", ""))
                .at(2, power).at(5, "setblock ~0 ~-1 ~0 minecraft:air").at(6, "setblock ~ ~1 ~ minecraft:gold_block").at(7, power));
    }

    /**
     * wp49: a crafter facing east at the origin, powered from above at tick 2 (it crafts at tick 6); what it puts into
     * the container in front (1, 0, 0) or throws out, its own slots and its block are recorded.
     */
    static Scenario craft(String name, String nbt) {
        return new Scenario("crafter_" + name, 20)
                .container(0, 0, 0, "minecraft:crafter[orientation=east_up]" + nbt).state(0, 0, 0).drops().entities()
                .at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block");
    }

    static void crafterScenarios(List<Scenario> out) {
        // One log makes four planks, thrown out in front.
        out.add(craft("planks", items(slot(4, "oak_log", 3))));
        // A recipe that needs the shape, and slots that hold more than one.
        out.add(craft("sticks", items(slot(1, "oak_planks", 2), slot(4, "oak_planks", 5))));
        out.add(craft("crafting_table_stacks", items(slot(0, "oak_planks", 10), slot(1, "oak_planks", 3), slot(3, "oak_planks", 1), slot(4, "oak_planks", 64))));
        out.add(craft("no_recipe", items(slot(0, "dirt", 1))));
        out.add(craft("empty", ""));
        // Milk buckets leave their buckets behind.
        out.add(craft("cake", items(slot(0, "milk_bucket", 1), slot(1, "milk_bucket", 1), slot(2, "milk_bucket", 1), slot(3, "sugar", 1), slot(4, "egg", 1),
                slot(5, "sugar", 1), slot(6, "wheat", 2), slot(7, "wheat", 1), slot(8, "wheat", 1))));
        // The result goes into the container in front.
        out.add(craft("into_chest", items(slot(4, "oak_log", 1))).container(1, 0, 0, "minecraft:chest"));
        out.add(craft("into_full_chest", items(slot(4, "oak_log", 1))).container(1, 0, 0, "minecraft:chest" + items(fullChest("stone", 64, 27, 0))));
        String[] nearly = java.util.Arrays.copyOf(fullChest("stone", 64, 26, 0), 27);
        nearly[26] = slot(26, "oak_planks", 62);
        out.add(craft("into_nearly_full_chest", items(slot(4, "oak_log", 1))).container(1, 0, 0, "minecraft:chest" + items(nearly)));
        out.add(craft("into_hopper", items(slot(4, "oak_log", 1))).container(1, 0, 0, "minecraft:hopper[facing=east]"));
        out.add(craft("cake_into_chest", items(slot(0, "milk_bucket", 1), slot(1, "milk_bucket", 1), slot(2, "milk_bucket", 1), slot(3, "sugar", 1), slot(4, "egg", 1),
                slot(5, "sugar", 1), slot(6, "wheat", 1), slot(7, "wheat", 1), slot(8, "wheat", 1))).container(1, 0, 0, "minecraft:chest"));
        // Into another crafter one item at a time, over its enabled slots.
        out.add(craft("into_crafter", items(slot(4, "oak_log", 1)))
                .container(1, 0, 0, "minecraft:crafter[orientation=north_up]{Items:[" + slot(2, "oak_planks", 1) + "],disabled_slots:[I;0,1]}"));
        // Slots switched off: kept, and counted by comparators.
        out.add(craft("disabled_slots", "{Items:[" + slot(4, "oak_log", 1) + "],disabled_slots:[I;0,1,8]}").comparator(0, 0, -1, "south"));
        out.add(craft("comparator_items", items(slot(0, "dirt", 1), slot(5, "dirt", 1))).comparator(0, 0, -1, "south"));
        out.add(craft("comparator_full", items(slot(0, "oak_log", 1), slot(1, "dirt", 1), slot(2, "dirt", 1), slot(3, "dirt", 1), slot(4, "dirt", 1), slot(5, "dirt", 1),
                slot(6, "dirt", 1), slot(7, "dirt", 1), slot(8, "dirt", 1))).comparator(0, 0, -1, "south"));
        // Facing down into a chest, and up.
        out.add(new Scenario("crafter_down_into_chest", 20).container(0, 0, 0, "minecraft:crafter[orientation=down_north]" + items(slot(4, "oak_log", 1)))
                .container(0, -1, 0, "minecraft:chest").state(0, 0, 0).drops().entities().at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        out.add(new Scenario("crafter_up_throws", 20).container(0, 0, 0, "minecraft:crafter[orientation=up_east]" + items(slot(4, "oak_log", 1)))
                .state(0, 0, 0).drops().entities().at(2, "setblock ~0 ~-1 ~0 minecraft:redstone_block"));
        // Power that comes and goes before the scheduled tick still crafts; power that stays does not craft again.
        out.add(craft("short_pulse", items(slot(4, "oak_log", 4))).at(3, "setblock ~0 ~1 ~0 minecraft:air"));
        out.add(craft("pulses_twice", items(slot(4, "oak_log", 4))).at(9, "setblock ~0 ~1 ~0 minecraft:air").at(12, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        out.add(new Scenario("crafter_unpowered", 12).container(0, 0, 0, "minecraft:crafter[orientation=east_up]" + items(slot(4, "oak_log", 1))).state(0, 0, 0).drops());
        // A hopper above spreads its items over the slots.
        out.add(new Scenario("crafter_fed_by_hopper", 60).container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "cobblestone", 20), slot(1, "dirt", 5)))
                .container(0, 0, 0, "minecraft:crafter[orientation=east_up]" + items(slot(2, "cobblestone", 3), slot(5, "cobblestone", 1))).state(0, 0, 0));
        out.add(new Scenario("crafter_fed_skips_disabled", 60).container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "cobblestone", 30)))
                .container(0, 0, 0, "minecraft:crafter[orientation=east_up]{disabled_slots:[I;0,1,2,3]}").state(0, 0, 0));
        out.add(new Scenario("crafter_fed_by_dropper", 40).container(0, 1, 0, "minecraft:dropper[facing=down]" + items(slot(0, "cobblestone", 9)))
                .container(0, 0, 0, "minecraft:crafter[orientation=east_up]").state(0, 0, 0)
                .at(2, "setblock ~1 ~1 ~0 minecraft:redstone_block").at(8, "setblock ~1 ~1 ~0 minecraft:air").at(10, "setblock ~1 ~1 ~0 minecraft:redstone_block")
                .at(16, "setblock ~1 ~1 ~0 minecraft:air").at(18, "setblock ~1 ~1 ~0 minecraft:redstone_block"));
        // A hopper below takes what the crafter holds (a crafter is a container like any).
        out.add(new Scenario("crafter_hopper_below", 40).container(0, 0, 0, "minecraft:crafter[orientation=east_up]" + items(slot(0, "dirt", 3))).container(0, -1, 0, "minecraft:hopper[facing=down]")
                .container(0, -2, 0, "minecraft:chest").state(0, 0, 0));
        // The command that empties it, and an item put into a switched-off slot switches it on.
        out.add(craft("item_command_enables_slot", "{disabled_slots:[I;3]}").at(1, "item replace block ~0 ~0 ~0 container.3 with minecraft:stone 2"));
        out.add(craft("clear_command", items(slot(4, "oak_log", 1))).at(1, "item replace block ~0 ~0 ~0 container.4 with minecraft:air"));
    }

    /**
     * wp49: a dispenser facing east is powered from above at tick 2 and fires at tick 6; what it does to the
     * block (1, 0, 0) in front, to its own slot, to the items about and to the entities that appear is recorded.
     */
    static Scenario dispense(String name, String item, int count) {
        return new Scenario("dispenser_" + name, 14)
                .container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(slot(0, item, count)))
                .state(1, 0, 0).drops().entities().mobs()
                .at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block");
    }

    static void dispenserScenarios(List<Scenario> out) {
        // Bone meal: crops that take a fixed step, one that is full grown, something that is no plant.
        out.add(dispense("bone_meal_torchflower", "bone_meal", 3).block(1, -1, 0, "minecraft:farmland[moisture=7]").block(1, 0, 0, "minecraft:torchflower_crop[age=0]"));
        out.add(dispense("bone_meal_berries", "bone_meal", 3).block(1, -1, 0, "minecraft:dirt").block(1, 0, 0, "minecraft:sweet_berry_bush[age=1]"));
        out.add(dispense("bone_meal_cocoa", "bone_meal", 3).block(1, 0, -1, "minecraft:jungle_log").block(1, 0, 0, "minecraft:cocoa[age=0,facing=north]"));
        out.add(dispense("bone_meal_full_grown", "bone_meal", 3).block(1, -1, 0, "minecraft:farmland[moisture=7]").block(1, 0, 0, "minecraft:wheat[age=7]"));
        out.add(dispense("bone_meal_stone", "bone_meal", 3).block(1, 0, 0, "minecraft:stone"));
        // Flint and steel: fire on a floor, a campfire and a candle lit, TNT primed, a stone that takes nothing.
        out.add(dispense("flint_fire", "flint_and_steel", 1).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("flint_netherrack_fire", "flint_and_steel", 1).block(1, -1, 0, "minecraft:netherrack"));
        out.add(dispense("flint_campfire", "flint_and_steel", 1).block(1, 0, 0, "minecraft:campfire[lit=false,facing=north,waterlogged=false,signal_fire=false]"));
        out.add(dispense("flint_candle", "flint_and_steel", 1).block(1, 0, 0, "minecraft:candle[lit=false,candles=2,waterlogged=false]"));
        out.add(dispense("flint_tnt", "flint_and_steel", 1).block(1, 0, 0, "minecraft:tnt"));
        out.add(dispense("flint_stone", "flint_and_steel", 1).block(1, 0, 0, "minecraft:stone"));
        out.add(dispense("flint_air_over_air", "flint_and_steel", 1));
        // Honeycomb on copper.
        out.add(dispense("honeycomb_copper", "honeycomb", 2).block(1, 0, 0, "minecraft:copper_block"));
        out.add(dispense("honeycomb_cut_copper_stairs", "honeycomb", 2).block(1, 0, 0, "minecraft:weathered_cut_copper_stairs[facing=north,half=top,shape=straight,waterlogged=false]"));
        out.add(dispense("honeycomb_stone", "honeycomb", 2).block(1, 0, 0, "minecraft:stone"));
        // Glowstone on a respawn anchor, a full one, anything else.
        out.add(dispense("glowstone_anchor", "glowstone", 2).block(1, 0, 0, "minecraft:respawn_anchor[charge=1]"));
        out.add(dispense("glowstone_full_anchor", "glowstone", 2).block(1, 0, 0, "minecraft:respawn_anchor[charge=4]"));
        out.add(dispense("glowstone_stone", "glowstone", 2).block(1, 0, 0, "minecraft:stone"));
        // Glass bottles: honey from a full hive, water from water, nothing from stone.
        out.add(dispense("bottle_hive", "glass_bottle", 2).block(1, 0, 0, "minecraft:beehive[facing=north,honey_level=5]"));
        out.add(dispense("bottle_hive_one", "glass_bottle", 1).block(1, 0, 0, "minecraft:beehive[facing=north,honey_level=5]"));
        out.add(dispense("bottle_hive_unripe", "glass_bottle", 2).block(1, 0, 0, "minecraft:beehive[facing=north,honey_level=4]"));
        out.add(dispense("bottle_water", "glass_bottle", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("bottle_stone", "glass_bottle", 2).block(1, 0, 0, "minecraft:stone"));
        // Water bottles: dirt becomes mud, a bottle of another potion is dropped.
        String water = "{Slot:0b,id:\"minecraft:potion\",count:2,components:{\"minecraft:potion_contents\":{potion:\"minecraft:water\"}}}";
        out.add(new Scenario("dispenser_water_bottle_dirt", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(water)).state(1, 0, 0).drops().entities()
                .block(1, 0, 0, "minecraft:dirt").at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        out.add(new Scenario("dispenser_water_bottle_stone", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(water)).state(1, 0, 0).drops().entities()
                .block(1, 0, 0, "minecraft:stone").at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        String awkward = "{Slot:0b,id:\"minecraft:potion\",count:2,components:{\"minecraft:potion_contents\":{potion:\"minecraft:awkward\"}}}";
        out.add(new Scenario("dispenser_awkward_potion_dirt", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(awkward)).state(1, 0, 0).drops().entities()
                .block(1, 0, 0, "minecraft:dirt").at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        // TNT is primed where it is dispensed.
        out.add(dispense("tnt", "tnt", 2));
        out.add(dispense("tnt_over_a_hole", "tnt", 2).block(1, -1, 0, "minecraft:stone"));
        // Shears on a hive, on nothing.
        out.add(dispense("shears_hive", "shears", 1).block(1, 0, 0, "minecraft:bee_nest[facing=north,honey_level=5]"));
        out.add(dispense("shears_unripe_hive", "shears", 1).block(1, 0, 0, "minecraft:bee_nest[facing=north,honey_level=3]"));
        out.add(dispense("shears_stone", "shears", 1).block(1, 0, 0, "minecraft:stone"));
        // Shulker boxes are put down (on the floor facing up, or facing the way the dispenser does over a drop).
        out.add(dispense("shulker_box_floor", "shulker_box", 1).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("shulker_box_over_air", "red_shulker_box", 2));
        out.add(dispense("shulker_box_blocked", "shulker_box", 1).block(1, 0, 0, "minecraft:stone"));
        // Boats go on water (or on the air over it).
        out.add(dispense("boat_water", "oak_boat", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("boat_over_water", "birch_chest_boat", 2).block(1, -1, 0, "minecraft:water[level=0]"));
        out.add(dispense("boat_on_land", "oak_boat", 2).block(1, -1, 0, "minecraft:stone"));
        // Armor stands.
        out.add(dispense("armor_stand", "armor_stand", 2).block(1, -1, 0, "minecraft:stone"));
        // Things that are shot out.
        for (String shot : new String[] {"arrow", "spectral_arrow", "snowball", "egg", "blue_egg", "brown_egg", "experience_bottle"}) {
            out.add(dispense("shoots_" + shot, shot, 2));
        }
        out.add(new Scenario("dispenser_shoots_splash_potion", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]"
                + items("{Slot:0b,id:\"minecraft:splash_potion\",count:2,components:{\"minecraft:potion_contents\":{potion:\"minecraft:swiftness\"}}}"))
                .state(1, 0, 0).drops().entities().at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        // Facing up and down, and a stack that runs out.
        out.add(new Scenario("dispenser_up_bone_meal_hits_nothing", 14).container(0, 0, 0, "minecraft:dispenser[facing=up]" + items(slot(0, "bone_meal", 1)))
                .block(0, 1, 0, "minecraft:stone").state(0, 1, 0).drops().entities().at(2, "setblock ~1 ~0 ~0 minecraft:redstone_block"));
        out.add(new Scenario("dispenser_last_flint_and_steel", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(slot(0, "flint_and_steel", 1)))
                .block(1, -1, 0, "minecraft:stone").state(1, 0, 0).drops().entities().at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
    }

    /** wp49: an entity summoned at the first tick, at the middle of the block (x, y, z) in front of the dispenser, without AI. */
    static Scenario mob(Scenario s, String type, double dx, double dy, double dz, String nbt) {
        return s.at(1, String.format(Locale.ROOT, "summon minecraft:%s %s %s %s {NoAI:1b,PersistenceRequired:1b%s}", type, BASE[0] + dx, BASE[1] + dy, BASE[2] + dz, nbt.isEmpty() ? "" : "," + nbt));
    }

    /** wp49: the behaviours the first dispenser scenarios left out. */
    static void dispenserScenarios2(List<Scenario> out) {
        String tipped = "{Slot:0b,id:\"minecraft:tipped_arrow\",count:2,components:{\"minecraft:potion_contents\":{potion:\"minecraft:swiftness\"}}}";
        out.add(new Scenario("dispenser_shoots_tipped_arrow", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(tipped))
                .state(1, 0, 0).drops().entities().at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        String rocket = "{Slot:0b,id:\"minecraft:firework_rocket\",count:2,components:{\"minecraft:fireworks\":{flight_duration:2,explosions:[{shape:\"small_ball\",colors:[I;255]}]}}}";
        out.add(new Scenario("dispenser_firework_rocket", 14).container(0, 0, 0, "minecraft:dispenser[facing=east]" + items(rocket))
                .state(1, 0, 0).drops().entities().at(2, "setblock ~0 ~1 ~0 minecraft:redstone_block"));
        out.add(dispense("fire_charge", "fire_charge", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("fire_charge_air", "fire_charge", 2));
        out.add(dispense("wind_charge", "wind_charge", 2));
        // Spawn eggs.
        out.add(dispense("spawn_egg_pig", "pig_spawn_egg", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("spawn_egg_creeper", "creeper_spawn_egg", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("spawn_egg_zombie_over_air", "zombie_spawn_egg", 1));
        out.add(dispense("spawn_egg_blocked", "pig_spawn_egg", 2).block(1, 0, 0, "minecraft:stone"));
        out.add(dispense("spawn_egg_sheep_up", "sheep_spawn_egg", 2).block(1, -1, 0, "minecraft:stone"));
        // Buckets.
        out.add(dispense("powder_snow_bucket", "powder_snow_bucket", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("powder_snow_bucket_stone", "powder_snow_bucket", 2).block(1, 0, 0, "minecraft:stone"));
        out.add(dispense("salmon_bucket_water", "salmon_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("salmon_bucket_land", "salmon_bucket", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("cod_bucket_water", "cod_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("pufferfish_bucket_water", "pufferfish_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("tropical_fish_bucket_water", "tropical_fish_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("axolotl_bucket_water", "axolotl_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("tadpole_bucket_water", "tadpole_bucket", 2).block(1, -1, 0, "minecraft:stone").block(1, 0, 0, "minecraft:water[level=0]"));
        out.add(dispense("sulfur_cube_bucket", "sulfur_cube_bucket", 2).block(1, -1, 0, "minecraft:stone"));
        // Skulls and pumpkins.
        out.add(dispense("wither_skull", "wither_skeleton_skull", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("wither_skull_blocked", "wither_skeleton_skull", 2).block(1, 0, 0, "minecraft:stone"));
        out.add(dispense("carved_pumpkin", "carved_pumpkin", 2).block(1, -1, 0, "minecraft:stone"));
        out.add(dispense("carved_pumpkin_snow_golem", "carved_pumpkin", 2).block(1, -1, 0, "minecraft:snow_block").block(1, -2, 0, "minecraft:snow_block"));
        out.add(dispense("carved_pumpkin_iron_golem", "carved_pumpkin", 2).block(1, -1, 0, "minecraft:iron_block").block(1, -2, 0, "minecraft:iron_block")
                .block(1, -1, 1, "minecraft:iron_block").block(1, -1, -1, "minecraft:iron_block"));
        // Equipment on whoever stands in front.
        for (String[] eq : new String[][] {{"iron_helmet", "zombie"}, {"iron_chestplate", "zombie"}, {"iron_boots", "skeleton"}, {"elytra", "zombie"}, {"carved_pumpkin", "zombie"}, {"saddle", "horse"}, {"leather_horse_armor", "horse"}, {"saddle", "strider"}, {"diamond_helmet", "armor_stand"}}) {
            Scenario s = dispense("equip_" + eq[0] + "_on_" + eq[1], eq[0], 2).block(1, -1, 0, "minecraft:stone");
            out.add(mob(s, eq[1], 1.5, 0, 0.5, eq[1].equals("horse") || eq[1].equals("pig") ? "Tame:1b" : ""));
        }
        out.add(mob(dispense("equip_helmet_on_player_less_zombie_with_helmet", "iron_helmet", 2).block(1, -1, 0, "minecraft:stone"), "zombie", 1.5, 0, 0.5,
                "equipment:{head:{id:\"minecraft:leather_helmet\",count:1}}"));
        out.add(mob(dispense("equip_wolf_armor", "wolf_armor", 2).block(1, -1, 0, "minecraft:stone"), "wolf", 1.5, 0, 0.5, "Owner:[I;1,2,3,4]"));
        // Chests onto the chested animals.
        out.add(mob(dispense("chest_on_donkey", "chest", 2).block(1, -1, 0, "minecraft:stone"), "donkey", 1.5, 0, 0.5, "Tame:1b"));
        out.add(mob(dispense("chest_on_horse", "chest", 2).block(1, -1, 0, "minecraft:stone"), "horse", 1.5, 0, 0.5, "Tame:1b"));
        out.add(mob(dispense("chest_on_untamed_donkey", "chest", 2).block(1, -1, 0, "minecraft:stone"), "donkey", 1.5, 0, 0.5, ""));
        out.add(mob(dispense("chest_on_llama", "chest", 2).block(1, -1, 0, "minecraft:stone"), "llama", 1.5, 0, 0.5, "Tame:1b"));
        // Shears on what can be sheared (the loot of a sheep is random, so its drops are left out).
        out.add(mob(dispense("shears_snow_golem", "shears", 1).block(1, -1, 0, "minecraft:stone"), "snow_golem", 1.5, 0, 0.5, ""));
        out.add(mob(dispense("shears_mooshroom", "shears", 1).block(1, -1, 0, "minecraft:stone"), "mooshroom", 1.5, 0, 0.5, ""));
        out.add(mob(dispense("shears_baby_mooshroom", "shears", 1).block(1, -1, 0, "minecraft:stone"), "mooshroom", 1.5, 0, 0.5, "Age:-24000"));
        out.add(mob(dispense("shears_cow", "shears", 1).block(1, -1, 0, "minecraft:stone"), "cow", 1.5, 0, 0.5, ""));
        out.add(mob(dispense("shears_pumpkinless_snow_golem", "shears", 1).block(1, -1, 0, "minecraft:stone"), "snow_golem", 1.5, 0, 0.5, "Pumpkin:0b"));
        // Brush on suspicious sand.
        out.add(dispense("brush_sand", "brush", 1).block(1, 0, 0, "minecraft:suspicious_sand[dusted=0]"));
        out.add(dispense("brush_stone", "brush", 1).block(1, 0, 0, "minecraft:stone"));
        // Something swallowable into a sulfur cube.
        out.add(mob(dispense("swallow_planks", "oak_planks", 2).block(1, -1, 0, "minecraft:stone"), "sulfur_cube", 1.5, 0, 0.5, "Size:1"));
        out.add(mob(dispense("swallow_by_baby", "oak_planks", 2).block(1, -1, 0, "minecraft:stone"), "sulfur_cube", 1.5, 0, 0.5, "Size:1,Age:-24000"));
        out.add(mob(dispense("swallow_dirt_when_full", "dirt", 2).block(1, -1, 0, "minecraft:stone"), "sulfur_cube", 1.5, 0, 0.5,
                "Size:1,equipment:{body:{id:\"minecraft:oak_planks\",count:1}}"));
    }

    static void cropProbes(List<Scenario> out) {
        out.add(new Scenario("probe_crop_on_dispenser", 8).container(0, 0, 0, "minecraft:dispenser[facing=up]").block(0, 1, 0, "minecraft:torchflower_crop[age=0]").state(0, 1, 0).drops());
        out.add(new Scenario("probe_crop_on_stone", 8).block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:torchflower_crop[age=0]").state(0, 1, 0).drops());
        out.add(new Scenario("probe_crop_on_dispenser_power", 8).container(0, 0, 0, "minecraft:dispenser[facing=up]").block(0, 1, 0, "minecraft:torchflower_crop[age=0]").state(0, 1, 0).drops()
                .at(2, "setblock ~1 ~0 ~0 minecraft:redstone_block"));
        out.add(new Scenario("probe_crop_on_stone_power", 8).block(0, 0, 0, "minecraft:stone").block(0, 1, 0, "minecraft:torchflower_crop[age=0]").state(0, 1, 0).drops()
                .at(2, "setblock ~1 ~0 ~0 minecraft:redstone_block"));
    }

    /** wp49: a wind charge (summoned, since a dispensed one is spread by its own random) meets a wall, a mob and a cart. */
    static void windScenarios(List<Scenario> out) {
        double bx = BASE[0], by = BASE[1], bz = BASE[2];
        String charge = "summon minecraft:wind_charge %s %s %s {Motion:[%sd,%sd,%sd]%s}";
        out.add(new Scenario("wind_wall", 14).block(4, 0, 0, "minecraft:stone").block(4, 1, 0, "minecraft:stone").block(4, -1, 0, "minecraft:stone").track()
                .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 0.5, bz + 0.5, 1.0, 0.0, 0.0, ""))
                .at(1, String.format(Locale.ROOT, "summon minecraft:chest_minecart %s %s %s {NoGravity:1b}", bx + 3.0, by + 0.5, bz + 1.0)));
        out.add(mob(new Scenario("wind_zombie", 14).track(), "husk", 4.5, 0, 0.5, "")
                .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 1.0, bz + 0.5, 1.0, 0.0, 0.0, "")));
        out.add(mob(new Scenario("wind_zombie_near_miss", 14).track(), "husk", 4.5, 0, 1.2, "")
                .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 1.0, bz + 0.5, 1.0, 0.0, 0.0, "")));
        out.add(new Scenario("wind_floor", 14).block(0, -1, 0, "minecraft:stone").block(1, -1, 0, "minecraft:stone").block(2, -1, 0, "minecraft:stone").track()
                .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 1.5, bz + 0.5, 0.3, -0.5, 0.0, ""))
                .at(1, String.format(Locale.ROOT, "summon minecraft:chest_minecart %s %s %s {NoGravity:1b}", bx + 1.5, by + 0.2, bz + 0.5)));
        out.add(mob(new Scenario("wind_probe_snowball", 14).track(), "husk", 4.5, 0, 0.5, "")
                .at(1, "summon minecraft:snowball " + (bx + 0.5) + " " + (by + 1.0) + " " + (bz + 0.5) + " {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}"));
        out.add(mob(new Scenario("wind_probe_low", 14).track(), "husk", 4.5, 0, 0.5, "")
                .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 0.5, bz + 0.5, 1.0, 0.0, 0.0, "")));
        out.add(new Scenario("wind_probe_cart", 14).track().at(1, String.format(Locale.ROOT, "summon minecraft:chest_minecart %s %s %s {NoGravity:1b}", bx + 0.5, by + 0.5, bz + 0.5))
                .at(1, String.format(Locale.ROOT, charge, bx + 4.5, by + 0.9, bz + 0.5, -1.0, 0.0, 0.0, "")));
        for (String t : new String[] {"pig", "cow", "armor_stand", "creeper", "villager", "sheep", "chicken", "wolf", "spider", "enderman", "husk", "iron_golem", "wandering_trader"}) {
            out.add(mob(new Scenario("wind_probe_" + t, 8).track(), t, 4.5, 0, 0.5, "")
                    .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 0.5, bz + 0.5, 1.0, 0.0, 0.0, "")));
        }
        for (double zx : new double[] {2.0, 3.0, 5.0, 6.0, 8.0}) {
            out.add(mob(new Scenario("wind_probe_zombie_at_" + (int) zx, 8).track(), "husk", zx, 0, 0.5, "")
                    .at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 0.5, bz + 0.5, 1.0, 0.0, 0.0, "")));
        }
        out.add(new Scenario("wind_open_air", 40).track().at(1, String.format(Locale.ROOT, charge, bx + 0.5, by + 5.0, bz + 0.5, 0.0, 0.3, 0.0, "")));
    }

    /** wp49: a comparator reads how far into the book a lectern is open. */
    static void lecternScenarios(List<Scenario> out) {
        String book = "{id:\"minecraft:written_book\",count:1,components:{\"minecraft:written_book_content\":{title:\"T\",author:\"A\",pages:[\"1\",\"2\",\"3\",\"4\",\"5\"]}}}";
        String one = "{id:\"minecraft:written_book\",count:1,components:{\"minecraft:written_book_content\":{title:\"T\",author:\"A\",pages:[\"1\"]}}}";
        String writable = "{id:\"minecraft:writable_book\",count:1,components:{\"minecraft:writable_book_content\":{pages:[\"a\",\"b\",\"c\"]}}}";
        for (int page = 0; page < 5; page++) {
            out.add(new Scenario("lectern_comparator_page_" + page, 6)
                    .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=true,powered=false]{Book:" + book + ",Page:" + page + "}")
                    .comparator(0, 0, 0, "south"));
        }
        out.add(new Scenario("lectern_comparator_one_page", 6)
                .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=true,powered=false]{Book:" + one + "}").comparator(0, 0, 0, "south"));
        out.add(new Scenario("lectern_comparator_writable_last", 6)
                .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=true,powered=false]{Book:" + writable + ",Page:2}").comparator(0, 0, 0, "south"));
        out.add(new Scenario("lectern_comparator_no_book", 6)
                .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=false,powered=false]").comparator(0, 0, 0, "south"));
        // Powered: the lectern gives 15 to the block under it and around; with a lamp beside it.
        out.add(new Scenario("lectern_powered_lights_lamp", 8)
                .block(1, 0, 1, "minecraft:redstone_lamp").state(1, 0, 1)
                .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=true,powered=true]{Book:" + book + "}").state(0, 0, 1));
        // A pulse ends after two ticks.
        out.add(new Scenario("lectern_pulse_ends", 8)
                .block(1, 0, 1, "minecraft:redstone_lamp").state(1, 0, 1)
                .block(0, 0, 1, "minecraft:lectern[facing=north,has_book=true,powered=false]{Book:" + book + "}").state(0, 0, 1)
                .at(2, "setblock ~0 ~0 ~1 minecraft:lectern[facing=north,has_book=true,powered=true]{Book:" + book + "}"));
    }

    /** wp49: decorated pots as containers: hoppers fill and empty them, comparators read them, broken they drop what they hold. */
    static void potScenarios(List<Scenario> out) {
        String pot = "minecraft:decorated_pot[facing=north,cracked=false,waterlogged=false]";
        String sherds = "sherds:{back:{id:\"minecraft:archer_pottery_sherd\"},left:{id:\"minecraft:brick\"},right:{id:\"minecraft:brick\"},front:{id:\"minecraft:angler_pottery_sherd\"}}";
        out.add(new Scenario("pot_hopper_fills_it", 40)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 10)))
                .container(0, 0, 0, pot + "{" + sherds + "}"));
        out.add(new Scenario("pot_hopper_fills_it_to_a_stack", 30)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "stone", 10)))
                .container(0, 0, 0, pot + "{item:{id:\"minecraft:stone\",count:62}}"));
        out.add(new Scenario("pot_hopper_wrong_item_stays_out", 20)
                .container(0, 1, 0, "minecraft:hopper[facing=down]" + items(slot(0, "dirt", 10)))
                .container(0, 0, 0, pot + "{item:{id:\"minecraft:stone\",count:3}}"));
        out.add(new Scenario("pot_hopper_empties_it", 40)
                .container(0, 1, 0, pot + "{item:{id:\"minecraft:cobblestone\",count:12}}")
                .container(0, 0, 0, "minecraft:hopper[facing=down]")
                .container(0, -1, 0, "minecraft:chest"));
        for (int n : new int[] {1, 7, 32, 64}) {
            Scenario s = new Scenario("pot_comparator_" + n, 6)
                    .container(0, 0, 1, pot + "{item:{id:\"minecraft:stone\",count:" + n + "}}")
                    .comparator(0, 0, 0, "south");
            out.add(s);
        }
        out.add(new Scenario("pot_comparator_unstackable", 6)
                .container(0, 0, 1, pot + "{item:{id:\"minecraft:iron_sword\",count:1}}")
                .comparator(0, 0, 0, "south"));
        out.add(new Scenario("pot_comparator_16_stack", 6)
                .container(0, 0, 1, pot + "{item:{id:\"minecraft:ender_pearl\",count:16}}")
                .comparator(0, 0, 0, "south"));
        out.add(new Scenario("pot_removed_drops_contents", 10).drops()
                .container(0, 0, 0, pot + "{item:{id:\"minecraft:cobblestone\",count:40}," + sherds + "}")
                .at(3, "setblock ~0 ~0 ~0 minecraft:air"));
    }

    /** wp49: beehives and bee nests: bees leave after their time, leave honey, stay in at night, wait for a free front. */
    static void hiveScenarios(List<Scenario> out) {
        java.util.function.BiFunction<Boolean, Integer, String> bee = (nectar, min) ->
                "{entity_data:{id:\"minecraft:bee\"" + (nectar ? ",HasNectar:1b" : "") + "},min_ticks_in_hive:" + min + ",ticks_in_hive:0}";
        String nest = "minecraft:bee_nest[facing=west,honey_level=0]";
        out.add(new Scenario("hive_releases_bees_after_their_time", 30).bees()
                .container(0, 0, 0, nest + "{bees:[" + bee.apply(true, 5) + "," + bee.apply(false, 8) + "]}").state(0, 0, 0));
        out.add(new Scenario("hive_three_nectar_bees_fill_it", 40).bees()
                .container(0, 0, 0, "minecraft:beehive[facing=north,honey_level=3]{bees:[" + bee.apply(true, 2) + "," + bee.apply(true, 4) + "," + bee.apply(true, 6) + "]}").state(0, 0, 0));
        out.add(new Scenario("hive_night_keeps_bees_in", 60).bees()
                .container(0, 0, 0, nest + "{bees:[" + bee.apply(true, 3) + "]}").state(0, 0, 0)
                .at(1, "time set 14000").at(30, "time set 1000"));
        out.add(new Scenario("hive_blocked_front_waits", 40).bees()
                .block(-1, 0, 0, "minecraft:stone")
                .container(0, 0, 0, nest + "{bees:[" + bee.apply(true, 3) + "]}").state(0, 0, 0)
                .at(20, "setblock ~-1 ~0 ~0 minecraft:air"));
        out.add(new Scenario("hive_fire_beside_sends_bees_out", 20).bees()
                .container(0, 0, 0, nest + "{bees:[" + bee.apply(true, 500) + "," + bee.apply(false, 500) + "]}").state(0, 0, 0)
                .at(5, "setblock ~1 ~0 ~0 minecraft:fire"));
        out.add(new Scenario("hive_broken_in_creative_keeps_nothing", 10).bees()
                .container(0, 0, 0, nest + "{bees:[" + bee.apply(true, 500) + "]}").state(0, 0, 0)
                .at(3, "setblock ~0 ~0 ~0 minecraft:air"));
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

    /** wp49: target blocks hit by arrows and snowballs at different spots (the signal, how long it holds). */
    static void targetScenarios(List<Scenario> out) {
        String target = "minecraft:target[power=0]";
        // {name, y offset of the flight in the block, z offset, projectile}
        Object[][] shots = {
            {"center", 0.5, 0.5, "arrow"}, {"quarter", 0.75, 0.5, "arrow"}, {"edge", 0.95, 0.5, "arrow"}, {"corner", 0.9, 0.1, "arrow"},
            {"low", 0.2, 0.55, "arrow"}, {"snowball_center", 0.5, 0.5, "snowball"}, {"snowball_edge", 0.8, 0.3, "snowball"}, {"spectral", 0.6, 0.6, "spectral_arrow"},
        };
        for (Object[] sh : shots) {
            Scenario s = new Scenario("target_" + sh[0], 45).block(0, 0, 0, target).state(0, 0, 0)
                    .block(1, 0, 0, "minecraft:redstone_lamp[lit=false]").state(1, 0, 0);
            // From the west, three blocks away, along x.
            s.at(1, String.format(Locale.ROOT, "summon minecraft:%s ~-3 ~%s ~%s {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}", sh[3], sh[1], sh[2]));
            out.add(s);
        }
        // Two arrows into one target: the second finds it already giving.
        Scenario s = new Scenario("target_two_arrows", 60).block(0, 0, 0, target).state(0, 0, 0);
        s.at(1, "summon minecraft:arrow ~-3 ~0.5 ~0.5 {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}");
        s.at(10, "summon minecraft:arrow ~-3 ~0.9 ~0.9 {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}");
        out.add(s);
        // From above and from the south.
        s = new Scenario("target_from_above", 45).block(0, 0, 0, target).state(0, 0, 0);
        s.at(1, "summon minecraft:arrow ~0.3 ~4 ~0.6 {NoGravity:1b,Motion:[0.0d,-1.0d,0.0d]}");
        out.add(s);
        s = new Scenario("target_from_south", 45).block(0, 0, 0, target).state(0, 0, 0);
        s.at(1, "summon minecraft:arrow ~0.3 ~0.7 ~4 {NoGravity:1b,Motion:[0.0d,0.0d,-1.0d]}");
        out.add(s);
    }

    /** wp49: what arrows (burning or not) and snowballs do to the blocks they hit. */
    static void projectileBlockScenarios(List<Scenario> out) {
        String POT = "minecraft:decorated_pot[facing=north,cracked=false,waterlogged=false]{item:{id:\"minecraft:stone\",count:5},sherds:{back:{id:\"minecraft:archer_pottery_sherd\"},left:{id:\"minecraft:brick\"},front:{id:\"minecraft:angler_pottery_sherd\"}}}";
        String fire = "summon minecraft:arrow ~-3 ~0.5 ~0.5 {NoGravity:1b,Fire:200s,Motion:[1.0d,0.0d,0.0d]}";
        String plain = "summon minecraft:arrow ~-3 ~0.5 ~0.5 {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}";
        String ball = "summon minecraft:snowball ~-3 ~0.5 ~0.5 {NoGravity:1b,Motion:[1.0d,0.0d,0.0d]}";
        Object[][] cases = {
            {"tnt_fire", "minecraft:tnt", fire}, {"tnt_plain", "minecraft:tnt", plain}, {"tnt_ball", "minecraft:tnt", ball},
            {"campfire_fire", "minecraft:campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]", fire},
            {"campfire_plain", "minecraft:campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]", plain},
            {"campfire_wet", "minecraft:campfire[facing=north,lit=false,waterlogged=true,signal_fire=false]", fire},
            {"soul_campfire_fire", "minecraft:soul_campfire[facing=north,lit=false,waterlogged=false,signal_fire=false]", fire},
            {"candle_fire", "minecraft:candle[candles=2,lit=false,waterlogged=false]", fire},
            {"candle_plain", "minecraft:candle[candles=2,lit=false,waterlogged=false]", plain},
            {"candle_wet", "minecraft:candle[candles=1,lit=false,waterlogged=true]", fire},
            {"candle_cake_fire", "minecraft:red_candle_cake[lit=false]", fire},
            {"candle_lit_fire", "minecraft:candle[candles=1,lit=true,waterlogged=false]", fire},
            {"chorus_plain", "minecraft:chorus_flower[age=5]", plain}, {"chorus_fire", "minecraft:chorus_flower[age=5]", fire},
            {"chorus_ball", "minecraft:chorus_flower[age=5]", ball},
            {"pot_plain", POT, plain}, {"pot_fire", POT, fire}, {"pot_ball", POT, ball},
        };
        for (Object[] c : cases) {
            Scenario s = new Scenario("projectile_" + c[0], 40).drops().block(0, -1, 0, "minecraft:end_stone").block(0, 0, 0, (String) c[1]).state(0, 0, 0);
            s.at(1, (String) c[2]);
            out.add(s);
        }
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
                "enable-command-block=true",
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
                double d = part.length() > 1 ? Double.parseDouble(part.substring(1)) : 0;
                double v = BASE[axis % 3] + d;
                out.append(v == Math.rint(v) ? String.valueOf((long) v) : Double.toString(v));
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
        // wp49: a beehive's bees: [ticks in the hive, least ticks, nectar] each.
        if (be instanceof net.minecraft.world.level.block.entity.BeehiveBlockEntity) {
            var tag = be.saveCustomOnly(be.getLevel().registryAccess());
            var bees = tag.getListOrEmpty("bees");
            List<Object> list = new ArrayList<>();
            for (int i = 0; i < bees.size(); i++) {
                var b = bees.getCompoundOrEmpty(i);
                list.add(List.of(b.getIntOr("ticks_in_hive", 0), b.getIntOr("min_ticks_in_hive", 0), b.getCompoundOrEmpty("entity_data").getBooleanOr("HasNectar", false) ? 1 : 0));
            }
            m.put("items", new ArrayList<>());
            m.put("hive", list);
            return m;
        }
        // wp49: a command block is no Container: how often its command worked, whether it is powered, its condition met and
        // always active, and the last output (without the time).
        if (be instanceof net.minecraft.world.level.block.entity.CommandBlockEntity cb) {
            String out = cb.getCommandBlock().getLastOutput().getString();
            if (out.length() >= 11 && out.charAt(0) == '[' && out.charAt(9) == ']') out = out.substring(11);
            m.put("items", new ArrayList<>());
            m.put("cmd", List.of(cb.getCommandBlock().getSuccessCount(), cb.isPowered() ? 1 : 0, cb.wasConditionMet() ? 1 : 0, cb.isAutomatic() ? 1 : 0, out));
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
        // wp49: a crafter's switched-off slots (a bit each), its crafting countdown and whether it is powered.
        if (be instanceof net.minecraft.world.level.block.entity.CrafterBlockEntity cb) {
            int mask = 0;
            for (int i = 0; i < 9; i++) if (cb.isSlotDisabled(i)) mask |= 1 << i;
            m.put("crafter", List.of(mask, field(be, "craftingTicksRemaining"), cb.isTriggered() ? 1 : 0));
        }
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

    /// The light engine runs on its own thread: the light a roof takes has to be settled before the tick.
    static void awaitLight(ServerLevel level) {
        var engine = level.getChunkSource().getLightEngine();
        for (int i = 0; i < 2; i++) {
            engine.tryScheduleUpdate();
            var done = engine.waitForPendingTasks(0, 0);
            long deadline = System.nanoTime() + 60_000_000_000L;
            level.getServer().managedBlock(() -> done.isDone() || System.nanoTime() > deadline);
            if (!done.isDone()) throw new IllegalStateException("light engine did not settle");
        }
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
        java.util.Set<java.util.UUID> seenBees = new java.util.HashSet<>();
        java.util.Set<java.util.UUID> seenEntities = new java.util.HashSet<>();
        // Whole level ticks while the server's own ticking stays frozen: `runsNormally` is what
        // `ServerLevel.tick` checks (the manager only updates it in its own tick).
        setRunsNormally(level, true);
        try {
            for (int t = 1; t <= s.ticks; t++) {
                for (String cmd : s.actions.getOrDefault(t, List.of())) command(server, absolute(cmd));
                if (s.align20) awaitLight(level);
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
                if (s.watchBees) {
                    // The bees that appeared this tick (where they were put) and how many there are.
                    var box = new net.minecraft.world.phys.AABB(BASE[0] - 8, BASE[1] - 8, BASE[2] - 8, BASE[0] + 12, BASE[1] + 12, BASE[2] + 12);
                    var all = level.getEntitiesOfClass(net.minecraft.world.entity.animal.bee.Bee.class, box);
                    List<List<Double>> fresh = new ArrayList<>();
                    for (var bee : all) if (seenBees.add(bee.getUUID())) fresh.add(List.of(bee.getX(), bee.getY(), bee.getZ()));
                    fresh.sort(Comparator.<List<Double>>comparingDouble(l -> l.get(0)).thenComparingDouble(l -> l.get(1)).thenComparingDouble(l -> l.get(2)));
                    tick.put("bees_new", new ArrayList<Object>(fresh));
                    tick.put("bee_count", all.size());
                }
                if (s.watchEntities) {
                    var box = new net.minecraft.world.phys.AABB(BASE[0] - 8, BASE[1] - 8, BASE[2] - 8, BASE[0] + 12, BASE[1] + 12, BASE[2] + 12);
                    List<List<Object>> fresh = new ArrayList<>();
                    for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.Entity.class, box)) {
                        if (e instanceof net.minecraft.world.entity.item.ItemEntity || e instanceof net.minecraft.world.entity.player.Player) continue;
                        if (seenEntities.add(e.getUUID())) {
                            // (A primed TNT hops a random way at its making: only where it is to a tenth is compared.)
                            double q = e instanceof net.minecraft.world.entity.item.PrimedTnt || e.getType() == net.minecraft.world.entity.EntityTypes.SULFUR_CUBE ? 10.0 : 10000.0;
                            // (A shot thing is spread by its own unseeded random: only that it is there is compared.)
                            if (e instanceof net.minecraft.world.entity.projectile.Projectile) {
                                fresh.add(List.of(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString(), 0.0, 0.0, 0.0));
                                continue;
                            }
                            fresh.add(List.of(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString(), Math.round(e.getX() * q) / q,
                                    Math.round(e.getY() * 10000) / 10000.0, Math.round(e.getZ() * q) / q));
                        }
                    }
                    fresh.sort(Comparator.<List<Object>, String>comparing(l -> (String) l.get(0)).thenComparingDouble(l -> (Double) l.get(1))
                            .thenComparingDouble(l -> (Double) l.get(2)).thenComparingDouble(l -> (Double) l.get(3)));
                    tick.put("entities_new", new ArrayList<Object>(fresh));
                }
                if (s.track) {
                    var box = new net.minecraft.world.phys.AABB(BASE[0] - 8, BASE[1] - 8, BASE[2] - 8, BASE[0] + 12, BASE[1] + 12, BASE[2] + 12);
                    List<List<Object>> all = new ArrayList<>();
                    for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.Entity.class, box)) {
                        if (e instanceof net.minecraft.world.entity.item.ItemEntity || e instanceof net.minecraft.world.entity.player.Player) continue;
                        var d = e.getDeltaMovement();
                        all.add(List.of(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString(), e.getX(), e.getY(), e.getZ(), d.x, d.y, d.z,
                                e instanceof net.minecraft.world.entity.LivingEntity l ? (double) l.getHealth() : -1.0));
                    }
                    all.sort(Comparator.<List<Object>, String>comparing(l -> (String) l.get(0)).thenComparingDouble(l -> (Double) l.get(1))
                            .thenComparingDouble(l -> (Double) l.get(3)).thenComparingDouble(l -> (Double) l.get(2)));
                    tick.put("track", new ArrayList<Object>(all));
                }
                if (s.watchMobs) {
                    var box = new net.minecraft.world.phys.AABB(BASE[0] - 8, BASE[1] - 8, BASE[2] - 8, BASE[0] + 12, BASE[1] + 12, BASE[2] + 12);
                    List<Object> sigs = new ArrayList<>();
                    for (var e : level.getEntitiesOfClass(net.minecraft.world.entity.LivingEntity.class, box)) {
                        if (e instanceof net.minecraft.world.entity.player.Player) continue;
                        StringBuilder sb = new StringBuilder(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString());
                        List<String> parts = new ArrayList<>();
                        for (var slot : net.minecraft.world.entity.EquipmentSlot.values()) {
                            var it = e.getItemBySlot(slot);
                            if (!it.isEmpty()) parts.add(slot.getName() + "=" + BuiltInRegistries.ITEM.getKey(it.getItem()) + "*" + it.getCount());
                        }
                        java.util.Collections.sort(parts);
                        for (String part : parts) sb.append('|').append(part);
                        if (e instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse h && h.hasChest()) sb.append("|chest");
                        sigs.add(sb.toString());
                    }
                    sigs.sort(Comparator.comparing(o -> (String) o));
                    tick.put("mobs", sigs);
                }
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
        // (Killed bees would linger for their death animation: they are discarded.)
        for (var bee : level.getEntitiesOfClass(net.minecraft.world.entity.animal.bee.Bee.class, new net.minecraft.world.phys.AABB(-64, -64, -64, 64, 320, 64))) bee.discard();
        command(server, "kill @e[type=tnt]");
        command(server, "kill @e[type=arrow]");
        command(server, "kill @e[type=spectral_arrow]");
        command(server, "kill @e[type=snowball]");
        // (wp49: boats, armor stands and what else a dispenser put out.)
        // (A killed mob lingers for its death animation: whatever is left is discarded.)
        for (var left : level.getEntitiesOfClass(net.minecraft.world.entity.Entity.class, new net.minecraft.world.phys.AABB(-64, -64, -64, 64, 320, 64))) {
            if (!(left instanceof net.minecraft.world.entity.player.Player)) left.discard();
        }
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
