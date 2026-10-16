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
