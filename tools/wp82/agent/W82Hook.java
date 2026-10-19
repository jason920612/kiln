package w82;

import java.io.FileWriter;
import java.io.PrintWriter;

public final class W82Hook {
    static PrintWriter out;

    static synchronized void log(String s) {
        try {
            if (out == null) {
                out = new PrintWriter(new FileWriter(System.getenv().getOrDefault("W82_OUT", "/tmp/w82.out"), true), true);
            }
            out.println(s);
        } catch (Exception e) {
            e.printStackTrace();
        }
    }

    public static void chunkEmpty(int result, Object pos) {
        log("isChunkEmpty " + pos + " -> " + (result != 0));
    }

    public static void validSpawn(Object result, Object chunk) {
        log("findValidSpawnInChunk " + chunk + " -> " + result);
    }

    public static void tallest(Object result, Object center) {
        log("findTallestBlock around " + center + " -> " + result);
    }
}
