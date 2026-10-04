// Differential test vectors for Kiln's fire (FireBlock.tick: aging, burning neighbours away,
// spreading, burning out; the faces fire takes; onPlace scheduling), recorded in a real
// vanilla 26.3 dedicated server started in-process.
//
// Each scenario builds blocks in a 16x16 area on a stone floor at y 99, seeds the level
// random and then runs rounds: every fire of the area (in y, z, x order, found at the start
// of the round) that is still fire gets BlockState.tick with the level random. Recorded: the
// area before the first round and after every round (non-air blocks above the floor), and a
// draw of the level random after each round (so every random call is accounted for).
// Block ticks do not run on their own (the tick rate manager is frozen) and no player is near,
// so fire_spread_radius_around_player is -1.
//
// usage (cwd = a scratch server directory, e.g. work/wp-fire/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/FireVectors.java <out.jsonl>
// (tools/fire_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

public class FireVectors {
    static ServerLevel level;
    static MinecraftServer server;
    static final int Y0 = 100, HEIGHT = 10;

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
        }, "FireVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -2; cx <= 6; cx++)
                for (int cz = -2; cz <= 2; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(2000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                command("gamerule fire_spread_radius_around_player -1");
                command("gamerule advance_weather false");
                command("weather clear");
                scenarios(lines);
            } catch (Throwable t) {
                t.printStackTrace();
                lines.add("{\"name\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("FireVectors: wrote " + lines.size() + " lines to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    // (name, difficulty, seed, rounds, commands relative to the area's corner at y 100)
    static final Object[][] SCENARIOS = {
        {"room", "normal", 11L, 40, new String[] {
            "fill ~2 ~ ~2 ~8 ~3 ~8 minecraft:oak_planks hollow",
            "fill ~2 ~ ~2 ~2 ~3 ~2 minecraft:oak_log",
            "fill ~8 ~ ~8 ~8 ~3 ~8 minecraft:spruce_log",
            "setblock ~4 ~ ~4 minecraft:white_wool",
            "setblock ~6 ~ ~6 minecraft:bookshelf",
            "setblock ~5 ~1 ~4 minecraft:hay_block",
            "setblock ~5 ~ ~5 minecraft:fire",
        }},
        {"forest", "hard", 22L, 40, new String[] {
            "fill ~1 ~ ~1 ~1 ~4 ~1 minecraft:oak_log",
            "fill ~0 ~5 ~0 ~2 ~6 ~2 minecraft:oak_leaves[persistent=true]",
            "fill ~5 ~ ~5 ~5 ~4 ~5 minecraft:birch_log",
            "fill ~4 ~5 ~4 ~6 ~6 ~6 minecraft:birch_leaves[persistent=true]",
            "fill ~0 ~ ~3 ~9 ~ ~3 minecraft:short_grass",
            "fill ~9 ~ ~0 ~9 ~ ~9 minecraft:oak_fence",
            "setblock ~3 ~ ~3 minecraft:fire",
            "setblock ~7 ~ ~7 minecraft:fire",
        }},
        {"easy_wool", "easy", 33L, 30, new String[] {
            "fill ~0 ~ ~0 ~9 ~ ~9 minecraft:red_wool",
            "fill ~2 ~1 ~2 ~7 ~1 ~7 minecraft:white_wool",
            "setblock ~5 ~1 ~5 minecraft:fire",
            "setblock ~0 ~1 ~0 minecraft:fire",
        }},
        {"netherrack", "normal", 44L, 50, new String[] {
            "fill ~0 ~-1 ~0 ~4 ~-1 ~4 minecraft:netherrack",
            "fill ~0 ~ ~0 ~4 ~ ~4 minecraft:fire",
            "fill ~6 ~ ~0 ~8 ~2 ~2 minecraft:oak_planks",
            "setblock ~5 ~ ~1 minecraft:fire",
        }},
        {"hanging", "normal", 55L, 40, new String[] {
            "fill ~0 ~-1 ~0 ~9 ~-1 ~9 minecraft:air",
            "fill ~4 ~ ~0 ~4 ~6 ~9 minecraft:jungle_planks",
            "fill ~3 ~ ~2 ~3 ~6 ~2 minecraft:air",
            "setblock ~3 ~3 ~3 minecraft:fire[east=true]",
            "setblock ~5 ~2 ~5 minecraft:fire[west=true]",
            "fill ~7 ~ ~0 ~7 ~6 ~9 minecraft:acacia_leaves[persistent=true]",
        }},
        {"waterlogged", "normal", 66L, 40, new String[] {
            "fill ~0 ~ ~0 ~9 ~ ~0 minecraft:oak_stairs[waterlogged=true]",
            "fill ~0 ~ ~2 ~9 ~ ~2 minecraft:oak_stairs",
            "fill ~0 ~ ~4 ~9 ~ ~4 minecraft:oak_slab[type=bottom,waterlogged=true]",
            "fill ~0 ~ ~6 ~9 ~ ~6 minecraft:oak_slab[type=bottom]",
            "fill ~0 ~ ~1 ~9 ~ ~1 minecraft:fire",
            "fill ~0 ~ ~5 ~9 ~ ~5 minecraft:fire",
            "setblock ~2 ~ ~8 minecraft:coal_block",
            "setblock ~3 ~ ~8 minecraft:fire",
        }},
        {"soul_and_old", "normal", 77L, 30, new String[] {
            "fill ~0 ~-1 ~0 ~2 ~-1 ~2 minecraft:soul_sand",
            "fill ~0 ~ ~0 ~2 ~ ~2 minecraft:soul_fire",
            "fill ~5 ~ ~5 ~7 ~ ~7 minecraft:fire[age=12]",
            "setblock ~6 ~1 ~6 minecraft:dried_kelp_block",
            "setblock ~8 ~ ~6 minecraft:target",
            "setblock ~5 ~ ~8 minecraft:scaffolding",
        }},
    };

    static void scenarios(List<String> lines) throws Exception {
        int i = 0;
        for (Object[] sc : SCENARIOS) {
            int x0 = i * 20, z0 = 0;
            i++;
            command("difficulty " + sc[1]);
            command(String.format("fill %d 99 %d %d 99 %d minecraft:stone", x0, z0, x0 + 15, z0 + 15));
            command(String.format("fill %d %d %d %d %d %d minecraft:air", x0, Y0, z0, x0 + 15, Y0 + HEIGHT - 1, z0 + 15));
            for (String c : (String[]) sc[4]) {
                command(String.format("execute positioned %d %d %d run %s", x0, Y0, z0, c));
            }
            long seed = (Long) sc[2];
            int rounds = (Integer) sc[3];
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", sc[0]);
            m.put("difficulty", level.getDifficulty().getId());
            m.put("seed", seed);
            m.put("x0", x0);
            m.put("z0", z0);
            m.put("initial", snapshot(x0, z0));
            level.getRandom().setSeed(seed);
            List<Object> after = new ArrayList<>();
            for (int r = 0; r < rounds; r++) {
                List<BlockPos> fires = new ArrayList<>();
                for (int y = Y0 - 1; y < Y0 + HEIGHT; y++)
                    for (int z = z0; z < z0 + 16; z++)
                        for (int x = x0; x < x0 + 16; x++) {
                            BlockPos p = new BlockPos(x, y, z);
                            if (level.getBlockState(p).is(Blocks.FIRE)) fires.add(p);
                        }
                for (BlockPos p : fires) {
                    BlockState s = level.getBlockState(p);
                    if (s.is(Blocks.FIRE)) s.tick(level, p, level.getRandom());
                }
                long probe = level.getRandom().nextLong();
                after.add(List.of(snapshot(x0, z0), probe));
            }
            m.put("rounds", after);
            lines.add(toJson(m));
        }
    }

    // Blocks from the floor up: [dx, dy, dz, state], without air above the floor and stone in it.
    static List<Object> snapshot(int x0, int z0) {
        List<Object> out = new ArrayList<>();
        for (int y = Y0 - 1; y < Y0 + HEIGHT; y++)
            for (int z = z0; z < z0 + 16; z++)
                for (int x = x0; x < x0 + 16; x++) {
                    BlockState s = level.getBlockState(new BlockPos(x, y, z));
                    if (y == Y0 - 1 ? s.is(Blocks.STONE) : s.isAir()) continue;
                    out.add(List.of(x - x0, y, z - z0, BlockStateParser.serialize(s)));
                }
        return out;
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
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            int i = 0;
            for (var e : m.entrySet()) b.append(i++ > 0 ? "," : "").append(toJson(e.getKey().toString())).append(':').append(toJson(e.getValue()));
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
