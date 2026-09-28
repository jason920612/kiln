// Differential test vectors for Kiln's game events, vibrations and sculk sensors. Runs
// scenarios in a real vanilla 26.3 dedicated server (started in-process): each scenario places
// its blocks with /setblock, then runs whole level ticks (ServerLevel.tick: scheduled ticks,
// block events and block entity tickers in vanilla's order) with commands before chosen ticks
// (`setblock ... destroy` makes a block_destroy game event, redstone blocks make note blocks
// play and pistons move), and records after every tick the watched block states and, for the
// watched sculk sensors, their vibration state: last frequency, the travelling vibration
// (event, distance, ticks left) and the selector's candidate (event, tick).
//
// Block entities tick in the order they were created; scenarios place their blocks sorted by
// position, which is the order Kiln ticks them in. Nothing here draws from a random that is
// recorded (the sensors' sound pitches do, but sounds are not compared).
//
// usage (cwd = a scratch server directory, e.g. work/m6s3-warden/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/SculkVectors.java <out.jsonl> [name-filter]
// (tools/sculk_vectors.py sets this up; the server listens on $KILN_MOB_PORT, default 25614)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.SculkSensorBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.gameevent.vibrations.VibrationInfo;
import net.minecraft.world.level.gameevent.vibrations.VibrationSelector;
import net.minecraft.world.level.gameevent.vibrations.VibrationSystem;

public class SculkVectors {
    static final int[] BASE = {0, 100, 0};

    // ---------------------------------------------------------------- scenario model

    static final class Scenario {
        final String name;
        int ticks;
        // Blocks relative to BASE: {dx, dy, dz, block argument of /setblock}.
        List<Object[]> blocks = new ArrayList<>();
        // Commands (relative coordinates as ~dx ~dy ~dz) before a tick.
        TreeMap<Integer, List<String>> actions = new TreeMap<>();
        List<int[]> sensors = new ArrayList<>();
        List<int[]> states = new ArrayList<>();

        Scenario(String name, int ticks) {
            this.name = name;
            this.ticks = ticks;
        }

        Scenario block(int x, int y, int z, String block) {
            blocks.add(new Object[] {x, y, z, block});
            return this;
        }

        /** A sculk sensor (or calibrated one), its state and vibration watched. */
        Scenario sensor(int x, int y, int z, String block) {
            sensors.add(new int[] {x, y, z});
            states.add(new int[] {x, y, z});
            return block(x, y, z, block);
        }

        Scenario sensor(int x, int y, int z) {
            return sensor(x, y, z, "minecraft:sculk_sensor");
        }

        Scenario state(int x, int y, int z) {
            states.add(new int[] {x, y, z});
            return this;
        }

        Scenario at(int tick, String command) {
            actions.computeIfAbsent(tick, k -> new ArrayList<>()).add(command);
            return this;
        }

        /** `setblock ... air destroy` of the block at (x, y, z) before `tick`. */
        Scenario destroy(int tick, int x, int y, int z) {
            return at(tick, String.format(Locale.ROOT, "setblock ~%d ~%d ~%d air destroy", x, y, z));
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
            m.put("sensors", positions(sensors));
            m.put("states", positions(states));
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

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        // A broken block heard at every distance up to 8 (and one diagonal): travel time and
        // power by distance; the sensor 9 away hears nothing.
        {
            Scenario s = new Scenario("distances", 60).block(0, 0, 0, "minecraft:stone");
            for (int d = 1; d <= 9; d++) s.sensor(0, 0, d);
            s.sensor(3, 2, 1);
            s.destroy(3, 0, 0, 0);
            out.add(s);
        }
        // Calibrated sensors hear within 16 blocks; the one 17 away does not.
        out.add(new Scenario("calibrated_range", 50)
                .block(0, 0, 0, "minecraft:stone")
                .sensor(0, 0, 10, "minecraft:calibrated_sculk_sensor[facing=north]")
                .sensor(0, 0, 16, "minecraft:calibrated_sculk_sensor[facing=north]")
                .sensor(0, 0, -17, "minecraft:calibrated_sculk_sensor[facing=south]")
                .destroy(2, 0, 0, 0));
        // A wool wall stops a vibration; one gap lets it through to the second sensor.
        {
            Scenario s = new Scenario("wool_wall", 40).block(0, 0, 0, "minecraft:stone").block(6, 0, 0, "minecraft:stone");
            for (int y = -1; y <= 2; y++)
                for (int x = -2; x <= 8; x++) s.block(x, y, 2, "minecraft:white_wool");
            s.block(6, 0, 2, "minecraft:air");
            s.sensor(0, 0, 4).sensor(6, 0, 4);
            s.destroy(2, 0, 0, 0).destroy(2, 6, 0, 0);
            out.add(s);
        }
        // Breaking wool (`#dampens_vibrations`) makes no vibration; breaking stone does.
        out.add(new Scenario("dampened", 40)
                .block(0, 0, 0, "minecraft:white_wool")
                .block(0, 0, 1, "minecraft:stone")
                .sensor(0, 0, 4)
                .destroy(2, 0, 0, 0)
                .destroy(12, 0, 0, 1));
        // Two blocks broken in the same tick: the closer one wins the selector.
        out.add(new Scenario("closest_wins", 40)
                .block(0, 0, 0, "minecraft:stone")
                .block(0, 0, 6, "minecraft:stone")
                .sensor(0, 0, 3)
                .destroy(2, 0, 0, 6)
                .destroy(2, 0, 0, 0));
        // A vibration while one travels, and while the sensor is active or cooling, is lost.
        out.add(new Scenario("busy", 90)
                .block(0, 0, 0, "minecraft:stone")
                .block(1, 0, 0, "minecraft:stone")
                .block(2, 0, 0, "minecraft:stone")
                .block(3, 0, 0, "minecraft:stone")
                .block(4, 0, 0, "minecraft:stone")
                .sensor(0, 0, 7)
                .destroy(2, 0, 0, 0)
                .destroy(5, 1, 0, 0)
                .destroy(20, 2, 0, 0)
                .destroy(45, 3, 0, 0)
                .destroy(60, 4, 0, 0));
        // Note blocks play (frequency 10) when a redstone block powers them; the sensor's
        // comparator reads the frequency, its dust and lamp its power.
        out.add(new Scenario("note_block", 60)
                .block(0, 0, 0, "minecraft:note_block")
                .sensor(0, 0, 3)
                .block(1, 0, 3, "minecraft:comparator[facing=west]")
                .block(1, -1, 3, "minecraft:stone")
                .block(2, 0, 3, "minecraft:redstone_wire")
                .block(2, -1, 3, "minecraft:stone")
                .block(-1, 0, 3, "minecraft:redstone_lamp")
                .state(1, 0, 3).state(2, 0, 3).state(-1, 0, 3)
                .at(3, "setblock ~1 ~0 ~0 minecraft:redstone_block"));
        // A piston pushing (block_activate) and pulling back (block_deactivate).
        out.add(new Scenario("piston", 90)
                .block(0, 0, 0, "minecraft:piston[facing=up]")
                .sensor(0, 0, 5)
                .state(0, 0, 0).state(0, 1, 0)
                .at(3, "setblock ~1 ~0 ~0 minecraft:redstone_block")
                .at(50, "setblock ~1 ~0 ~0 minecraft:air"));
        // A calibrated sensor told frequency 12 by the dust behind it hears a broken block (12)
        // but not a note block (10).
        out.add(new Scenario("calibrated_filter", 90)
                .sensor(0, 0, 0, "minecraft:calibrated_sculk_sensor[facing=north]")
                .block(0, 0, 1, "minecraft:redstone_wire")
                .block(0, -1, 1, "minecraft:stone")
                .block(0, 0, 2, "minecraft:redstone_wire")
                .block(0, -1, 2, "minecraft:stone")
                .block(0, 0, 3, "minecraft:redstone_wire")
                .block(0, -1, 3, "minecraft:stone")
                .block(0, 0, 4, "minecraft:redstone_wire")
                .block(0, -1, 4, "minecraft:stone")
                .block(0, 0, 5, "minecraft:redstone_block")
                .block(4, 0, 0, "minecraft:note_block")
                .block(-4, 0, 0, "minecraft:stone")
                .state(0, 0, 1)
                .at(3, "setblock ~5 ~0 ~0 minecraft:redstone_block")
                .destroy(40, -4, 0, 0));
        // Amethyst next to an activated sensor resonates: a sensor far away hears the
        // resonance at the vibration's frequency.
        out.add(new Scenario("resonance", 80)
                .block(0, 0, 0, "minecraft:stone")
                .sensor(0, 0, 2)
                .block(0, 0, 3, "minecraft:amethyst_block")
                .sensor(0, 0, 10)
                .sensor(6, 0, 3)
                .destroy(2, 0, 0, 0));
        // An active sensor powers the block above strongly (a lamp on a block above lights) and
        // redstone dust beside it weakly.
        out.add(new Scenario("redstone_out", 60)
                .block(0, 0, 0, "minecraft:stone")
                .sensor(0, 0, 3)
                .block(0, 1, 3, "minecraft:redstone_lamp")
                .block(1, 0, 3, "minecraft:redstone_wire")
                .block(1, -1, 3, "minecraft:stone")
                .block(2, 0, 3, "minecraft:redstone_wire")
                .block(2, -1, 3, "minecraft:stone")
                .state(0, 1, 3).state(1, 0, 3).state(2, 0, 3)
                .destroy(2, 0, 0, 0));
        // A sensor set off by a sensor: the first one's output moves a piston next to the second.
        out.add(new Scenario("chain", 90)
                .block(0, 0, 0, "minecraft:stone")
                .sensor(0, 0, 2)
                .block(0, 0, 3, "minecraft:piston[facing=south]")
                .sensor(0, 0, 10)
                .state(0, 0, 3)
                .destroy(2, 0, 0, 0));
        return out;
    }

    // ---------------------------------------------------------------- main

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
        }, "SculkVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : scenarios()) {
            if (filter == null || s.name.contains(filter)) selected.add(s);
        }
        System.out.println("SculkVectors: " + selected.size() + " scenarios");
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -3; cx <= 3; cx++)
                for (int cz = -3; cz <= 3; cz++) {
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
        System.out.println("SculkVectors: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        String port = Optional.ofNullable(System.getenv("KILN_MOB_PORT")).orElse("25614");
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + port,
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:the_void\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=4",
                "simulation-distance=4",
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
        for (int i = 0; i < 3000; i++) {
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

    static String eventName(VibrationInfo v) {
        return BuiltInRegistries.GAME_EVENT.getKey(v.gameEvent().value()).toString();
    }

    /**
     * [last frequency, travelling event, its distance, ticks left, candidate event, its tick
     * counted from the scenario's start (-1: none)].
     */
    static List<Object> vibration(BlockEntity be, long start) throws Exception {
        if (!(be instanceof SculkSensorBlockEntity s)) return Arrays.asList(null, null, null, null, null, null);
        VibrationSystem.Data data = s.getVibrationData();
        VibrationInfo cur = data.getCurrentVibration();
        @SuppressWarnings("unchecked")
        Optional<Object> sel = (Optional<Object>) field(data.getSelectionStrategy(), "currentVibrationData");
        String selEvent = null;
        long selTick = -1;
        if (sel.isPresent()) {
            Object ev = sel.get();
            var eventAccessor = ev.getClass().getDeclaredMethod("event");
            var tickAccessor = ev.getClass().getDeclaredMethod("tick");
            eventAccessor.setAccessible(true);
            tickAccessor.setAccessible(true);
            selEvent = eventName((VibrationInfo) eventAccessor.invoke(ev));
            selTick = (Long) tickAccessor.invoke(ev) - start;
        }
        return Arrays.asList(s.getLastVibrationFrequency(), cur == null ? null : eventName(cur), cur == null ? null : (double) cur.distance(),
                data.getTravelTimeInTicks(), selEvent, selTick);
    }

    static String run(MinecraftServer server, Scenario s) throws Exception {
        ServerLevel level = server.overworld();
        for (Object[] b : s.sortedBlocks()) {
            command(server, String.format(Locale.ROOT, "setblock %d %d %d %s", BASE[0] + (int) b[0], BASE[1] + (int) b[1],
                    BASE[2] + (int) b[2], b[3]));
        }
        List<Object> ticks = new ArrayList<>();
        long start = level.getGameTime();
        setRunsNormally(level, true);
        try {
            for (int t = 1; t <= s.ticks; t++) {
                for (String cmd : s.actions.getOrDefault(t, List.of())) command(server, absolute(cmd));
                level.tick(() -> true);
                Map<String, Object> tick = new LinkedHashMap<>();
                List<Object> st = new ArrayList<>();
                for (int[] p : s.states) st.add(stateString(level.getBlockState(pos(p))));
                tick.put("states", st);
                List<Object> vs = new ArrayList<>();
                for (int[] p : s.sensors) vs.add(vibration(level.getBlockEntity(pos(p)), start));
                tick.put("sensors", vs);
                ticks.add(tick);
            }
        } finally {
            setRunsNormally(level, false);
        }
        // Clear the area (without drops) for the next scenario.
        command(server, String.format(Locale.ROOT, "fill %d %d %d %d %d %d air strict", BASE[0] - 12, BASE[1] - 2, BASE[2] - 20,
                BASE[0] + 12, BASE[1] + 4, BASE[2] + 20));
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
