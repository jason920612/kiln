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
        return out;
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
                "server-port=" + System.getenv().getOrDefault("KILN_HARNESS_PORT", "25597"),
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
        if (!(be instanceof Container c)) return m;
        List<Object> items = new ArrayList<>();
        for (int i = 0; i < c.getContainerSize(); i++) {
            ItemStack st = c.getItem(i);
            if (!st.isEmpty()) items.add(List.of(i, BuiltInRegistries.ITEM.getKey(st.getItem()).toString(), st.getCount()));
        }
        m.put("items", items);
        if (be instanceof HopperBlockEntity) m.put("cooldown", field(be, "cooldownTime"));
        if (be instanceof AbstractFurnaceBlockEntity) {
            m.put("furnace", List.of(field(be, "litTimeRemaining"), field(be, "litTotalTime"), field(be, "cookingTimer"),
                    field(be, "cookingTotalTime")));
        }
        return m;
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
}
