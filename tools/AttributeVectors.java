// wp75: the client-syncable attributes a vanilla 26.3 mob has right after /summon and five ticks later (AttributeMap.getSyncableAttributes: only the
// attributes something asked for an instance of): name, base value, modifiers. For every living entity type with default attributes. Built on EntityNbtVectors.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/AttributeVectors.java <out.jsonl> [name-filter]


import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;

public class AttributeVectors {
    record Case(String name, String kind, String type, String nbt) {}

    /** Every living entity type that has default attributes (one case each). */
    static List<Case> cases() {
        List<Case> c = new ArrayList<>();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            String id = BuiltInRegistries.ENTITY_TYPE.getKey(type).toString();
            if (id.equals("minecraft:player") || id.equals("minecraft:ender_dragon") || id.equals("minecraft:wither")) continue;
            if (!net.minecraft.world.entity.ai.attributes.DefaultAttributes.hasSupplier(type)) continue;
            c.add(new Case("attrs_" + id.substring(10), "entity", id.substring(10), "{}"));
        }
        return c;
    }

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
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "AttributeVectors main");
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
            // A scoreboard for the text displays that read one.
            var src = server.createCommandSourceStack();
            server.getCommands().performPrefixedCommand(src, "scoreboard objectives add o dummy");
            for (Case c : cases()) {
                if (filter != null && !c.name.matches(filter) && !c.name.contains(filter)) continue;
                try {
                    lines.add(run(level, c));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + c.name + "\",\"error\":\"" + t.toString().replace('"', '\'').replace('\\', '/') + "\"}");
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("AttributeVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String attrs(Entity e) {
        StringBuilder sb = new StringBuilder("[");
        boolean first = true;
        if (e instanceof net.minecraft.world.entity.LivingEntity l) {
            for (var inst : l.getAttributes().getSyncableAttributes()) {
                if (!first) sb.append(',');
                first = false;
                sb.append(String.format(Locale.ROOT, "[\"%s\",%s,[", inst.getAttribute().unwrapKey().get().identifier(), Double.toString(inst.getBaseValue())));
                boolean f2 = true;
                for (var m : inst.getModifiers()) {
                    if (!f2) sb.append(',');
                    f2 = false;
                    sb.append(String.format(Locale.ROOT, "[\"%s\",%s,\"%s\"]", m.id(), Double.toString(m.amount()), m.operation().getSerializedName()));
                }
                sb.append("]]");
            }
        }
        return sb.append(']').toString();
    }

    static String run(ServerLevel level, Case c) throws Exception {
        var server = level.getServer();
        // The summon command: the same path Kiln's `/summon` follows.
        int x = 8, y = 80, z = 8;
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "summon minecraft:" + c.type + " " + x + " " + y + " " + z);
        Entity found = null;
        for (Entity e : level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(x - 3, y - 3, z - 3, x + 3, y + 3, z + 3), e -> true)) found = e;
        if (found == null) return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"loaded\":false}", c.name, c.type);
        String fresh = attrs(found);
        // A few ticks later (the mob's own code may touch more attributes).
        Field rn = net.minecraft.world.TickRateManager.class.getDeclaredField("runGameElements");
        rn.setAccessible(true);
        rn.set(level.tickRateManager(), true);
        StringBuilder per = new StringBuilder();
        for (int i = 0; i < 5; i++) {
            level.tick(() -> true);
            per.append(i > 0 ? "," : "").append(attrs(found));
        }
        rn.set(level.tickRateManager(), false);
        String later = attrs(found);
        found.discard();
        for (Entity e : level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(x - 6, y - 6, z - 6, x + 6, y + 6, z + 6), e -> true)) e.discard();
        return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"loaded\":true,\"fresh\":%s,\"ticks\":[%s],\"later\":%s}", c.name, c.type, fresh, per, later);
    }

    static String d(double v) {
        return Double.toString(v);
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
