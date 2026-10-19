package w87;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;

/** wp87: the log of {@link W87Agent}. */
public final class W87Hook {
    public static synchronized void alt(Object wither, int head, int id) {
        try {
            String out = System.getenv("W87_OUT");
            if (out == null || id <= 0 || head < 1) {
                return;
            }
            var w = (net.minecraft.world.entity.boss.wither.WitherBoss) wither;
            var target = w.level().getEntity(id);
            String line = w.level().getGameTime() + " " + head + " "
                    + (target == null ? "? 0 0" : net.minecraft.world.entity.EntityType.getKey(target.getType()).getPath() + " " + (long) Math.floor(target.getX()) + " " + (long) Math.floor(target.getZ())) + "\n";
            Files.writeString(Path.of(out), line, StandardOpenOption.CREATE, StandardOpenOption.APPEND);
        } catch (Throwable t) {
            System.err.println("W87Hook: " + t);
        }
    }
}
