// Differential test vectors for natural spawning's structure overrides: runs a vanilla 26.3 dedicated
// server in-process (a generated world with structures; port from $KILN_HARNESS_PORT or the first
// free of 25581-25583), finds structures of the kinds that matter (fortresses and bastions in the nether;
// swamp huts, ocean monuments, pillager outposts, trial chambers, ancient cities, ruins, mineshafts,
// strongholds, villages, ... in the overworld), generates the chunks around them and records, for
// sample positions in and around every structure start, what `NaturalSpawner.mobsAt` returns per
// mob category (entity type, weight, count range), together with the `structures` data (references and
// the pieces' boxes) of the chunks involved, as vanilla saves it.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/SpawnVectors.java <out.jsonl> [seed]
// (tools/spawn_vectors.py sets this up)
//
// Output (JSON lines): first a line {"lists": {id: [[type, weight, min, max], ...]}, "categories": [...]},
// then {"chunk": [dim, x, z], "structures": <typed NBT>} lines, then {"sample": [dim, x, y, z, biome, below_nether_bricks, [list id per category]]}.

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderSet;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.MobCategory;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.NaturalSpawner;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructureStart;

public class SpawnVectors {
    static final List<MobCategory> CATEGORIES = List.of(MobCategory.values());
    static final Map<String, Integer> listIds = new LinkedHashMap<>();
    static final List<String> listJson = new ArrayList<>();

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static void run(String[] args) throws Exception {
        Path out = Path.of(args[0]).toAbsolutePath();
        String seed = args.length > 1 ? args[1] : "1234";
        writeServerFiles(seed);
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "SpawnVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                collect(server, lines);
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out))) {
            StringBuilder head = new StringBuilder("{\"lists\":{");
            for (int i = 0; i < listJson.size(); i++) head.append(i > 0 ? "," : "").append('"').append(i).append("\":").append(listJson.get(i));
            head.append("},\"categories\":[");
            for (int i = 0; i < CATEGORIES.size(); i++) head.append(i > 0 ? "," : "").append('"').append(CATEGORIES.get(i).getName()).append('"');
            head.append("]}");
            w.println(head);
            for (String l : lines) w.println(l);
        }
        System.out.println("SpawnVectors: wrote " + lines.size() + " lines to " + out);
        server.halt(false);
        System.exit(0);
    }

    static final String[][] SEARCHES = {
        // dimension, structure, how many to look for
        {"minecraft:overworld", "minecraft:swamp_hut", "3"},
        {"minecraft:overworld", "minecraft:monument", "3"},
        {"minecraft:overworld", "minecraft:pillager_outpost", "3"},
        {"minecraft:overworld", "minecraft:trial_chambers", "3"},
        {"minecraft:overworld", "minecraft:ancient_city", "2"},
        {"minecraft:overworld", "minecraft:ocean_ruin_cold", "1"},
        {"minecraft:overworld", "minecraft:ocean_ruin_warm", "1"},
        {"minecraft:overworld", "minecraft:mineshaft", "2"},
        {"minecraft:overworld", "minecraft:stronghold", "1"},
        {"minecraft:overworld", "minecraft:village_plains", "1"},
        {"minecraft:overworld", "minecraft:desert_pyramid", "1"},
        {"minecraft:overworld", "minecraft:jungle_pyramid", "1"},
        {"minecraft:overworld", "minecraft:igloo", "1"},
        {"minecraft:overworld", "minecraft:shipwreck", "1"},
        {"minecraft:overworld", "minecraft:mansion", "1"},
        {"minecraft:the_nether", "minecraft:fortress", "4"},
        {"minecraft:the_nether", "minecraft:bastion_remnant", "3"},
        {"minecraft:the_nether", "minecraft:nether_fossil", "2"},
        {"minecraft:the_nether", "minecraft:ruined_portal_nether", "1"},
        {"minecraft:the_end", "minecraft:end_city", "1"},
    };

    static void collect(MinecraftServer server, List<String> lines) throws Exception {
        Method mobsAt = NaturalSpawner.class.getDeclaredMethod("mobsAt", ServerLevel.class, StructureManager.class, ChunkGenerator.class, MobCategory.class, BlockPos.class);
        mobsAt.setAccessible(true);
        Random rnd = new Random(77);
        Set<String> seen = new HashSet<>();
        Map<String, Set<Long>> dumped = new HashMap<>();
        int samples = 0;
        for (String[] search : SEARCHES) {
            ServerLevel level = server.getLevel(ResourceKey.create(Registries.DIMENSION, Identifier.parse(search[0])));
            if (level == null) continue;
            var registry = level.registryAccess().lookupOrThrow(Registries.STRUCTURE);
            var holder = registry.get(ResourceKey.create(Registries.STRUCTURE, Identifier.parse(search[1])));
            if (holder.isEmpty()) {
                System.out.println("SpawnVectors: no structure " + search[1]);
                continue;
            }
            int want = Integer.parseInt(search[2]);
            List<BlockPos> found = new ArrayList<>();
            int[][] origins = {{0, 0}, {5000, 5000}, {-6000, 4000}, {3000, -7000}, {-9000, -9000}, {12000, 1000}};
            ChunkGenerator generator = level.getChunkSource().getGenerator();
            for (int[] o : origins) {
                if (found.size() >= want) break;
                var r = generator.findNearestMapStructure(level, HolderSet.direct(holder.get()), new BlockPos(o[0], 64, o[1]), 100, false);
                if (r == null) continue;
                BlockPos p = r.getFirst();
                boolean dup = false;
                for (BlockPos q : found) if (Math.abs(q.getX() - p.getX()) < 300 && Math.abs(q.getZ() - p.getZ()) < 300) dup = true;
                if (!dup) found.add(p);
            }
            System.out.println("SpawnVectors: " + search[1] + " in " + search[0] + ": " + found);
            for (BlockPos at : found) {
                ChunkPos c0 = ChunkPos.containing(at);
                int r = search[1].equals("minecraft:ancient_city") || search[1].equals("minecraft:mansion") ? 8 : 6;
                List<ChunkAccess> area = new ArrayList<>();
                for (int dx = -r; dx <= r; dx++)
                    for (int dz = -r; dz <= r; dz++) area.add(level.getChunk(c0.x() + dx, c0.z() + dz));
                // the starts in the area, and samples in and around them
                List<StructureStart> starts = new ArrayList<>();
                for (ChunkAccess ca : area) for (var st : ca.getAllStarts().values()) if (st.isValid() && !starts.contains(st)) starts.add(st);
                List<int[]> positions = new ArrayList<>();
                for (StructureStart st : starts) {
                    BoundingBox bb = st.getBoundingBox();
                    var pieces = st.getPieces();
                    for (int pi = 0; pi < pieces.size(); pi += Math.max(1, pieces.size() / 50)) {
                        BoundingBox pb = pieces.get(pi).getBoundingBox();
                        positions.add(new int[] {pb.minX(), pb.minY(), pb.minZ()});
                        positions.add(new int[] {pb.maxX(), pb.maxY(), pb.maxZ()});
                        positions.add(new int[] {pb.minX() - 1, pb.minY(), pb.minZ() - 1});
                        positions.add(new int[] {pb.maxX() + 1, pb.maxY(), pb.maxZ() + 1});
                        positions.add(new int[] {(pb.minX() + pb.maxX()) / 2, (pb.minY() + pb.maxY()) / 2, (pb.minZ() + pb.maxZ()) / 2});
                        positions.add(new int[] {(pb.minX() + pb.maxX()) / 2, pb.minY() - 1, (pb.minZ() + pb.maxZ()) / 2});
                    }
                    positions.add(new int[] {bb.minX(), bb.minY(), bb.minZ()});
                    positions.add(new int[] {bb.maxX(), bb.maxY(), bb.maxZ()});
                    positions.add(new int[] {bb.minX() - 1, bb.minY(), bb.minZ()});
                    positions.add(new int[] {bb.maxX(), bb.maxY() + 1, bb.maxZ()});
                    positions.add(new int[] {bb.getCenter().getX(), bb.getCenter().getY(), bb.getCenter().getZ()});
                    for (int i = 0; i < 70; i++) {
                        positions.add(new int[] {bb.minX() + rnd.nextInt(bb.getXSpan()), bb.minY() + rnd.nextInt(bb.getYSpan()), bb.minZ() + rnd.nextInt(bb.getZSpan())});
                    }
                    for (int i = 0; i < 12; i++) {
                        positions.add(new int[] {bb.minX() - 6 + rnd.nextInt(bb.getXSpan() + 12), bb.minY() - 6 + rnd.nextInt(bb.getYSpan() + 12), bb.minZ() - 6 + rnd.nextInt(bb.getZSpan() + 12)});
                    }
                }
                // a few places far from anything
                for (int i = 0; i < 10; i++) positions.add(new int[] {at.getX() + rnd.nextInt(400) - 200, level.getMinY() + 5 + rnd.nextInt(level.getHeight() - 10), at.getZ() + rnd.nextInt(400) - 200});
                Set<Long> chunksNeeded = new HashSet<>();
                for (int[] p : positions) {
                    String key = search[0] + " " + p[0] + " " + p[1] + " " + p[2];
                    if (!seen.add(key)) continue;
                    BlockPos pos = new BlockPos(p[0], p[1], p[2]);
                    ChunkPos cp = ChunkPos.containing(pos);
                    ChunkAccess here = level.getChunk(cp.x(), cp.z());
                    if (here == null) continue;
                    StringBuilder ids = new StringBuilder();
                    for (MobCategory cat : CATEGORIES) {
                        Object list = mobsAt.invoke(null, level, level.structureManager(), generator, cat, pos);
                        ids.append(ids.length() > 0 ? "," : "").append(listId(list));
                    }
                    boolean nb = level.getBlockState(pos.below()).is(Blocks.NETHER_BRICKS);
                    String biome = level.getBiome(pos).unwrapKey().map(k -> k.identifier().toString()).orElse("?");
                    lines.add(String.format(Locale.ROOT, "{\"sample\":[\"%s\",%d,%d,%d,\"%s\",%b,[%s]]}", search[0], p[0], p[1], p[2], biome, nb, ids));
                    samples++;
                    chunksNeeded.add(cp.pack());
                    for (var refs : here.getAllReferences().values()) for (long l : refs) chunksNeeded.add(l);
                }
                Set<Long> done = dumped.computeIfAbsent(search[0], k -> new HashSet<>());
                for (long l : chunksNeeded) {
                    if (!done.add(l)) continue;
                    ChunkPos cp = ChunkPos.unpack(l);
                    ChunkAccess ca = level.getChunk(cp.x(), cp.z(), ChunkStatus.STRUCTURE_STARTS);
                    // (a full chunk comes wrapped; one that is not full has no references saved: the starts are enough)
                    if (ca instanceof net.minecraft.world.level.chunk.ImposterProtoChunk ip) ca = ip.getWrapped();
                    var tag = net.minecraft.world.level.chunk.storage.SerializableChunkData.copyOf(level, ca);
                    net.minecraft.nbt.CompoundTag structures = tag.write().getCompoundOrEmpty("structures");
                    lines.add(String.format(Locale.ROOT, "{\"chunk\":[\"%s\",%d,%d],\"structures\":%s}", search[0], cp.x(), cp.z(), stripped(structures)));
                }
            }
        }
        System.out.println("SpawnVectors: " + samples + " samples, " + listJson.size() + " distinct lists");
    }

    /// The `structures` tag with each start reduced to its pieces' boxes.
    static String stripped(net.minecraft.nbt.CompoundTag structures) {
        net.minecraft.nbt.CompoundTag out = new net.minecraft.nbt.CompoundTag();
        out.put("References", structures.getCompoundOrEmpty("References"));
        net.minecraft.nbt.CompoundTag starts = new net.minecraft.nbt.CompoundTag();
        net.minecraft.nbt.CompoundTag all = structures.getCompoundOrEmpty("starts");
        for (String key : all.keySet()) {
            net.minecraft.nbt.CompoundTag st = all.getCompoundOrEmpty(key);
            net.minecraft.nbt.CompoundTag keep = new net.minecraft.nbt.CompoundTag();
            keep.putString("id", st.getStringOr("id", ""));
            net.minecraft.nbt.ListTag children = new net.minecraft.nbt.ListTag();
            for (var ch : st.getListOrEmpty("Children")) {
                net.minecraft.nbt.CompoundTag c = new net.minecraft.nbt.CompoundTag();
                if (ch instanceof net.minecraft.nbt.CompoundTag cc) c.put("BB", cc.get("BB"));
                children.add(c);
            }
            keep.put("Children", children);
            starts.put(key, keep);
        }
        out.put("starts", starts);
        return tagJson(out);
    }

    /// The id of a list of `[type, weight, min, max]` (interned).
    static int listId(Object weightedList) throws Exception {
        @SuppressWarnings("unchecked")
        var items = ((net.minecraft.util.random.WeightedList<net.minecraft.world.level.biome.MobSpawnSettings.SpawnerData>) weightedList).unwrap();
        StringBuilder sb = new StringBuilder("[");
        for (int i = 0; i < items.size(); i++) {
            var w = items.get(i);
            var d = w.value();
            if (i > 0) sb.append(',');
            sb.append(String.format(Locale.ROOT, "[\"%s\",%d,%d,%d]", net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getKey(d.type()), w.weight(), d.count().minInclusive(), d.count().maxInclusive()));
        }
        String json = sb.append(']').toString();
        Integer id = listIds.get(json);
        if (id == null) {
            id = listJson.size();
            listIds.put(json, id);
            listJson.add(json);
        }
        return id;
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
        if (t instanceof net.minecraft.nbt.IntArrayTag ia) return "{\"ia\":" + java.util.Arrays.toString(ia.getAsIntArray()) + "}";
        if (t instanceof net.minecraft.nbt.StringTag st) return "{\"str\":\"" + st.value().replace("\\", "\\\\").replace("\"", "\\\"") + "\"}";
        if (t instanceof net.minecraft.nbt.IntTag i) return "{\"i\":" + i.intValue() + "}";
        if (t instanceof net.minecraft.nbt.LongArrayTag la) {
            StringBuilder sb = new StringBuilder("{\"la\":[");
            long[] v = la.getAsLongArray();
            for (int i = 0; i < v.length; i++) sb.append(i > 0 ? "," : "").append('"').append(v[i]).append('"');
            return sb.append("]}").toString();
        }
        throw new IllegalArgumentException("tag " + t);
    }

    static void writeServerFiles(String seed) throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + harnessPort(),
                "online-mode=false",
                "level-name=world",
                "level-seed=" + seed,
                "generate-structures=true",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "spawn-monsters=false",
                "spawn-animals=false",
                "allow-nether=true",
                "difficulty=normal",
                "") + "\n");
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 6000; i++) {
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

    /// $KILN_HARNESS_PORT, else the first free port of 25581-25583 (waits while all are busy).
    static String harnessPort() {
        String env = System.getenv("KILN_HARNESS_PORT");
        if (env != null) return env;
        for (int i = 0; i < 900; i++) {
            for (int p = 25581; p <= 25583; p++) {
                try (var s = new java.net.ServerSocket(p)) {
                    return Integer.toString(p);
                } catch (java.io.IOException e) {
                    // busy
                }
            }
            try {
                Thread.sleep(2000);
            } catch (InterruptedException e) {
                throw new IllegalStateException(e);
            }
        }
        throw new IllegalStateException("no free harness port");
    }
}
