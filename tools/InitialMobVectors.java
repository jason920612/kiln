// Differential test vectors for the mobs that chunk generation makes (wp49): in a real vanilla 26.3 dedicated server with a
// generated world, `NaturalSpawner.spawnMobsForChunkGeneration` (the SPAWN status of `ChunkGenerator.spawnOriginalMobs`)
// puts the animals of a biome into every new chunk. The server's level is frozen and a window of chunks around the
// origin is generated; one line per chunk that got mobs: its mobs (type, position, yaw, baby flag).
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/InitialMobVectors.java <out.jsonl> [seed] [radius in chunks]

import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.chunk.status.ChunkStatus;

public class InitialMobVectors {
    static MinecraftServer server;

    public static void main(String[] args) throws Exception {
        Path out = Path.of(args[0]).toAbsolutePath();
        String seed = args.length > 1 ? args[1] : "12345";
        int radius = args.length > 2 ? Integer.parseInt(args[2]) : 20;
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n", "server-port=25583", "online-mode=false", "level-name=world", "level-seed=" + seed,
                "level-type=minecraft\\:normal", "generate-structures=true", "max-tick-time=-1", "view-distance=3", "simulation-distance=3", "sync-chunk-writes=false",
                "enable-rcon=false", "enable-query=false", "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "InitialMobVectors main");
        main.start();
        server = awaitServer();
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                ServerLevel level = server.overworld();
                level.tickRateManager().setFrozen(true);
                for (int cx = -radius; cx <= radius; cx++) {
                    for (int cz = -radius; cz <= radius; cz++) {
                        level.setChunkForced(cx, cz, true);
                        level.getChunk(cx, cz, ChunkStatus.FULL, true);
                    }
                }
                // Debug: INITIAL_MOB_DUMP="x,y,z;x,y,z" prints the blocks at those places.
                String dump = System.getenv("INITIAL_MOB_DUMP");
                if (dump != null) {
                    for (String p : dump.split(";")) {
                        String[] c = p.split(",");
                        var bp = new net.minecraft.core.BlockPos(Integer.parseInt(c[0]), Integer.parseInt(c[1]), Integer.parseInt(c[2]));
                        System.out.println("BLOCK " + p + " " + level.getBlockState(bp));
                    }
                }
                // Debug: INITIAL_MOB_COUNT="x0,z0,x1,z1" counts the blocks of that box by type.
                String count = System.getenv("INITIAL_MOB_COUNT");
                if (count != null) {
                    String[] c = count.split(",");
                    Map<String, Integer> counts = new TreeMap<>();
                    for (int x = Integer.parseInt(c[0]); x <= Integer.parseInt(c[2]); x++) {
                        for (int z = Integer.parseInt(c[1]); z <= Integer.parseInt(c[3]); z++) {
                            for (int y = level.getMinY(); y < level.getMaxY(); y++) {
                                String n = BuiltInRegistries.BLOCK.getKey(level.getBlockState(new net.minecraft.core.BlockPos(x, y, z)).getBlock()).toString();
                                counts.merge(n, 1, Integer::sum);
                            }
                        }
                    }
                    for (var en : counts.entrySet()) System.out.println("COUNT " + en.getKey() + " " + en.getValue());
                }
                Map<Long, List<Entity>> byChunk = new TreeMap<>();
                for (Entity e : level.getAllEntities()) {
                    if (!(e instanceof Mob)) continue;
                    byChunk.computeIfAbsent(ChunkPos.pack(e.blockPosition().getX() >> 4, e.blockPosition().getZ() >> 4), k -> new ArrayList<>()).add(e);
                }
                for (var en : byChunk.entrySet()) {
                    int cx = ChunkPos.getX(en.getKey()), cz = ChunkPos.getZ(en.getKey());
                    if (Math.abs(cx) > radius - 2 || Math.abs(cz) > radius - 2) continue;
                    List<Entity> mobs = en.getValue();
                    mobs.sort(Comparator.comparing((Entity e) -> BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()).toString()).thenComparingDouble(Entity::getX).thenComparingDouble(Entity::getZ));
                    StringBuilder b = new StringBuilder();
                    for (Entity e : mobs) {
                        if (b.length() > 0) b.append(',');
                        b.append(String.format(Locale.ROOT, "[\"%s\",%s,%s,%s,%s,%b,\"%s\"]", BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()), Double.toString(e.getX()), Double.toString(e.getY()),
                                Double.toString(e.getZ()), Float.toString(e.getYRot()), e instanceof net.minecraft.world.entity.AgeableMob a && a.isBaby(), e.getType().getCategory().getName()));
                    }
                    lines.add("{\"chunk\":[" + cx + "," + cz + "],\"mobs\":[" + b + "]}");
                }
                lines.add(0, "{\"seed\":" + seed + ",\"radius\":" + radius + ",\"checked\":" + (radius - 2) + "}");
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("InitialMobVectors: wrote " + lines.size() + " lines to " + out);
        server.halt(false);
        System.exit(0);
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 6000; i++) {
            for (Thread t : Thread.getAllStackTraces().keySet()) {
                if (!t.getName().equals("Server thread")) continue;
                java.lang.reflect.Field holderField = Thread.class.getDeclaredField("holder");
                holderField.setAccessible(true);
                Object holder = holderField.get(t);
                java.lang.reflect.Field taskField = holder.getClass().getDeclaredField("task");
                taskField.setAccessible(true);
                Object task = taskField.get(holder);
                for (java.lang.reflect.Field f : task.getClass().getDeclaredFields()) {
                    f.setAccessible(true);
                    if (f.get(task) instanceof java.util.concurrent.atomic.AtomicReference<?> ref && ref.get() instanceof MinecraftServer s) {
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
