// Runs machines of redstone pieces found by kiln-blocks' `flyer_search_3d` in a vanilla 26.3 server and prints where they went: vanilla is the
// judge of whether a layout flies. Input lines (`JSON ...` lines of the search, without the prefix):
//   {"pieces":[[x,y,z,"minecraft:sticky_piston[facing=north,extended=false]"],...],"start":[x,y,z],"tap":true,"move":[dx,dy,dz]}
// the machine stands at (24 + x, 100 + y, 8 + z); `start` is the piece placed last (with updates; the others without) or, with `tap`, a cell
// where a stone is put and taken away again. Output: the input line plus "vanilla":[dx,dy,dz] (the average movement of the pieces between tick
// 40 and tick 100, as Kiln measures it) and "intact".
//
// usage (cwd = a scratch server directory, KILN_HARNESS_PORT set):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/Flyer3D.java <in.jsonl> <out.jsonl>

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.concurrent.atomic.AtomicReference;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

public class Flyer3D {
    static MinecraftServer server;
    static ServerLevel level;
    static final int QUIET = 818;
    static final int OX = 24, OY = 100, OZ = 8;

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

    /** {sum x, sum y, sum z, count}: what is not air, a piston head or a moving piston, in x 0..63, y 70..129, z -8..23. */
    static long[] measure() {
        long[] r = new long[4];
        for (int x = 0; x < 64; x++)
            for (int z = -8; z < 24; z++)
                for (int y = 70; y < 130; y++) {
                    BlockState s = level.getBlockState(new BlockPos(x, y, z));
                    if (!s.isAir() && !s.is(Blocks.PISTON_HEAD) && !s.is(Blocks.MOVING_PISTON)) {
                        r[0] += x;
                        r[1] += y;
                        r[2] += z;
                        r[3]++;
                    }
                }
        return r;
    }

    static void clear() throws Exception {
        for (int x = 0; x < 64; x++)
            for (int z = -8; z < 24; z++)
                for (int y = 70; y < 130; y++) {
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

    static int[] ints(String line, String key) {
        Matcher m = Pattern.compile("\"" + key + "\":\\[(-?\\d+),(-?\\d+),(-?\\d+)\\]").matcher(line);
        if (!m.find()) throw new IllegalArgumentException(key + " in " + line);
        return new int[] {Integer.parseInt(m.group(1)), Integer.parseInt(m.group(2)), Integer.parseInt(m.group(3))};
    }

    static void main0(String[] args) throws Exception {
        Path in = Path.of(args[0]).toAbsolutePath(), out = Path.of(args[1]).toAbsolutePath();
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
        }, "Flyer3D main").start();
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
        int differ = 0, flies = 0;
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            for (String line : lines) {
                if (line.isBlank()) continue;
                Matcher pm = Pattern.compile("\\[(-?\\d+),(-?\\d+),(-?\\d+),\"([^\"]+)\"\\]").matcher(line.substring(line.indexOf("\"pieces\"")));
                List<Object[]> pieces = new ArrayList<>();
                while (pm.find()) pieces.add(new Object[] {Integer.parseInt(pm.group(1)), Integer.parseInt(pm.group(2)), Integer.parseInt(pm.group(3)), pm.group(4)});
                int[] start = ints(line, "start");
                boolean tap = line.contains("\"tap\":true");
                long[][] res = new long[2][];
                server.submit(() -> {
                    try {
                        clear();
                        if (System.getenv("FLYER_FLOOR") != null) {
                            for (int fx = 0; fx < 64; fx++) for (int fz = -8; fz < 24; fz++) level.setBlock(new BlockPos(fx, 99, fz), Blocks.STONE.defaultBlockState(), QUIET);
                        }
                        BlockPos st = new BlockPos(OX + start[0], OY + start[1], OZ + start[2]);
                        BlockPos trigger = null;
                        BlockState trigState = null;
                        for (Object[] p : pieces) {
                            BlockPos bp = new BlockPos(OX + (int) p[0], OY + (int) p[1], OZ + (int) p[2]);
                            BlockState s = BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, (String) p[3], false).blockState();
                            if (bp.equals(st) && !tap) {
                                trigger = bp;
                                trigState = s;
                            } else {
                                level.setBlock(bp, s, QUIET);
                            }
                        }
                        if (tap) {
                            level.setBlock(st, Blocks.STONE.defaultBlockState(), 3);
                            tick();
                            level.setBlock(st, Blocks.AIR.defaultBlockState(), 3);
                        }
                        if (trigger != null) level.setBlock(trigger, trigState, 3);
                        for (int t = 1; t <= 100; t++) {
                            tick();
                            if (t == 40) res[0] = measure();
                        }
                        res[1] = measure();
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                }).get();
                long n = res[1][3];
                boolean intact = n == pieces.size();
                long[] mv = new long[3];
                for (int a = 0; a < 3; a++) mv[a] = n == 0 ? 0 : (res[1][a] - res[0][a]) / n;
                boolean kilnNull = line.contains("\"move\":null");
                int[] kiln = kilnNull ? new int[3] : ints(line, "move");
                boolean same = kilnNull ? true : (intact && mv[0] == kiln[0] && mv[1] == kiln[1] && mv[2] == kiln[2]);
                boolean fly = intact && (Math.abs(mv[0]) >= 3 || Math.abs(mv[1]) >= 3 || Math.abs(mv[2]) >= 3);
                if (kilnNull && fly) same = false;
                if (!same) differ++;
                if (fly) flies++;
                w.println(line.substring(0, line.lastIndexOf('}')) + ",\"vanilla\":[" + mv[0] + "," + mv[1] + "," + mv[2] + "],\"intact\":" + intact + ",\"same\":" + same + "}");
                w.flush();
            }
        }
        System.out.println("Flyer3D: " + lines.size() + " machines, " + differ + " differ from Kiln, " + flies + " fly in vanilla");
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
