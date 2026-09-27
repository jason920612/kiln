// Differential test vectors for kiln-entity: runs entity scenarios in a real vanilla 26.3
// dedicated server (started in-process), ticking the entities by hand the way ServerLevel does
// (commonTick + tick) and recording every entity's state after every tick.
//
// Each scenario builds a few blocks around a fixed base, spawns entities with given positions,
// velocities and random seeds, ticks them and dumps one JSON line: the setup (so Rust can replay
// it) and the per-tick trace. Doubles are printed with Double.toString (exact round trip).
//
// usage (cwd = a scratch server directory, e.g. work/wp4-entities/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/EntityVectors.java <out.jsonl> [name-filter]
// (tools/entity_physics_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.entity.item.FallingBlockEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

public class EntityVectors {
    static final int BX = 1024, BY = 100, BZ = 1024;
    static final int CLEAR = 12, CLEAR_DOWN = 16, CLEAR_UP = 24;

    // ---------------------------------------------------------------- scenario model

    static final class Spec {
        final String kind;
        final double x, y, z, dx, dy, dz;
        final long seed;
        final Map<String, Object> extra = new LinkedHashMap<>();

        Spec(String kind, double x, double y, double z, double dx, double dy, double dz, long seed) {
            this.kind = kind;
            this.x = x;
            this.y = y;
            this.z = z;
            this.dx = dx;
            this.dy = dy;
            this.dz = dz;
            this.seed = seed;
        }

        Spec with(String k, Object v) {
            extra.put(k, v);
            return this;
        }
    }

    static final class Scenario {
        final String name;
        final Map<BlockPos, BlockState> blocks = new LinkedHashMap<>();
        final List<Spec> entities = new ArrayList<>();
        int ticks = 60;
        long levelSeed;
        /** Region (relative to the base) whose final blocks are recorded, or null. */
        int[] region;

        Scenario(String name, long levelSeed) {
            this.name = name;
            this.levelSeed = levelSeed;
        }

        Scenario block(int x, int y, int z, String state) {
            blocks.put(new BlockPos(BX + x, BY + y, BZ + z), parseState(state));
            return this;
        }

        Scenario fill(int x0, int y0, int z0, int x1, int y1, int z1, String state) {
            for (int x = x0; x <= x1; x++)
                for (int y = y0; y <= y1; y++)
                    for (int z = z0; z <= z1; z++) block(x, y, z, state);
            return this;
        }

        Spec entity(String kind, double x, double y, double z, double dx, double dy, double dz, long seed) {
            Spec s = new Spec(kind, BX + x, BY + y, BZ + z, dx, dy, dz, seed);
            entities.add(s);
            return s;
        }

        Scenario ticks(int n) {
            ticks = n;
            return this;
        }
    }

    static BlockState parseState(String s) {
        try {
            return net.minecraft.commands.arguments.blocks.BlockStateParser
                    .parseForBlock(BuiltInRegistries.BLOCK, s, false).blockState();
        } catch (Exception e) {
            throw new IllegalArgumentException("bad block state " + s, e);
        }
    }

    // ---------------------------------------------------------------- scenarios

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        Scenarios.items(out);
        return out;
    }

    // ---------------------------------------------------------------- runner

    public static void main(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        String filter = args.length > 1 ? args[1] : null;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "EntityVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> all = scenarios();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : all) {
            if (filter == null || s.name.contains(filter)) selected.add(s);
        }
        System.out.println("EntityVectors: " + selected.size() + " scenarios");
        CompletableFuture<Void> prepared = server.submit(() -> prepare(server));
        prepared.get();
        // Let the forced chunks reach entity-ticking status.
        Thread.sleep(3000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            ServerLevel level = server.overworld();
            for (Scenario s : selected) {
                try {
                    lines.add(run(level, s));
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"name\":\"" + s.name + "\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
                }
                cleanup(level, s);
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("EntityVectors: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=25592",
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:the_void\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "enable-rcon=false",
                "enable-query=false",
                "spawn-monsters=false",
                "generate-structures=false",
                "") + "\n");
        // A fresh world every run.
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    /** Finds the server through the "Server thread" task (MinecraftServer.spin's AtomicReference). */
    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 600; i++) {
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

    static void prepare(MinecraftServer server) {
        ServerLevel level = server.overworld();
        level.tickRateManager().setFrozen(true);
        for (int cx = (BX >> 4) - 3; cx <= (BX >> 4) + 3; cx++) {
            for (int cz = (BZ >> 4) - 3; cz <= (BZ >> 4) + 3; cz++) {
                level.setChunkForced(cx, cz, true);
                level.getChunk(cx, cz);
            }
        }
    }

    static final int FLAGS =
            Block.UPDATE_CLIENTS | Block.UPDATE_KNOWN_SHAPE | Block.UPDATE_SUPPRESS_DROPS | Block.UPDATE_SKIP_ON_PLACE;

    static void cleanup(ServerLevel level, Scenario s) {
        for (Entity e : level.getEntities((Entity) null, box(), e -> true)) e.discard();
        BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (BlockPos p : s.blocks.keySet()) level.setBlock(p, air, FLAGS);
        if (s.region != null) {
            int[] r = s.region;
            for (int x = r[0]; x <= r[3]; x++)
                for (int y = r[1]; y <= r[4]; y++)
                    for (int z = r[2]; z <= r[5]; z++) level.setBlock(new BlockPos(BX + x, BY + y, BZ + z), air, FLAGS);
        }
    }

    static AABB box() {
        return new AABB(BX - 40, BY - 40, BZ - 40, BX + 40, BY + 60, BZ + 40);
    }

    static String run(ServerLevel level, Scenario s) throws Exception {
        for (var b : s.blocks.entrySet()) level.setBlock(b.getKey(), b.getValue(), FLAGS);
        level.getRandom().setSeed(s.levelSeed);
        List<Entity> tracked = new ArrayList<>();
        StringBuilder spawnJson = new StringBuilder();
        for (Spec spec : s.entities) {
            Entity e = spawn(level, spec);
            tracked.add(e);
            if (spawnJson.length() > 0) spawnJson.append(',');
            spawnJson.append(specJson(spec, e));
        }
        StringBuilder trace = new StringBuilder();
        StringBuilder spawned = new StringBuilder();
        for (int tick = 0; tick < s.ticks; tick++) {
            for (Entity e : new ArrayList<>(tracked)) {
                if (!e.isRemoved()) level.tickNonPassenger(e);
            }
            for (Entity e : level.getEntities((Entity) null, box(), e -> true)) {
                if (!tracked.contains(e)) {
                    tracked.add(e);
                    if (spawned.length() > 0) spawned.append(',');
                    spawned.append(String.format(Locale.ROOT, "{\"tick\":%d,\"type\":\"%s\",\"state\":%s}", tick,
                            BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()), state(e)));
                }
            }
            if (tick > 0) trace.append(',');
            trace.append('[');
            for (int i = 0; i < tracked.size(); i++) {
                if (i > 0) trace.append(',');
                trace.append(state(tracked.get(i)));
            }
            trace.append(']');
        }
        StringBuilder blocks = new StringBuilder();
        for (var b : s.blocks.entrySet()) {
            if (blocks.length() > 0) blocks.append(',');
            BlockPos p = b.getKey();
            blocks.append(String.format(Locale.ROOT, "[%d,%d,%d,%d]", p.getX(), p.getY(), p.getZ(),
                    Block.getId(b.getValue())));
        }
        StringBuilder finalBlocks = new StringBuilder();
        if (s.region != null) {
            int[] r = s.region;
            for (int x = r[0]; x <= r[3]; x++)
                for (int y = r[1]; y <= r[4]; y++)
                    for (int z = r[2]; z <= r[5]; z++) {
                        BlockPos p = new BlockPos(BX + x, BY + y, BZ + z);
                        int id = Block.getId(level.getBlockState(p));
                        if (id == 0) continue;
                        if (finalBlocks.length() > 0) finalBlocks.append(',');
                        finalBlocks.append(String.format(Locale.ROOT, "[%d,%d,%d,%d]", p.getX(), p.getY(), p.getZ(), id));
                    }
        }
        return String.format(Locale.ROOT,
                "{\"name\":\"%s\",\"level_seed\":%d,\"ticks\":%d,\"blocks\":[%s],\"entities\":[%s],\"spawned\":[%s],"
                        + "\"final_blocks\":%s,\"trace\":[%s]}",
                s.name, s.levelSeed, s.ticks, blocks, spawnJson, spawned,
                s.region == null ? "null" : "[" + finalBlocks + "]", trace);
    }

    static Entity spawn(ServerLevel level, Spec spec) throws Exception {
        Entity e;
        switch (spec.kind) {
            case "item" -> {
                ItemEntity item = new ItemEntity(EntityTypes.ITEM, level);
                var itemType = BuiltInRegistries.ITEM.getValue(Identifier.parse((String) spec.extra.getOrDefault("item", "minecraft:stone")));
                item.setItem(new ItemStack(itemType, (Integer) spec.extra.getOrDefault("count", 1)));
                item.setPickUpDelay((Integer) spec.extra.getOrDefault("pickup_delay", 10));
                setInt(ItemEntity.class, item, "age", (Integer) spec.extra.getOrDefault("age", 0));
                e = item;
            }
            case "tnt" -> {
                PrimedTnt tnt = new PrimedTnt(EntityTypes.TNT, level);
                tnt.setFuse((Integer) spec.extra.getOrDefault("fuse", 80));
                e = tnt;
            }
            case "falling_block" -> {
                FallingBlockEntity f = new FallingBlockEntity(EntityTypes.FALLING_BLOCK, level);
                Field bs = FallingBlockEntity.class.getDeclaredField("blockState");
                bs.setAccessible(true);
                bs.set(f, parseState((String) spec.extra.get("block")));
                f.time = (Integer) spec.extra.getOrDefault("time", 0);
                if (Boolean.TRUE.equals(spec.extra.get("cancel_drop"))) {
                    Field cd = FallingBlockEntity.class.getDeclaredField("cancelDrop");
                    cd.setAccessible(true);
                    cd.set(f, true);
                }
                f.dropItem = (Boolean) spec.extra.getOrDefault("drop_item", true);
                if (spec.extra.containsKey("hurt_per_distance")) {
                    f.setHurtsEntities(((Number) spec.extra.get("hurt_per_distance")).floatValue(),
                            (Integer) spec.extra.getOrDefault("hurt_max", 40));
                }
                e = f;
            }
            case "experience_orb" -> {
                ExperienceOrb orb = new ExperienceOrb(EntityTypes.EXPERIENCE_ORB, level);
                var setValue = ExperienceOrb.class.getDeclaredMethod("setValue", int.class);
                setValue.setAccessible(true);
                setValue.invoke(orb, (Integer) spec.extra.getOrDefault("value", 1));
                setInt(ExperienceOrb.class, orb, "count", (Integer) spec.extra.getOrDefault("count", 1));
                setInt(ExperienceOrb.class, orb, "age", (Integer) spec.extra.getOrDefault("age", 0));
                e = orb;
            }
            default -> throw new IllegalArgumentException(spec.kind);
        }
        e.setPos(spec.x, spec.y, spec.z);
        e.setDeltaMovement(spec.dx, spec.dy, spec.dz);
        if (spec.extra.containsKey("yaw")) e.setYRot(((Number) spec.extra.get("yaw")).floatValue());
        if (spec.extra.containsKey("no_gravity")) e.setNoGravity(true);
        if (spec.extra.containsKey("fire")) e.setRemainingFireTicks((Integer) spec.extra.get("fire"));
        if (spec.extra.containsKey("fall_distance")) e.fallDistance = ((Number) spec.extra.get("fall_distance")).doubleValue();
        if (spec.extra.containsKey("on_ground")) e.setOnGround(true);
        e.getRandom().setSeed(spec.seed);
        if (!level.addFreshEntity(e)) throw new IllegalStateException("could not add " + spec.kind);
        return e;
    }

    static void setInt(Class<?> c, Object o, String field, int v) throws Exception {
        Field f = c.getDeclaredField(field);
        f.setAccessible(true);
        f.setInt(o, v);
    }

    static int getInt(Class<?> c, Object o, String field) {
        try {
            Field f = c.getDeclaredField(field);
            f.setAccessible(true);
            return f.getInt(o);
        } catch (Exception ex) {
            throw new RuntimeException(ex);
        }
    }

    static String specJson(Spec spec, Entity e) {
        StringBuilder extra = new StringBuilder();
        for (var kv : spec.extra.entrySet()) {
            extra.append(",\"").append(kv.getKey()).append("\":");
            Object v = kv.getValue();
            extra.append(v instanceof String str ? "\"" + str + "\"" : String.valueOf(v));
        }
        return String.format(Locale.ROOT,
                "{\"kind\":\"%s\",\"id\":%d,\"seed\":%d,\"pos\":[%s,%s,%s],\"motion\":[%s,%s,%s],\"yaw\":%s%s}",
                spec.kind, e.getId(), spec.seed, d(spec.x), d(spec.y), d(spec.z), d(spec.dx), d(spec.dy), d(spec.dz),
                Float.toString(e.getYRot()), extra);
    }

    static String d(double v) {
        return Double.toString(v);
    }

    /** [id, x, y, z, dx, dy, dz, onGround, horizontalCollision, verticalCollision, fallDistance, removed, fire, air, ...kind-specific] */
    static String state(Entity e) {
        Vec3 p = e.position();
        Vec3 v = e.getDeltaMovement();
        StringBuilder sb = new StringBuilder();
        sb.append('[').append(e.getId()).append(',').append(d(p.x)).append(',').append(d(p.y)).append(',').append(d(p.z))
                .append(',').append(d(v.x)).append(',').append(d(v.y)).append(',').append(d(v.z))
                .append(',').append(e.onGround() ? 1 : 0).append(',').append(e.horizontalCollision ? 1 : 0)
                .append(',').append(e.verticalCollision ? 1 : 0).append(',').append(d(e.fallDistance))
                .append(',').append(e.isRemoved() ? 1 : 0).append(',').append(e.getRemainingFireTicks())
                .append(',').append(e.getAirSupply());
        if (e instanceof ItemEntity item) {
            sb.append(',').append(item.getItem().getCount()).append(',').append(item.getAge())
                    .append(',').append(getInt(ItemEntity.class, item, "pickupDelay"))
                    .append(',').append(getInt(ItemEntity.class, item, "health"));
        } else if (e instanceof PrimedTnt tnt) {
            sb.append(',').append(tnt.getFuse());
        } else if (e instanceof FallingBlockEntity f) {
            sb.append(',').append(f.time).append(',').append(Block.getId(f.getBlockState()));
        } else if (e instanceof ExperienceOrb orb) {
            sb.append(',').append(orb.getValue()).append(',').append(getInt(ExperienceOrb.class, orb, "count"))
                    .append(',').append(getInt(ExperienceOrb.class, orb, "age"));
        }
        return sb.append(']').toString();
    }
}

/** The scenario catalogue. Seeds make every run identical. */
class Scenarios {
    static final String[] SURFACES = {
        "minecraft:stone", "minecraft:ice", "minecraft:packed_ice", "minecraft:blue_ice", "minecraft:slime_block",
        "minecraft:soul_sand", "minecraft:honey_block", "minecraft:grass_block", "minecraft:mud", "minecraft:farmland",
        "minecraft:dirt_path", "minecraft:snow[layers=1]", "minecraft:snow[layers=3]", "minecraft:snow[layers=7]",
        "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]", "minecraft:white_carpet",
        "minecraft:red_bed[part=foot,facing=north]", "minecraft:hopper", "minecraft:cauldron",
        "minecraft:soul_soil", "minecraft:magma_block", "minecraft:glass", "minecraft:oak_leaves",
        "minecraft:chest[facing=north]", "minecraft:enchanting_table", "minecraft:daylight_detector",
        "minecraft:stonecutter[facing=north]", "minecraft:lectern[facing=north]", "minecraft:composter[level=3]",
    };

    static void items(List<EntityVectors.Scenario> out) {
        Random r = new Random(1);
        // Items dropped onto a 5x5 floor of each surface, from rest and with random velocities.
        for (String surface : SURFACES) {
            for (int k = 0; k < 4; k++) {
                var s = new EntityVectors.Scenario("item_floor/" + surface + "/" + k, r.nextLong());
                s.fill(-2, 0, -2, 2, 0, 2, surface);
                double x = 0.2 + r.nextDouble() * 0.6, z = 0.2 + r.nextDouble() * 0.6;
                double h = 1.0 + r.nextDouble() * 3;
                double dx = k == 0 ? 0 : (r.nextDouble() - 0.5) * 0.4;
                double dz = k == 0 ? 0 : (r.nextDouble() - 0.5) * 0.4;
                double dy = k == 0 ? 0 : r.nextDouble() * 0.3;
                s.entity("item", x, h, z, dx, dy, dz, r.nextLong()).with("item", "minecraft:cobblestone");
                s.ticks(80);
                out.add(s);
            }
        }
    }
}
