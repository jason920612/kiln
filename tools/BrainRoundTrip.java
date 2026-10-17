// wp77: what a vanilla 26.3 living entity saves as `Brain` after it was loaded from `Brain:{memories:{...}}`: for every living entity type all the
// serializable memory types at once (a valid value each, half of them with a `ttl`), and for the brain-driven types each memory alone, valid
// and invalid (a string where the codec wants something else). Samples right after /summon and 1 and 3 ticks later. Built on SavedStateVectors.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/BrainRoundTrip.java <out.jsonl> [name-filter]

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

public class BrainRoundTrip {
    static final int[] SAMPLES = {0, 1, 3};

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    /** The serializable memory types and a valid value for each (SNBT). */
    static final String[][] MEMORIES = {
        {"home", "{dimension:\"minecraft:overworld\",pos:[I;1,2,3]}"},
        {"job_site", "{dimension:\"minecraft:overworld\",pos:[I;4,5,6]}"},
        {"potential_job_site", "{dimension:\"minecraft:the_nether\",pos:[I;7,8,9]}"},
        {"meeting_point", "{dimension:\"minecraft:overworld\",pos:[I;10,11,12]}"},
        {"golem_detected_recently", "1b"},
        {"danger_detected_recently", "1b"},
        {"last_slept", "12345L"},
        {"last_woken", "12346L"},
        {"last_worked_at_poi", "12347L"},
        {"play_dead_ticks", "77"},
        {"temptation_cooldown_ticks", "78"},
        {"gaze_cooldown_ticks", "79"},
        {"long_jump_cooling_down", "80"},
        {"has_hunting_cooldown", "1b"},
        {"ram_cooldown_ticks", "81"},
        {"is_in_water", "{}"},
        {"is_pregnant", "{}"},
        {"is_panicking", "1b"},
        {"visited_block_positions", "[{dimension:\"minecraft:overworld\",pos:[I;1,2,3]},{dimension:\"minecraft:overworld\",pos:[I;4,5,6]}]"},
        {"unreachable_transport_block_positions", "[{dimension:\"minecraft:overworld\",pos:[I;1,2,3]}]"},
        {"charge_cooldown_ticks", "82"},
        {"attack_target_cooldown", "83"},
        {"angry_at", "[I;1,2,3,4]"},
        {"universal_anger", "1b"},
        {"admiring_item", "1b"},
        {"admiring_disabled", "1b"},
        {"hunted_recently", "1b"},
        {"recent_projectile", "{}"},
        {"is_sniffing", "{}"},
        {"is_emerging", "{}"},
        {"roar_sound_delay", "{}"},
        {"dig_cooldown", "{}"},
        {"roar_sound_cooldown", "{}"},
        {"sniff_cooldown", "{}"},
        {"touch_cooldown", "{}"},
        {"vibration_cooldown", "{}"},
        {"sonic_boom_cooldown", "{}"},
        {"sonic_boom_sound_cooldown", "{}"},
        {"sonic_boom_sound_delay", "{}"},
        {"liked_player", "[I;5,6,7,8]"},
        {"liked_noteblock", "{dimension:\"minecraft:overworld\",pos:[I;1,2,3]}"},
        {"liked_noteblock_cooldown_ticks", "84"},
        {"item_pickup_cooldown_ticks", "85"},
        {"sniffer_explored_positions", "[{dimension:\"minecraft:overworld\",pos:[I;1,2,3]}]"},
        {"breeze_jump_cooldown", "{}"},
        {"breeze_shoot", "{}"},
        {"breeze_shoot_charging", "{}"},
        {"breeze_shoot_recover", "{}"},
        {"breeze_shoot_cooldown", "{}"},
        {"breeze_jump_inhaling", "{}"},
        {"breeze_jump_target", "[I;1,2,3]"},
        {"breeze_leaving_water", "{}"},
    };

    static final String[] BRAIN_TYPES = {"piglin", "piglin_brute", "hoglin", "zoglin", "villager", "axolotl", "goat", "frog", "allay", "warden", "sniffer", "camel", "camel_husk", "armadillo", "breeze", "creaking", "copper_golem", "happy_ghast", "ghastling", "tadpole", "nautilus", "zombie_nautilus"};

    record Case(String name, String type, String nbt) {}

    static String entry(String[] m, int i) {
        return "\"minecraft:" + m[0] + "\":{value:" + m[1] + (i % 2 == 1 ? ",ttl:" + (100 + i) + "L" : "") + "}";
    }

    static java.util.List<Case> cases(String type) {
        java.util.List<Case> out = new java.util.ArrayList<>();
        StringBuilder all = new StringBuilder();
        for (int i = 0; i < MEMORIES.length; i++) all.append(i > 0 ? "," : "").append(entry(MEMORIES[i], i));
        out.add(new Case("brain_all_" + type, type, "{Brain:{memories:{" + all + "}}}"));
        if (java.util.Arrays.asList(BRAIN_TYPES).contains(type)) {
            for (int i = 0; i < MEMORIES.length; i++) {
                out.add(new Case("brain_one_" + type + "_" + MEMORIES[i][0], type, "{Brain:{memories:{" + entry(MEMORIES[i], 0) + "}}}"));
                out.add(new Case("brain_ttl_" + type + "_" + MEMORIES[i][0], type, "{Brain:{memories:{" + entry(MEMORIES[i], 1) + "}}}"));
            }
            // An invalid value: a string where the codec wants something else, next to a valid one.
            for (int i = 0; i < MEMORIES.length && (type.equals("piglin") || type.equals("villager") || type.equals("warden")); i++) {
                String good = entry(MEMORIES[(i + 1) % MEMORIES.length], 0);
                out.add(new Case("brain_bad_" + type + "_" + MEMORIES[i][0], type, "{Brain:{memories:{\"minecraft:" + MEMORIES[i][0] + "\":{value:\"x\"}," + good + "}}}"));
            }
            out.add(new Case("brain_unknown_" + type, type, "{Brain:{memories:{\"minecraft:nonsense\":{value:1b},\"minecraft:home\":{value:{dimension:\"minecraft:overworld\",pos:[I;1,2,3]}}}}}"));
            out.add(new Case("brain_novalue_" + type, type, "{Brain:{memories:{\"minecraft:home\":{ttl:5L},\"minecraft:last_slept\":{value:5L,ttl:\"x\"}}}}"));
            out.add(new Case("brain_notcompound_" + type, type, "{Brain:{memories:5}}"));
            out.add(new Case("brain_ttl_edge_" + type, type, "{Brain:{memories:{\"minecraft:last_slept\":{value:5L,ttl:0L},\"minecraft:last_woken\":{value:5L,ttl:-5L},\"minecraft:play_dead_ticks\":{value:3,ttl:1L},\"minecraft:ram_cooldown_ticks\":{value:3,ttl:2L}}}}"));
        }
        return out;
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
        }, "BrainRoundTrip main");
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
                for (Case c : cases(shortId)) {
                    if (filter != null && !c.name.matches(filter) && !c.name.contains(filter)) continue;
                    try {
                        lines.add(run(level, c));
                    } catch (Throwable t) {
                        t.printStackTrace();
                        lines.add("{\"name\":\"" + c.name + "\",\"error\":\"" + t.toString().replace('"', '\'').replace('\\', '/') + "\"}");
                    }
                }
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("BrainRoundTrip: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static String saved(ServerLevel level, Entity e) {
        var out = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
        e.save(out);
        var tag = out.buildResult();
        var brain = tag.get("Brain");
        return "{\"brain\":" + (brain == null ? "null" : tagJson(brain)) + "}";
    }

    static String run(ServerLevel level, Case c) throws Exception {
        String name = c.name, type = c.type;
        var server = level.getServer();
        int x = 8, y = 80, z = 8;
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "summon minecraft:" + type + " " + x + " " + y + " " + z + " " + c.nbt);
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
        return String.format(Locale.ROOT, "{\"name\":\"%s\",\"type\":\"minecraft:%s\",\"nbt\":\"%s\",\"loaded\":true,\"samples\":[%s]}", name, type, c.nbt.replace("\\", "\\\\").replace("\"", "\\\""), samples);
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
