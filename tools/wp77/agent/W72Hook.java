package w72;

import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.BlockPos;

/** wp72: the counters of {@link W72Agent}. */
public final class W72Hook {
    static final Map<String, long[]> C = new TreeMap<>();
    static boolean hooked;

    static synchronized void add(String k) {
        C.computeIfAbsent(k, x -> new long[1])[0]++;
        if (!hooked) {
            hooked = true;
            Runtime.getRuntime().addShutdownHook(new Thread(W72Hook::dump));
        }
    }

    public static void slimeRet(int result, Object pos, Object level) {
        BlockPos p = (BlockPos) pos;
        boolean slimeChunk = false;
        try {
            long seed = ((net.minecraft.world.level.WorldGenLevel) level).getSeed();
            slimeChunk = net.minecraft.world.level.levelgen.WorldgenRandom.seedSlimeChunk(p.getX() >> 4, p.getZ() >> 4, seed, 987234911L).nextInt(10) == 0;
        } catch (Throwable t) {
            add("error " + t);
        }
        String where = p.getY() < 40 ? "y<40" : p.getY() < 70 && p.getY() > 50 ? "y51-69" : "other-y";
        add("slime predicate " + where + " slimeChunk=" + slimeChunk + " -> " + (result != 0));
    }

    static String typeName(Object spawnerData) {
        try {
            return net.minecraft.world.entity.EntityType.getKey(((net.minecraft.world.level.biome.MobSpawnSettings.SpawnerData) spawnerData).type()).getPath();
        } catch (Throwable t) {
            return "?";
        }
    }

    public static void validPos(int result, Object data, Object pos) {
        add("validPosition " + typeName(data) + " -> " + (result != 0));
        String bx = System.getProperty("w72.box");
        if (bx != null && pos instanceof BlockPos p) {
            String[] v = bx.split(",");
            int[] n = new int[6];
            for (int i = 0; i < 6; i++) n[i] = Integer.parseInt(v[i].trim());
            if (p.getX() >= n[0] && p.getX() <= n[3] && p.getY() >= n[1] && p.getY() <= n[4] && p.getZ() >= n[2] && p.getZ() <= n[5]) {
                add("validPosition IN-BOX " + typeName(data) + " -> " + (result != 0));
                add("validPosition IN-BOX Y=" + p.getY() + " " + typeName(data) + " -> " + (result != 0));
            }
        }
    }

    /** NaturalSpawner.spawnCategoryForChunk(category, level, chunk, ...): one try of a chunk (counted for the chunks the box touches). */
    public static void chunkAttempt(Object category, Object chunk) {
        try {
            var pos = ((net.minecraft.world.level.chunk.LevelChunk) chunk).getPos();
            add("chunkAttempt " + category + " all");
            String bx = System.getProperty("w72.box");
            if (bx == null) return;
            String[] v = bx.split(",");
            int[] n = new int[6];
            for (int i = 0; i < 6; i++) n[i] = Integer.parseInt(v[i].trim());
            if (pos.x() >= (n[0] >> 4) && pos.x() <= (n[3] >> 4) && pos.z() >= (n[2] >> 4) && pos.z() <= (n[5] >> 4)) {
                add("chunkAttempt " + category + " " + pos.x() + "," + pos.z());
            }
        } catch (Throwable t) {
            add("chunkAttempt error " + t);
        }
    }

    public static void validMob(int result, Object mob) {
        String n;
        try {
            n = net.minecraft.world.entity.EntityType.getKey(((net.minecraft.world.entity.Mob) mob).getType()).getPath();
        } catch (Throwable t) {
            n = "?";
        }
        add("validMob " + n + " -> " + (result != 0));
    }

    public static void globalCap(int result, Object state, Object category) {
        try {
            var f = state.getClass().getDeclaredField("spawnableChunkCount");
            f.setAccessible(true);
            var g = state.getClass().getDeclaredField("mobCategoryCounts");
            g.setAccessible(true);
            var counts = (it.unimi.dsi.fastutil.objects.Object2IntOpenHashMap<?>) g.get(state);
            int n = counts.getInt(category);
            if (String.valueOf(category).equals("CREATURE") || String.valueOf(category).equals("MONSTER")) {
                add("global " + category + " chunks=" + f.getInt(state) + " count=" + n + " -> " + (result != 0));
            }
        } catch (Throwable t) {
            add("globalCap error " + t);
        }
    }

    public static void cats(Object list) {
        add("categories " + String.valueOf(list));
    }

    public static void pick(Object optional) {
        try {
            var o = (java.util.Optional<?>) optional;
            add("pick " + (o.isPresent() ? typeName(o.get()) : "(none)"));
        } catch (Throwable t) {
            add("pick error");
        }
    }

    public static synchronized void dump() {
        String out = System.getProperty("w72.out", System.getenv("W72_OUT"));
        if (out == null) return;
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(out)))) {
            for (var e : C.entrySet()) w.println(e.getKey() + "\t" + e.getValue()[0]);
        } catch (Exception ex) {
            ex.printStackTrace();
        }
    }
}
