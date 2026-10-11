// Runs layouts of redstone machines (tools of wp55: kiln-blocks' `flyer_dump`) in a vanilla 26.3 server and compares what they do with what
// Kiln's pistons and observers did: where the west edge of the machine is at tick 40 and at tick 120. A layout that flies (the edge keeps
// moving) in vanilla and not in Kiln, or the other way round, is a difference in Kiln's piston or observer code.
//
// Input lines: {"rows":[[..],[..]],"last":[z,i],"x40":N,"x120":N} as `flyer_dump` writes them (kinds: 0 air, 1 slime, 2/3 sticky piston
// east/west, 6/7/8/9 observer east/west/north/south, 10/11 observer up/down); the machine stands at x = 16 + i, y = 100, z = 8 + row, the
// piece `last` is placed last (with updates), the others without.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/FlyerCheck.java <in.jsonl> <out.jsonl> [limit]

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.storage.ServerLevelData;

public class FlyerCheck {
    static MinecraftServer server;
    static ServerLevel level;
    static final int QUIET = 818;
    static final String[] KINDS = {
            "minecraft:air", "minecraft:slime_block", "minecraft:sticky_piston[facing=east,extended=false]", "minecraft:sticky_piston[facing=west,extended=false]",
            "minecraft:piston[facing=east,extended=false]", "minecraft:piston[facing=west,extended=false]",
            "minecraft:observer[facing=east,powered=false]", "minecraft:observer[facing=west,powered=false]",
            "minecraft:observer[facing=north,powered=false]", "minecraft:observer[facing=south,powered=false]",
            "minecraft:observer[facing=up,powered=false]", "minecraft:observer[facing=down,powered=false]"};

    static BlockState state(int kind) throws Exception {
        return BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, KINDS[kind], false).blockState();
    }

    static void tick() throws Exception {
        var f = net.minecraft.world.TickRateManager.class.getDeclaredField("runGameElements");
        f.setAccessible(true);
        f.set(level.tickRateManager(), true);
        try {
            level.tick(() -> true);
        } finally {
            f.set(level.tickRateManager(), false);
        }
    }

    static int[] measure() {
        int min = Integer.MAX_VALUE, count = 0;
        long h = 0;
        for (int x = 0; x < 80; x++)
            for (int z = 6; z < 12; z++)
                for (int y = 98; y < 103; y++) {
                    BlockState s = level.getBlockState(new BlockPos(x, y, z));
                    if (!s.isAir() && !s.is(Blocks.PISTON_HEAD) && !s.is(Blocks.MOVING_PISTON)) {
                        min = Math.min(min, x);
                        count++;
                        h = h * 1_000_003L + BuiltInRegistries.BLOCK.getId(s.getBlock()) * 31L + x * 77L + z * 13L + y;
                    }
                }
        return new int[] {min, count, (int) (h ^ (h >>> 32))};
    }

    static void clear() throws Exception {
        for (int x = 0; x < 96; x++)
            for (int z = 4; z < 14; z++)
                for (int y = 96; y < 106; y++) {
                    BlockPos p = new BlockPos(x, y, z);
                    if (!level.getBlockState(p).isAir()) level.setBlock(p, Blocks.AIR.defaultBlockState(), QUIET);
                }
        // whatever is still scheduled or moving runs out
        for (int i = 0; i < 12; i++) tick();
    }

    public static void main(String[] args) {
        try {
            main0(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static int field(String line, String key) {
        int i = line.indexOf("\"" + key + "\":") + key.length() + 3;
        int j = i;
        while (j < line.length() && (Character.isDigit(line.charAt(j)) || line.charAt(j) == '-')) j++;
        return Integer.parseInt(line.substring(i, j));
    }

    static int[][] ints(String s) {
        // "rows":[[a,b,..],[..]]
        int i = s.indexOf("\"rows\":") + 7;
        int end = s.indexOf("],\"last\"");
        String body = s.substring(i + 1, end + 1);
        String[] parts = body.split("\\],\\[");
        int[][] out = new int[parts.length][];
        for (int k = 0; k < parts.length; k++) {
            String p = parts[k].replace("[", "").replace("]", "").trim();
            String[] f = p.isEmpty() ? new String[0] : p.split(",\\s*");
            out[k] = new int[f.length];
            for (int m = 0; m < f.length; m++) out[k][m] = Integer.parseInt(f[m].trim());
        }
        return out;
    }

    static void main0(String[] args) throws Exception {
        Path in = Path.of(args[0]).toAbsolutePath(), out = Path.of(args[1]).toAbsolutePath();
        int limit = args.length > 2 ? Integer.parseInt(args[2]) : Integer.MAX_VALUE;
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n", "server-port=" + System.getenv("KILN_HARNESS_PORT"), "online-mode=false", "level-name=world",
                "level-type=minecraft\\:flat", "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:the_void\"}",
                "spawn-protection=0", "max-tick-time=-1", "view-distance=3", "simulation-distance=3", "sync-chunk-writes=false", "generate-structures=false", "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
        new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "FlyerCheck main").start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -1; cx <= 6; cx++)
                for (int cz = -1; cz <= 1; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(1000);
        List<String> lines = Files.readAllLines(in);
        int[] counts = new int[4];
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            for (int n = 0; n < lines.size() && n < limit; n++) {
                String line = lines.get(n);
                int[][] rows = ints(line);
                int idx = line.indexOf("\"last\":[") + 8;
                String[] lf = line.substring(idx, line.indexOf("]", idx)).split(",");
                final int lastZ = Integer.parseInt(lf[0].trim());
                final int lastI = Integer.parseInt(lf[1].trim());
                int kx40 = field(line, "x40"), kx120 = field(line, "x120");
                int[] vanilla = new int[5];
                int finalN = n;
                server.submit(() -> {
                    try {
                        clear();
                        BlockPos trigger = null;
                        BlockState trigState = null;
                        int pieces = 0;
                        for (int z = 0; z < rows.length; z++)
                            for (int i = 0; i < rows[z].length; i++) {
                                if (rows[z][i] == 0) continue;
                                pieces++;
                                BlockPos p = new BlockPos(16 + i, 100, 8 + z);
                                BlockState s = state(rows[z][i]);
                                if (z == lastZ && i == lastI) {
                                    trigger = p;
                                    trigState = s;
                                } else {
                                    level.setBlock(p, s, QUIET);
                                }
                            }
                        level.setBlock(trigger, trigState, 3);
                        int[] at40 = null, at110 = null;
                        for (int t = 1; t <= 120; t++) {
                            tick();
                            if (System.getenv("FLYER_DEBUG") != null && (t <= 12 || t % 10 == 0) && finalN < 3) {
                                StringBuilder sb = new StringBuilder("FLYDBG n" + finalN + " t" + t + ":");
                                for (int zz = 6; zz < 12; zz++)
                                    for (int xx = 14; xx < 24; xx++) {
                                        BlockState bs = level.getBlockState(new BlockPos(xx, 100, zz));
                                        if (!bs.isAir()) sb.append(" (").append(xx).append(',').append(zz).append(')').append(BuiltInRegistries.BLOCK.getKey(bs.getBlock()).getPath());
                                    }
                                System.out.println(sb);
                            }
                            if (t == 40) at40 = measure();
                            if (t == 110) at110 = measure();
                        }
                        int[] end = measure();
                        vanilla[0] = at40[0];
                        vanilla[1] = end[0];
                        vanilla[2] = end[1] == pieces ? 1 : 0;
                        vanilla[3] = end[2] != at110[2] ? 1 : 0;
                        vanilla[4] = pieces;
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                }).get();
                // (Kiln's lines for machines that fell apart carry its measurement of what was left; compared the same way)
                boolean kilnIntact = line.contains("\"intact\":1") || !line.contains("\"intact\"");
                boolean same = vanilla[0] == kx40 && vanilla[1] == kx120 && (kilnIntact == (vanilla[2] == 1));
                boolean flies = vanilla[2] == 1 && Math.abs(vanilla[1] - vanilla[0]) >= 3;
                counts[0]++;
                if (!same) counts[1]++;
                if (flies) counts[2]++;
                if (!same || flies) {
                    w.println("{\"n\":" + n + ",\"line\":" + line.replace("\n", "") + ",\"vanilla\":[" + vanilla[0] + "," + vanilla[1] + "," + vanilla[2] + "," + vanilla[3] + "],\"same\":" + same + ",\"flies\":" + flies + "}");
                    w.flush();
                }
                if (n % 200 == 0) System.out.println("FlyerCheck: " + n + " layouts, " + counts[1] + " differ, " + counts[2] + " fly");
            }
        }
        System.out.println("FlyerCheck: " + counts[0] + " layouts, " + counts[1] + " differ from Kiln, " + counts[2] + " fly in vanilla");
        server.halt(false);
        System.exit(0);
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
}
