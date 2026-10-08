// Differential test vectors for Kiln's weather, precipitation, sky darkening, the night skip
// and getting out of bed, recorded in a real vanilla 26.3 dedicated server started in-process.
//
// Scenarios (one JSON line each):
// - "cycle": WeatherData counters and the overworld's rain and thunder levels after every
//   advanceWeatherCycle, from a given state with the level random seeded.
// - "sky": getSkyDarken for clock times and rain and thunder levels.
// - "precipitation": blocks after rounds of tickPrecipitation over an area of fixed biomes
//   (fillbiome) with water, cauldrons and snow, the level random seeded.
// - "wake": the overworld clock after moveToTimeMarker(wake_up_from_sleep), then the weather
//   counters after resetWeatherCycle and one advanceWeatherCycle.
// - "standup": AbstractBedBlock.findStandUpPosition around beds in different surroundings.
//
// usage (cwd = a scratch server directory, e.g. work/wx-weather/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/WeatherVectors.java <out.jsonl>
// (tools/weather_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.clock.ClockTimeMarkers;
import net.minecraft.world.clock.WorldClock;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.AbstractBedBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.saveddata.WeatherData;
import net.minecraft.world.phys.Vec3;

public class WeatherVectors {
    static ServerLevel level;
    static MinecraftServer server;

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
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "WeatherVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
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
            try {
                command("gamerule advance_weather true");
                command("gamerule advance_time false");
                Map<String, Object> info = new LinkedHashMap<>();
                info.put("name", "info");
                info.put("random", level.getRandom().getClass().getName());
                info.put("sea_level", level.getSeaLevel());
                info.put("seed", level.getSeed());
                lines.add(toJson(info));
                cycles(lines);
                sky(lines);
                precipitation(lines);
                wake(lines);
                standup(lines);
            } catch (Throwable t) {
                t.printStackTrace();
                lines.add("{\"name\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("WeatherVectors: wrote " + lines.size() + " lines to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    // ---------------------------------------------------------------- weather cycle

    static Field levelField(String name) throws Exception {
        Field f = Level.class.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static void setLevels(float rain, float thunder) throws Exception {
        levelField("rainLevel").setFloat(level, rain);
        levelField("oRainLevel").setFloat(level, rain);
        levelField("thunderLevel").setFloat(level, thunder);
        levelField("oThunderLevel").setFloat(level, thunder);
    }

    static List<Object> state() throws Exception {
        WeatherData w = level.getWeatherData();
        List<Object> s = new ArrayList<>();
        s.add(w.getClearWeatherTime());
        s.add(w.getRainTime());
        s.add(w.getThunderTime());
        s.add(w.isRaining());
        s.add(w.isThundering());
        s.add(levelField("rainLevel").getFloat(level));
        s.add(levelField("thunderLevel").getFloat(level));
        s.add(level.isRaining());
        s.add(level.isThundering());
        return s;
    }

    static void cycles(List<String> lines) throws Exception {
        Method advance = ServerLevel.class.getDeclaredMethod("advanceWeatherCycle");
        advance.setAccessible(true);
        // (clear, rain, thunder, raining, thundering, rain level, thunder level, seed, ticks)
        Object[][] starts = {
            {0, 0, 0, false, false, 0f, 0f, 1L, 5},
            {0, 3, 7, true, false, 0.5f, 0f, 2L, 130},
            {0, 5, 2, true, true, 1f, 1f, 3L, 140},
            {3, 100, 100, true, true, 1f, 0.95f, 4L, 110},
            {0, 1, 1, false, false, 0.19f, 0f, 5L, 40},
            {0, 0, 0, true, true, 0f, 0f, 6L, 30},
            {0, 40, 0, true, false, 0.3f, 0.1f, 12345L, 60},
        };
        int n = 0;
        for (Object[] st : starts) {
            WeatherData w = level.getWeatherData();
            w.setClearWeatherTime((Integer) st[0]);
            w.setRainTime((Integer) st[1]);
            w.setThunderTime((Integer) st[2]);
            w.setRaining((Boolean) st[3]);
            w.setThundering((Boolean) st[4]);
            setLevels((Float) st[5], (Float) st[6]);
            long seed = (Long) st[7];
            level.getRandom().setSeed(seed);
            List<Object> ticks = new ArrayList<>();
            for (int t = 0; t < (Integer) st[8]; t++) {
                advance.invoke(level);
                ticks.add(state());
            }
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", "cycle" + n++);
            m.put("start", java.util.Arrays.asList(st));
            m.put("ticks", ticks);
            lines.add(toJson(m));
        }
    }

    // ---------------------------------------------------------------- sky darkening

    static Holder<WorldClock> clock() {
        return level.dimensionType().defaultClock().orElseThrow();
    }

    static void sky(List<String> lines) throws Exception {
        List<Object> samples = new ArrayList<>();
        long[] times = {0, 133, 1000, 6000, 11867, 12000, 12500, 13000, 13670, 14000, 18000, 22330, 23000, 23500, 23999, 30000};
        float[][] weather = {{0f, 0f}, {1f, 0f}, {0.5f, 0f}, {1f, 1f}, {1f, 0.5f}, {0.3f, 0.2f}, {0.9f, 0.9f}};
        for (long t : times) {
            server.clockManager().setTotalTicks(clock(), t);
            for (float[] w : weather) {
                setLevels(w[0], w[1]);
                level.environmentAttributes().invalidateTickCache();
                level.updateSkyBrightness();
                samples.add(List.of(t, w[0], w[1], level.getSkyDarken()));
            }
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", "sky");
        m.put("samples", samples);
        lines.add(toJson(m));
    }

    // ---------------------------------------------------------------- precipitation

    static void precipitation(List<String> lines) throws Exception {
        // Areas 16x16 at y 100 on a stone floor: (x0, z0, biome).
        Object[][] areas = {
            {0, 0, "minecraft:snowy_plains"},
            {16, 0, "minecraft:plains"},
            {0, 16, "minecraft:frozen_ocean"},
            {16, 16, "minecraft:ice_spikes"},
        };
        int[] maxSnow = {1, 3};
        int scenario = 0;
        for (int ms : maxSnow) {
            for (Object[] a : areas) {
                int x0 = (Integer) a[0], z0 = (Integer) a[1];
                String biome = (String) a[2];
                command(String.format("fill %d 99 %d %d 99 %d minecraft:stone", x0, z0, x0 + 15, z0 + 15));
                command(String.format("fill %d 100 %d %d 110 %d minecraft:air", x0, z0, x0 + 15, z0 + 15));
                command(String.format("fillbiome %d 96 %d %d 112 %d %s", x0, z0, x0 + 15, z0 + 15, biome));
                // A water pool, cauldrons of every kind, snow, slabs, glass.
                command(String.format("fill %d 99 %d %d 99 %d minecraft:water", x0 + 1, z0 + 1, x0 + 5, z0 + 5));
                command(String.format("setblock %d 100 %d minecraft:cauldron", x0 + 8, z0 + 1));
                command(String.format("setblock %d 100 %d minecraft:water_cauldron[level=1]", x0 + 9, z0 + 1));
                command(String.format("setblock %d 100 %d minecraft:water_cauldron[level=2]", x0 + 10, z0 + 1));
                command(String.format("setblock %d 100 %d minecraft:powder_snow_cauldron[level=1]", x0 + 11, z0 + 1));
                command(String.format("setblock %d 100 %d minecraft:water_cauldron[level=3]", x0 + 12, z0 + 1));
                command(String.format("fill %d 100 %d %d 100 %d minecraft:snow[layers=2]", x0 + 8, z0 + 4, x0 + 12, z0 + 5));
                command(String.format("fill %d 100 %d %d 100 %d minecraft:stone_slab[type=bottom]", x0 + 8, z0 + 8, x0 + 12, z0 + 9));
                command(String.format("fill %d 100 %d %d 100 %d minecraft:glass", x0 + 1, z0 + 10, x0 + 3, z0 + 12));
                command(String.format("fill %d 100 %d %d 100 %d minecraft:ice", x0 + 5, z0 + 10, x0 + 6, z0 + 12));
                command("gamerule max_snow_accumulation_height " + ms);
                setLevels(1f, 0f);
                level.getRandom().setSeed(1000 + scenario);
                for (int round = 0; round < 12; round++) {
                    for (int dz = 0; dz < 16; dz++)
                        for (int dx = 0; dx < 16; dx++)
                            level.tickPrecipitation(new BlockPos(x0 + dx, 0, z0 + dz));
                }
                List<Object> cols = new ArrayList<>();
                for (int dz = 0; dz < 16; dz++)
                    for (int dx = 0; dx < 16; dx++) {
                        List<Object> col = new ArrayList<>();
                        for (int y = 99; y <= 101; y++)
                            col.add(BlockStateParser.serialize(level.getBlockState(new BlockPos(x0 + dx, y, z0 + dz))));
                        cols.add(col);
                    }
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("name", "precipitation" + scenario);
                m.put("x0", x0);
                m.put("z0", z0);
                m.put("biome", biome);
                m.put("max_snow", ms);
                m.put("seed", 1000 + scenario);
                m.put("rounds", 12);
                m.put("columns", cols);
                lines.add(toJson(m));
                scenario++;
            }
        }
        setLevels(0f, 0f);
    }

    // ---------------------------------------------------------------- night skip

    static void wake(List<String> lines) throws Exception {
        Method advance = ServerLevel.class.getDeclaredMethod("advanceWeatherCycle");
        advance.setAccessible(true);
        long[] times = {13000, 18000, 23999, 24000, 24001, 47000, 100000};
        List<Object> out = new ArrayList<>();
        int n = 0;
        for (long t : times) {
            server.clockManager().setTotalTicks(clock(), t);
            Object result = server.clockManager().moveToTimeMarker(clock(), ClockTimeMarkers.WAKE_UP_FROM_SLEEP);
            long after = server.clockManager().getInstance(clock()).totalTicks();
            WeatherData w = level.getWeatherData();
            w.setRaining(true);
            w.setThundering(true);
            w.setRainTime(500);
            w.setThunderTime(300);
            setLevels(1f, 1f);
            level.resetWeatherCycle();
            level.getRandom().setSeed(77 + n++);
            advance.invoke(level);
            out.add(List.of(t, result.toString(), after, state()));
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", "wake");
        m.put("samples", out);
        lines.add(toJson(m));
    }

    // ---------------------------------------------------------------- getting up

    static void standup(List<String> lines) throws Exception {
        // Beds at y 100 on stone, with surroundings: (label, extra commands relative to head).
        int x0 = 40, z0 = 0;
        String[][] layouts = {
            {"open"},
            {"wall_south", "fill ~-3 ~ ~1 ~3 ~1 ~1 minecraft:stone"},
            {"boxed", "fill ~-3 ~ ~-1 ~3 ~1 ~-1 minecraft:stone", "fill ~-3 ~ ~1 ~3 ~1 ~1 minecraft:stone"},
            {"lava_side", "fill ~-3 ~-1 ~1 ~3 ~-1 ~1 minecraft:lava"},
            {"closed", "fill ~-3 ~ ~-1 ~3 ~1 ~1 minecraft:stone", "setblock ~ ~ ~ minecraft:red_bed[part=head,facing=east]", "setblock ~-1 ~ ~ minecraft:red_bed[part=foot,facing=east]"},
        };
        float[] yaws = {0f, 90f, 180f, -90f, 45f};
        List<Object> out = new ArrayList<>();
        int i = 0;
        for (String[] layout : layouts) {
            int hx = x0 + i * 10, hz = z0;
            i++;
            command(String.format("fill %d 99 %d %d 99 %d minecraft:stone", hx - 5, hz - 5, hx + 5, hz + 5));
            command(String.format("fill %d 100 %d %d 103 %d minecraft:air", hx - 5, hz - 5, hx + 5, hz + 5));
            command(String.format("setblock %d 100 %d minecraft:red_bed[part=head,facing=east]", hx, hz));
            command(String.format("setblock %d 100 %d minecraft:red_bed[part=foot,facing=east]", hx - 1, hz));
            for (int k = 1; k < layout.length; k++) {
                command(String.format("execute positioned %d 100 %d run %s", hx, hz, layout[k]));
            }
            List<Object> r = new ArrayList<>();
            for (float yaw : yaws) {
                var v = AbstractBedBlock.findStandUpPosition(EntityTypes.PLAYER, level, new BlockPos(hx, 100, hz), Direction.EAST, yaw);
                r.add(v.map(p -> (Object) List.of(p.x() - hx, p.y(), p.z() - hz)).orElse(null));
            }
            out.add(List.of(layout[0], r));
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", "standup");
        m.put("layouts", layouts.length);
        m.put("yaws", List.of(0f, 90f, 180f, -90f, 45f));
        m.put("samples", out);
        lines.add(toJson(m));
    }

    // ---------------------------------------------------------------- server

    static void command(String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
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

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof Float f) return Float.toString(f);
        if (o instanceof Double d) return Double.toString(d);
        if (o instanceof Object[] a) return toJson(java.util.Arrays.asList(a));
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
        return toJson(String.format(Locale.ROOT, "%s", o));
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
