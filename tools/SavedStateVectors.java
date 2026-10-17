// wp77: the `attributes` list and the `Brain` compound a vanilla 26.3 living entity saves right after /summon and after 1, 2, 5, 20, 100 and 300
// ticks (what `/data get entity`, the chunk and level.dat player store write). One case per living entity type with default attributes; the
// mob stands on a stone platform. Built on AttributeVectors.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/SavedStateVectors.java <out.jsonl> [name-filter]

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;

public class SavedStateVectors {
    static final int[] SAMPLES = java.util.stream.IntStream.concat(java.util.stream.IntStream.rangeClosed(0, 40), java.util.stream.IntStream.of(60, 100, 300)).toArray();

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    /** `setup` commands run before the summon, `summon` is the command (with {type}); both optional per variant. */
    record Variant(String name, String summonTail) {}

    static final Variant[] VARIANTS = {new Variant("", ""), new Variant("_baby", " {Age:-24000,IsBaby:1b}")};

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
        }, "SavedStateVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        server.submit(() -> {
            ServerLevel l = server.overworld();
            for (int cx = -1; cx <= 1; cx++)
                for (int cz = -1; cz <= 1; cz++) {
                    l.setChunkForced(cx, cz, true);
                    l.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(3000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            ServerLevel level = server.overworld();
            level.tickRateManager().setFrozen(true);
            var src = server.createCommandSourceStack();
            server.getCommands().performPrefixedCommand(src, "gamerule spawn_mobs false");
            server.getCommands().performPrefixedCommand(src, "difficulty normal");
            server.getCommands().performPrefixedCommand(src, "fill 0 79 0 15 79 15 stone");
            server.getCommands().performPrefixedCommand(src, "fill 0 80 0 15 85 15 air");
            for (var type : BuiltInRegistries.ENTITY_TYPE) {
                String id = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
                if (id.equals("minecraft:player") || id.equals("minecraft:ender_dragon") || id.equals("minecraft:wither")) continue;
                if (!net.minecraft.world.entity.ai.attributes.DefaultAttributes.hasSupplier(type)) continue;
                String shortId = id.substring(10);
                for (Variant v : VARIANTS) {
                    String name = "saved_" + shortId + v.name;
                    if (filter != null && !name.matches(filter) && !name.contains(filter)) continue;
                    try {
                        lines.add(run(level, name, shortId, v.summonTail));
                    } catch (Throwable t) {
                        t.printStackTrace();
                        lines.add("{\"name\":\"" + name + "\",\"error\":\"" + t.toString().replace('"', '\'').replace('\\', '/') + "\"}");
                    }
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("SavedStateVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String saved(ServerLevel level, Entity e) {
        var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
        e.save(out);
        var tag = out.buildResult();
        var attrs = tag.get("attributes");
        var brain = tag.get("Brain");
        // (finalizeSpawn makes a baby by chance: the case says which it got.)
        boolean baby = tag.getIntOr("Age", 0) < 0 || tag.getBooleanOr("IsBaby", false);
        return "{\"baby\":" + baby + ",\"attributes\":" + (attrs == null ? "null" : tagJson(attrs)) + ",\"brain\":" + (brain == null ? "null" : tagJson(brain)) + "}";
    }

    static String run(ServerLevel level, String name, String type, String tail) throws Exception {
        var server = level.getServer();
        int x = 8, y = 80, z = 8;
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "summon minecraft:" + type + " " + x + " " + y + " " + z + tail);
        Entity found = null;
        for (Entity e : level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(x - 3, y - 3, z - 3, x + 3, y + 3, z + 3), e -> true)) found = e;
        if (found == null) return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"loaded\":false}", name, type);
        Field rn = net.minecraft.world.TickRateManager.class.getDeclaredField("runGameElements");
        rn.setAccessible(true);
        StringBuilder samples = new StringBuilder();
        int done = 0;
        for (int s : SAMPLES) {
            if (s > done) {
                rn.set(level.tickRateManager(), true);
                for (; done < s; done++) level.tick(() -> true);
                rn.set(level.tickRateManager(), false);
            }
            if (found.isRemoved()) break;
            samples.append(samples.length() > 0 ? "," : "").append("{\"t\":").append(s).append(",\"saved\":").append(saved(level, found)).append('}');
        }
        found.discard();
        for (Entity e : level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(x - 20, y - 20, z - 20, x + 20, y + 20, z + 20), e -> true)) e.discard();
        return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"loaded\":true,\"samples\":[%s]}", name, type, samples);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + (System.getenv("KILN_HARNESS_PORT") != null ? System.getenv("KILN_HARNESS_PORT") : "25581"),
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:plains\"}",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "spawn-monsters=false",
                "spawn-animals=false",
                "generate-structures=false",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
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

    static String tagJson(net.minecraft.nbt.Tag t) {
        if (t instanceof net.minecraft.nbt.CompoundTag c) {
            StringBuilder sb = new StringBuilder("{\"c\":{");
            boolean first = true;
            for (String k : c.keySet()) {
                if (!first) sb.append(',');
                first = false;
                sb.append('"').append(k).append("\":").append(tagJson(c.get(k)));
            }
            return sb.append("}}").toString();
        }
        if (t instanceof net.minecraft.nbt.ListTag l) {
            StringBuilder sb = new StringBuilder("{\"l\":[");
            for (int i = 0; i < l.size(); i++) {
                if (i > 0) sb.append(',');
                sb.append(tagJson(l.get(i)));
            }
            return sb.append("]}").toString();
        }
        if (t instanceof net.minecraft.nbt.ByteTag b) return "{\"b\":" + b.byteValue() + "}";
        if (t instanceof net.minecraft.nbt.ShortTag s) return "{\"s\":" + s.shortValue() + "}";
        if (t instanceof net.minecraft.nbt.IntTag i) return "{\"i\":" + i.intValue() + "}";
        if (t instanceof net.minecraft.nbt.LongTag g) return "{\"L\":\"" + g.longValue() + "\"}";
        if (t instanceof net.minecraft.nbt.FloatTag f) return "{\"f\":" + Float.toString(f.floatValue()) + "}";
        if (t instanceof net.minecraft.nbt.DoubleTag dd) return "{\"d\":" + Double.toString(dd.doubleValue()) + "}";
        if (t instanceof net.minecraft.nbt.StringTag st) return "{\"str\":\"" + st.value().replace("\\", "\\\\").replace("\"", "\\\"") + "\"}";
        if (t instanceof net.minecraft.nbt.IntArrayTag ia) return "{\"ia\":" + java.util.Arrays.toString(ia.getAsIntArray()) + "}";
        if (t instanceof net.minecraft.nbt.ByteArrayTag ba) return "{\"ba\":" + java.util.Arrays.toString(ba.getAsByteArray()) + "}";
        if (t instanceof net.minecraft.nbt.LongArrayTag la) {
            StringBuilder sb = new StringBuilder("{\"la\":[");
            long[] v = la.getAsLongArray();
            for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append('"').append(v[i]).append('"');
            return sb.append("]}").toString();
        }
        throw new IllegalArgumentException("tag " + t);
    }
}
