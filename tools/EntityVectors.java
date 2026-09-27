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
        Scenarios.fallingBlocks(out);
        Scenarios.tnt(out);
        Scenarios.orbs(out);
        Scenarios.players(out);
        return out;
    }

    // ---------------------------------------------------------------- runner

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
        // Explosion loot is not simulated by kiln-entity (it reports destroyed blocks instead).
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), "gamerule minecraft:block_drops false");
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
        for (BlockPos p : s.blocks.keySet()) {
            level.removeBlockEntity(p);
            level.setBlock(p, air, FLAGS);
        }
        if (s.region != null) {
            int[] r = s.region;
            for (int x = r[0]; x <= r[3]; x++)
                for (int y = r[1]; y <= r[4]; y++)
                    for (int z = r[2]; z <= r[5]; z++) {
                        BlockPos p = new BlockPos(BX + x, BY + y, BZ + z);
                        level.removeBlockEntity(p);
                        level.setBlock(p, air, FLAGS);
                    }
        }
        // Containers may have dropped their contents.
        for (Entity e : level.getEntities((Entity) null, box(), e -> true)) e.discard();
    }

    static AABB box() {
        return new AABB(BX - 40, BY - 250, BZ - 40, BX + 40, BY + 60, BZ + 40);
    }

    static String run(ServerLevel level, Scenario s) throws Exception {
        for (var b : s.blocks.entrySet()) level.setBlock(b.getKey(), b.getValue(), FLAGS);
        level.getRandom().setSeed(s.levelSeed);
        List<Entity> tracked = new ArrayList<>();
        StringBuilder spawnJson = new StringBuilder();
        for (Spec spec : s.entities) {
            Entity e = spawn(level, spec);
            if (e instanceof net.minecraft.server.level.ServerPlayer player) PLAYER_MOVES.put(player, (double[][]) spec.extra.get("moves"));
            tracked.add(e);
            if (spawnJson.length() > 0) spawnJson.append(',');
            spawnJson.append(specJson(spec, e));
        }
        StringBuilder trace = new StringBuilder();
        StringBuilder spawned = new StringBuilder();
        for (int tick = 0; tick < s.ticks; tick++) {
            for (Entity e : new ArrayList<>(tracked)) {
                if (e instanceof net.minecraft.server.level.ServerPlayer player) {
                    double[] m = PLAYER_MOVES.get(player)[tick];
                    Vec3 move = new Vec3(m[0], m[1], m[2]);
                    player.setDeltaMovement(move);
                    player.move(net.minecraft.world.entity.MoverType.PLAYER, move);
                } else if (!e.isRemoved()) {
                    level.tickNonPassenger(e);
                }
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
                BlockState falling = parseState((String) spec.extra.get("block"));
                bs.set(f, falling);
                spec.with("block_id", Block.getId(falling));
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
            case "player" -> {
                var profile = new com.mojang.authlib.GameProfile(java.util.UUID.nameUUIDFromBytes(new byte[] {1}), "Kiln");
                var player = new net.minecraft.server.level.ServerPlayer(level.getServer(), level, profile,
                        net.minecraft.server.level.ClientInformation.createDefault());
                if (Boolean.TRUE.equals(spec.extra.get("shift"))) {
                    player.setShiftKeyDown(true);
                    player.setPose(net.minecraft.world.entity.Pose.CROUCHING);
                }
                if (Boolean.TRUE.equals(spec.extra.get("flying"))) player.getAbilities().flying = true;
                player.setPos(spec.x, spec.y, spec.z);
                player.setDeltaMovement(spec.dx, spec.dy, spec.dz);
                if (Boolean.TRUE.equals(spec.extra.get("on_ground"))) player.setOnGround(true);
                return player;
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
            extra.append(v instanceof String str ? "\"" + str + "\"" : v instanceof double[][] a ? movesJson(a) : String.valueOf(v));
        }
        return String.format(Locale.ROOT,
                "{\"kind\":\"%s\",\"id\":%d,\"seed\":%d,\"pos\":[%s,%s,%s],\"motion\":[%s,%s,%s],\"yaw\":%s%s}",
                spec.kind, e.getId(), spec.seed, d(spec.x), d(spec.y), d(spec.z), d(spec.dx), d(spec.dy), d(spec.dz),
                Float.toString(e.getYRot()), extra);
    }

    static final Map<Entity, double[][]> PLAYER_MOVES = new HashMap<>();

    static String movesJson(double[][] a) {
        StringBuilder sb = new StringBuilder("[");
        for (int i = 0; i < a.length; i++) {
            if (i > 0) sb.append(',');
            sb.append('[').append(d(a[i][0])).append(',').append(d(a[i][1])).append(',').append(d(a[i][2])).append(']');
        }
        return sb.append(']').toString();
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
        "minecraft:red_bed[part=foot,facing=north]", "minecraft:cauldron",
        "minecraft:soul_soil", "minecraft:magma_block", "minecraft:glass", "minecraft:oak_leaves",
        "minecraft:chest[facing=north]", "minecraft:enchanting_table", "minecraft:daylight_detector",
        "minecraft:stonecutter[facing=north]", "minecraft:lectern[facing=north]", "minecraft:composter[level=3]",
        "minecraft:hay_block",
    };

    /** Blocks with non-cube collision shapes, for cluttered terrain. */
    static final String[] SHAPES = {
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight]",
        "minecraft:oak_stairs[facing=east,half=bottom,shape=straight]",
        "minecraft:oak_stairs[facing=south,half=top,shape=straight]",
        "minecraft:stone_stairs[facing=west,half=bottom,shape=inner_left]",
        "minecraft:stone_stairs[facing=north,half=bottom,shape=outer_right]",
        "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]", "minecraft:stone_slab[type=double]",
        "minecraft:oak_fence[north=true,south=true]", "minecraft:oak_fence[east=true]", "minecraft:oak_fence",
        "minecraft:cobblestone_wall[up=true,north=low,south=tall]", "minecraft:cobblestone_wall[up=false,east=low,west=low]",
        "minecraft:glass_pane[north=true,south=true]", "minecraft:iron_bars[east=true,west=true]",
        "minecraft:oak_trapdoor[half=bottom,open=false]", "minecraft:oak_trapdoor[half=top,open=false]",
        "minecraft:oak_trapdoor[facing=north,open=true]", "minecraft:oak_door[facing=east,half=lower,open=false]",
        "minecraft:white_carpet", "minecraft:snow[layers=2]", "minecraft:snow[layers=5]", "minecraft:snow[layers=8]",
        "minecraft:lantern[hanging=false]", "minecraft:iron_chain[axis=y]", "minecraft:anvil[facing=north]",
        "minecraft:cauldron", "minecraft:composter[level=0]", "minecraft:bell[attachment=floor,facing=north]",
        "minecraft:grindstone[face=floor,facing=north]", "minecraft:lectern[facing=east]",
        "minecraft:red_bed[part=head,facing=south]", "minecraft:cake[bites=3]", "minecraft:candle[candles=3]",
        "minecraft:skeleton_skull", "minecraft:flower_pot", "minecraft:end_rod[facing=up]",
        "minecraft:lightning_rod[facing=east]", "minecraft:oak_fence_gate[facing=north,open=false]",
        "minecraft:oak_fence_gate[facing=north,open=true]", "minecraft:brewing_stand",
        "minecraft:enchanting_table", "minecraft:dirt_path", "minecraft:farmland", "minecraft:soul_sand",
        "minecraft:honey_block", "minecraft:mud", "minecraft:azalea", "minecraft:sea_pickle[pickles=2,waterlogged=false]",
        "minecraft:turtle_egg[eggs=2]", "minecraft:decorated_pot", "minecraft:big_dripleaf[tilt=none]",
        "minecraft:stonecutter[facing=north]", "minecraft:campfire[lit=false]", "minecraft:daylight_detector",
        "minecraft:chest[facing=south]", "minecraft:ender_chest[facing=west]", "minecraft:conduit",
        "minecraft:ladder[facing=north]", "minecraft:vine[north=true]", "minecraft:moss_carpet",
        "minecraft:stone", "minecraft:glass", "minecraft:oak_leaves",
    };

    static final String[] ITEMS = {
        "minecraft:cobblestone", "minecraft:diamond", "minecraft:oak_log", "minecraft:iron_sword", "minecraft:ender_pearl",
        "minecraft:netherite_ingot", "minecraft:egg", "minecraft:snowball",
    };

    static double rnd(Random r, double lo, double hi) {
        return lo + r.nextDouble() * (hi - lo);
    }

    static void items(List<EntityVectors.Scenario> out) {
        floors(out);
        clutter(out);
        fluids(out);
        effects(out);
        merges(out);
        misc(out);
    }

    /** Items dropped onto a 7x7 floor of each surface, from rest and with random velocities. */
    static void floors(List<EntityVectors.Scenario> out) {
        Random r = new Random(1);
        for (String surface : SURFACES) {
            for (int k = 0; k < 4; k++) {
                var s = new EntityVectors.Scenario("item_floor/" + surface + "/" + k, r.nextLong());
                s.fill(-3, 0, -3, 3, 0, 3, surface);
                double x = rnd(r, 0.2, 0.8), z = rnd(r, 0.2, 0.8), h = rnd(r, 1.0, 4.0);
                double dx = k == 0 ? 0 : rnd(r, -0.2, 0.2), dz = k == 0 ? 0 : rnd(r, -0.2, 0.2), dy = k == 0 ? 0 : rnd(r, 0, 0.3);
                s.entity("item", x, h, z, dx, dy, dz, r.nextLong()).with("item", "minecraft:cobblestone");
                s.ticks(100);
                out.add(s);
            }
        }
    }

    /** A 7x7 floor with random shaped blocks on it; several items thrown across. */
    static void clutter(List<EntityVectors.Scenario> out) {
        Random r = new Random(2);
        for (int k = 0; k < 120; k++) {
            var s = new EntityVectors.Scenario("item_clutter/" + k, r.nextLong());
            s.fill(-3, 0, -3, 3, 0, 3, "minecraft:stone");
            int n = 6 + r.nextInt(14);
            for (int i = 0; i < n; i++) {
                int x = r.nextInt(7) - 3, z = r.nextInt(7) - 3, y = 1 + (r.nextInt(4) == 0 ? 1 : 0);
                s.block(x, y, z, SHAPES[r.nextInt(SHAPES.length)]);
            }
            int items = 1 + r.nextInt(4);
            for (int i = 0; i < items; i++) {
                s.entity("item", rnd(r, -2, 3), rnd(r, 1.5, 4.5), rnd(r, -2, 3), rnd(r, -0.25, 0.25), rnd(r, -0.1, 0.35),
                        rnd(r, -0.25, 0.25), r.nextLong()).with("item", ITEMS[r.nextInt(ITEMS.length)]).with("pickup_delay", 32767);
            }
            s.ticks(120);
            out.add(s);
        }
    }

    static void fluids(List<EntityVectors.Scenario> out) {
        Random r = new Random(3);
        // Pools of sources and random flowing levels, water and lava.
        for (int k = 0; k < 50; k++) {
            boolean lava = k % 5 == 4;
            String fluid = lava ? "minecraft:lava" : "minecraft:water";
            var s = new EntityVectors.Scenario("item_fluid/pool/" + k, r.nextLong());
            s.fill(-3, 0, -3, 3, 0, 3, "minecraft:stone");
            int depth = 1 + r.nextInt(3);
            for (int x = -2; x <= 2; x++)
                for (int z = -2; z <= 2; z++)
                    for (int y = 1; y <= depth; y++) {
                        int level = r.nextInt(3) == 0 ? 1 + r.nextInt(7) : 0;
                        if (r.nextInt(6) == 0) level = 8 + r.nextInt(8);
                        s.block(x, y, z, fluid + "[level=" + level + "]");
                    }
            for (int x = -3; x <= 3; x++)
                for (int y = 1; y <= depth; y++) {
                    s.block(x, y, -3, "minecraft:stone");
                    s.block(x, y, 3, "minecraft:stone");
                    s.block(-3, y, x, "minecraft:stone");
                    s.block(3, y, x, "minecraft:stone");
                }
            int items = 1 + r.nextInt(3);
            for (int i = 0; i < items; i++) {
                s.entity("item", rnd(r, -1.5, 2.5), rnd(r, 0.8, depth + 2.5), rnd(r, -1.5, 2.5), rnd(r, -0.2, 0.2),
                        rnd(r, -0.3, 0.3), rnd(r, -0.2, 0.2), r.nextLong())
                        .with("item", ITEMS[r.nextInt(ITEMS.length)]).with("pickup_delay", 32767);
            }
            s.ticks(120);
            out.add(s);
        }
        // Water currents along a channel.
        for (int k = 0; k < 16; k++) {
            var s = new EntityVectors.Scenario("item_fluid/current/" + k, r.nextLong());
            s.fill(-6, 0, -1, 6, 0, 1, "minecraft:stone");
            for (int x = -5; x <= 5; x++) {
                int level = Math.min(7, Math.abs(x - (k % 3 == 0 ? -5 : 0)));
                s.block(x, 1, 0, "minecraft:water[level=" + (x == -5 ? 0 : level) + "]");
                s.block(x, 1, -1, "minecraft:stone");
                s.block(x, 1, 1, "minecraft:stone");
            }
            if (k % 4 == 1) {
                s.block(2, 2, 0, "minecraft:water[level=8]");
            }
            s.entity("item", rnd(r, -4, 3), rnd(r, 1.2, 2.2), rnd(r, 0.3, 0.7), rnd(r, -0.1, 0.1), 0, 0, r.nextLong())
                    .with("pickup_delay", 32767);
            s.ticks(150);
            out.add(s);
        }
        // Bubble columns over soul sand (up) and magma (down).
        for (int k = 0; k < 12; k++) {
            boolean down = k % 2 == 1;
            var s = new EntityVectors.Scenario("item_fluid/bubble/" + k, r.nextLong());
            s.block(0, 0, 0, down ? "minecraft:magma_block" : "minecraft:soul_sand");
            int h = 2 + r.nextInt(4);
            for (int y = 1; y <= h; y++) s.block(0, y, 0, "minecraft:bubble_column[drag=" + down + "]");
            if (k % 3 == 0) s.block(0, h + 1, 0, "minecraft:water[level=0]");
            s.entity("item", rnd(r, 0.3, 0.7), rnd(r, 1.0, h + 2.0), rnd(r, 0.3, 0.7), 0, rnd(r, -0.2, 0.2), 0, r.nextLong())
                    .with("pickup_delay", 32767);
            s.ticks(100);
            out.add(s);
        }
        // Waterlogged blocks and water plants.
        String[] logged = {"minecraft:oak_slab[type=bottom,waterlogged=true]", "minecraft:oak_stairs[waterlogged=true]",
            "minecraft:seagrass", "minecraft:kelp[age=3]", "minecraft:kelp_plant", "minecraft:oak_fence[waterlogged=true]",
            "minecraft:glass_pane[waterlogged=true,east=true]", "minecraft:oak_trapdoor[half=top,waterlogged=true]"};
        for (int k = 0; k < 16; k++) {
            var s = new EntityVectors.Scenario("item_fluid/logged/" + k, r.nextLong());
            s.fill(-2, 0, -2, 2, 0, 2, "minecraft:stone");
            for (int x = -1; x <= 1; x++)
                for (int z = -1; z <= 1; z++) s.block(x, 1, z, r.nextBoolean() ? logged[r.nextInt(logged.length)] : "minecraft:water");
            s.entity("item", rnd(r, -0.5, 1.5), rnd(r, 1.2, 3), rnd(r, -0.5, 1.5), rnd(r, -0.1, 0.1), 0, rnd(r, -0.1, 0.1),
                    r.nextLong()).with("pickup_delay", 32767);
            s.ticks(100);
            out.add(s);
        }
    }

    static void effects(List<EntityVectors.Scenario> out) {
        Random r = new Random(4);
        String[] blocks = {"minecraft:cobweb", "minecraft:sweet_berry_bush[age=3]", "minecraft:powder_snow",
            "minecraft:fire", "minecraft:soul_fire", "minecraft:campfire[lit=true]", "minecraft:cactus", "minecraft:honey_block",
            "minecraft:water_cauldron[level=2]", "minecraft:lava_cauldron", "minecraft:powder_snow_cauldron[level=3]",
            "minecraft:scaffolding[distance=0,bottom=false]", "minecraft:scaffolding[distance=2,bottom=true]"};
        for (int k = 0; k < 52; k++) {
            String b = blocks[k % blocks.length];
            var s = new EntityVectors.Scenario("item_effect/" + b + "/" + k / blocks.length, r.nextLong());
            s.fill(-2, 0, -2, 2, 0, 2, b.contains("fire") && !b.contains("campfire") ? "minecraft:netherrack" : "minecraft:stone");
            for (int y = 1; y <= 2; y++) s.block(0, y, 0, b);
            if (b.contains("honey")) s.block(0, 2, 0, "minecraft:air");
            double x = k / blocks.length == 3 ? rnd(r, 1.02, 1.2) : rnd(r, 0.2, 0.8);
            s.entity("item", x, rnd(r, 1.2, 4), rnd(r, 0.2, 0.8), rnd(r, -0.1, 0.1), rnd(r, -0.1, 0.2), rnd(r, -0.1, 0.1), r.nextLong())
                    .with("item", k % 7 == 3 ? "minecraft:netherite_ingot" : "minecraft:cobblestone").with("pickup_delay", 32767);
            s.ticks(120);
            out.add(s);
        }
        // Burning items.
        for (int k = 0; k < 6; k++) {
            var s = new EntityVectors.Scenario("item_effect/burning/" + k, r.nextLong());
            s.fill(-2, 0, -2, 2, 0, 2, "minecraft:stone");
            if (k % 2 == 1) s.block(0, 1, 0, "minecraft:water");
            s.entity("item", rnd(r, 0.2, 0.8), 1.5, rnd(r, 0.2, 0.8), 0, 0, 0, r.nextLong()).with("fire", 30 + k * 20)
                    .with("pickup_delay", 32767);
            s.ticks(120);
            out.add(s);
        }
    }

    static void merges(List<EntityVectors.Scenario> out) {
        Random r = new Random(5);
        String[][] kinds = {{"minecraft:cobblestone"}, {"minecraft:cobblestone", "minecraft:diamond"},
            {"minecraft:ender_pearl"}, {"minecraft:iron_sword"}, {"minecraft:egg", "minecraft:snowball"}};
        for (int k = 0; k < 40; k++) {
            var s = new EntityVectors.Scenario("item_merge/" + k, r.nextLong());
            s.fill(-3, 0, -3, 3, 0, 3, k % 5 == 2 ? "minecraft:ice" : "minecraft:stone");
            String[] names = kinds[k % kinds.length];
            int n = 2 + r.nextInt(5);
            for (int i = 0; i < n; i++) {
                String name = names[r.nextInt(names.length)];
                int max = name.contains("sword") ? 1 : name.contains("pearl") || name.contains("egg") || name.contains("snowball") ? 16 : 64;
                s.entity("item", rnd(r, -0.5, 1.5), rnd(r, 1.0, 2.0), rnd(r, -0.5, 1.5), rnd(r, -0.05, 0.05), 0,
                        rnd(r, -0.05, 0.05), r.nextLong()).with("item", name).with("count", 1 + r.nextInt(max))
                        .with("pickup_delay", r.nextInt(3) == 0 ? 0 : 40).with("age", r.nextInt(4) == 0 ? 5950 : r.nextInt(100));
            }
            s.ticks(200);
            out.add(s);
        }
    }

    static void misc(List<EntityVectors.Scenario> out) {
        Random r = new Random(6);
        // Spawned inside blocks: pushed out toward the nearest free side.
        for (int k = 0; k < 20; k++) {
            var s = new EntityVectors.Scenario("item_stuck/" + k, r.nextLong());
            s.fill(-2, 0, -2, 2, 3, 2, "minecraft:stone");
            int open = r.nextInt(5);
            int[][] sides = {{0, 2, -1}, {0, 2, 1}, {-1, 2, 0}, {1, 2, 0}, {0, 3, 0}};
            for (int i = 0; i <= open; i++) s.block(sides[i][0], sides[i][1], sides[i][2], "minecraft:air");
            s.block(0, 4, 0, "minecraft:air");
            s.entity("item", rnd(r, 0.3, 0.7), rnd(r, 2.1, 2.6), rnd(r, 0.3, 0.7), 0, 0, 0, r.nextLong()).with("pickup_delay", 32767);
            s.ticks(60);
            out.add(s);
        }
        // Fast items: long falls onto slime, hay and into water (fall distance clip), thrown into walls.
        for (int k = 0; k < 20; k++) {
            var s = new EntityVectors.Scenario("item_fast/" + k, r.nextLong());
            String floor = k % 3 == 0 ? "minecraft:slime_block" : k % 3 == 1 ? "minecraft:stone" : "minecraft:hay_block";
            s.fill(-4, 0, -4, 4, 0, 4, floor);
            if (k % 4 == 1) s.fill(-4, 1, -4, 4, 2, 4, "minecraft:water");
            if (k % 5 == 2) s.fill(3, 1, -4, 3, 6, 4, "minecraft:stone");
            s.entity("item", rnd(r, -1, 1), rnd(r, 15, 22), rnd(r, -1, 1), rnd(r, -1.2, 1.2), rnd(r, -1.5, 0.5), rnd(r, -1.2, 1.2),
                    r.nextLong()).with("pickup_delay", 32767).with("fall_distance", 3.0);
            s.ticks(100);
            out.add(s);
        }
        // Despawn at age 6000 and the infinite lifetime marker.
        for (int k = 0; k < 4; k++) {
            var s = new EntityVectors.Scenario("item_age/" + k, r.nextLong());
            s.fill(-1, 0, -1, 1, 0, 1, "minecraft:stone");
            s.entity("item", 0.5, 1.0, 0.5, 0, 0, 0, r.nextLong()).with("age", k == 3 ? -32768 : 5980 + k).with("pickup_delay", k == 2 ? 32767 : 5);
            s.ticks(40);
            out.add(s);
        }
        // Falling out of the world.
        var s = new EntityVectors.Scenario("item_void/0", r.nextLong());
        s.entity("item", 0.5, -60 - EntityVectors.BY, 0.5, 0, -3, 0, r.nextLong());
        s.ticks(30);
        out.add(s);
    }

    static void fallingBlocks(List<EntityVectors.Scenario> out) {
        Random r = new Random(7);
        String[] falling = {"minecraft:sand", "minecraft:gravel", "minecraft:red_sand", "minecraft:white_concrete_powder",
            "minecraft:anvil[facing=east]", "minecraft:suspicious_sand", "minecraft:dragon_egg"};
        String[] targets = {"minecraft:stone", "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]", "minecraft:torch",
            "minecraft:white_carpet", "minecraft:snow[layers=1]", "minecraft:snow[layers=3]", "minecraft:water", "minecraft:lava",
            "minecraft:oak_fence", "minecraft:short_grass", "minecraft:cobweb", "minecraft:rail", "minecraft:chest[facing=north]",
            "minecraft:water[level=3]", "minecraft:oak_trapdoor[half=bottom]", "minecraft:hopper", "minecraft:fire",
            "minecraft:glow_lichen[down=true]", "minecraft:oak_leaves", "minecraft:soul_sand", "minecraft:honey_block",
            "minecraft:powder_snow", "minecraft:scaffolding[distance=0,bottom=false]", "minecraft:cactus"};
        for (int k = 0; k < 100; k++) {
            String f = falling[k % falling.length];
            String t = targets[r.nextInt(targets.length)];
            var s = new EntityVectors.Scenario("falling/" + k, r.nextLong());
            s.fill(-2, 0, -2, 2, 0, 2, "minecraft:stone");
            s.block(0, 1, 0, t);
            if (t.contains("torch") || t.contains("rail") || t.contains("carpet") || t.contains("grass")) s.block(0, 0, 0, "minecraft:stone");
            if (r.nextInt(4) == 0) s.block(0, 2, 0, t.contains("water") ? "minecraft:water" : "minecraft:air");
            double h = 2 + r.nextInt(8);
            double dx = r.nextInt(3) == 0 ? rnd(r, -0.1, 0.1) : 0;
            s.entity("falling_block", 0.5, h, 0.5, dx, 0, 0, r.nextLong()).with("block", f).with("time", r.nextInt(4) == 0 ? 1 : 0);
            s.region = new int[] {-3, 0, -3, 3, 12, 3};
            s.ticks(60);
            out.add(s);
        }
        // Long falls (drop after 600 ticks is too long; out of world after 100 below min y).
        var s = new EntityVectors.Scenario("falling/void", r.nextLong());
        s.entity("falling_block", 0.5, -EntityVectors.BY - 120, 0.5, 0, -2, 0, r.nextLong()).with("block", "minecraft:sand").with("time", 95);
        s.ticks(20);
        out.add(s);
    }

    static void tnt(List<EntityVectors.Scenario> out) {
        Random r = new Random(8);
        String[] floors = {"minecraft:stone", "minecraft:ice", "minecraft:slime_block", "minecraft:honey_block", "minecraft:soul_sand"};
        // Fuses and movement without explosions reaching anything interesting.
        for (int k = 0; k < 20; k++) {
            var s = new EntityVectors.Scenario("tnt/move/" + k, r.nextLong());
            s.fill(-4, 0, -4, 4, 0, 4, floors[k % floors.length]);
            if (k % 4 == 3) s.fill(-4, 1, -4, 4, 2, 4, "minecraft:water");
            s.entity("tnt", rnd(r, -1, 1), rnd(r, 1, 4), rnd(r, -1, 1), rnd(r, -0.1, 0.1), rnd(r, 0, 0.3), rnd(r, -0.1, 0.1), r.nextLong())
                    .with("fuse", 30 + r.nextInt(40));
            s.ticks(25);
            out.add(s);
        }
        // Explosions over terrain with items and TNT around (no loot, no chained TNT blocks).
        String[] terrain = {"minecraft:stone", "minecraft:dirt", "minecraft:obsidian", "minecraft:oak_planks", "minecraft:glass",
            "minecraft:sand", "minecraft:water"};
        for (int k = 0; k < 40; k++) {
            var s = new EntityVectors.Scenario("tnt/explode/" + k, r.nextLong());
            s.fill(-5, -3, -5, 5, 0, 5, terrain[r.nextInt(terrain.length)]);
            for (int i = 0; i < 12; i++) s.block(r.nextInt(11) - 5, 1 + r.nextInt(2), r.nextInt(11) - 5, terrain[r.nextInt(terrain.length)]);
            if (k % 3 == 0) s.fill(-2, 1, 2, 2, 3, 2, "minecraft:obsidian");
            s.entity("tnt", rnd(r, 0, 1), 1.0, rnd(r, 0, 1), 0, 0, 0, r.nextLong()).with("fuse", 1 + r.nextInt(3));
            int n = r.nextInt(5);
            for (int i = 0; i < n; i++) {
                if (r.nextBoolean()) {
                    s.entity("tnt", rnd(r, -4, 5), rnd(r, 1, 3), rnd(r, -4, 5), 0, 0, 0, r.nextLong()).with("fuse", 20 + r.nextInt(30));
                } else {
                    s.entity("item", rnd(r, -4, 5), rnd(r, 1, 3), rnd(r, -4, 5), 0, 0, 0, r.nextLong())
                            .with("item", r.nextBoolean() ? "minecraft:nether_star" : "minecraft:cobblestone").with("pickup_delay", 32767);
                }
            }
            s.region = new int[] {-6, -4, -6, 6, 4, 6};
            s.ticks(12);
            out.add(s);
        }
    }

    static void orbs(List<EntityVectors.Scenario> out) {
        Random r = new Random(9);
        String[] floors = {"minecraft:stone", "minecraft:ice", "minecraft:soul_sand", "minecraft:water", "minecraft:lava",
            "minecraft:slime_block", "minecraft:honey_block", "minecraft:cobweb"};
        for (int k = 0; k < 40; k++) {
            var s = new EntityVectors.Scenario("orb/" + k, r.nextLong());
            String f = floors[k % floors.length];
            s.fill(-3, 0, -3, 3, 0, 3, "minecraft:stone");
            if (f.contains("water") || f.contains("lava") || f.contains("cobweb")) s.fill(-2, 1, -2, 2, 2, 2, f);
            else s.fill(-3, 0, -3, 3, 0, 3, f);
            int n = 1 + r.nextInt(3);
            for (int i = 0; i < n; i++) {
                s.entity("experience_orb", rnd(r, -1, 2), rnd(r, 1.2, 4), rnd(r, -1, 2), rnd(r, -0.2, 0.2), rnd(r, 0, 0.3),
                        rnd(r, -0.2, 0.2), r.nextLong()).with("value", 1 + r.nextInt(3));
            }
            s.ticks(100);
            out.add(s);
        }
        // Enough orbs that ids differ by 40 and merge.
        for (int k = 0; k < 3; k++) {
            var s = new EntityVectors.Scenario("orb/merge/" + k, r.nextLong());
            s.fill(-3, 0, -3, 3, 0, 3, "minecraft:stone");
            for (int i = 0; i < 45; i++) {
                s.entity("experience_orb", rnd(r, 0, 1), rnd(r, 1.1, 1.5), rnd(r, 0, 1), 0, 0, 0, r.nextLong()).with("value", 1 + k % 2);
            }
            s.ticks(45);
            out.add(s);
        }
        // Stuck inside blocks.
        for (int k = 0; k < 6; k++) {
            var s = new EntityVectors.Scenario("orb/stuck/" + k, r.nextLong());
            s.fill(-1, 0, -1, 1, 2, 1, "minecraft:stone");
            s.block(k % 3 - 1, 1, 0, "minecraft:air");
            s.entity("experience_orb", rnd(r, 0.3, 0.7), rnd(r, 1.2, 1.7), rnd(r, 0.3, 0.7), 0, 0, 0, r.nextLong());
            s.ticks(40);
            out.add(s);
        }
    }

    /** Server-side player.move(PLAYER, delta) calls: step-up, collisions, sneaking at edges. */
    static void players(List<EntityVectors.Scenario> out) {
        Random r = new Random(10);
        for (int k = 0; k < 120; k++) {
            var s = new EntityVectors.Scenario("player/" + k, r.nextLong());
            boolean edge = k % 3 == 0;
            if (edge) {
                s.fill(-1, 0, -1, 1, 0, 1, "minecraft:stone");
                if (k % 2 == 0) s.block(1, 0, 1, "minecraft:oak_slab[type=bottom]");
            } else {
                s.fill(-4, 0, -4, 4, 0, 4, "minecraft:stone");
                int n = 8 + r.nextInt(12);
                for (int i = 0; i < n; i++) s.block(r.nextInt(9) - 4, 1 + (r.nextInt(5) == 0 ? 1 : 0), r.nextInt(9) - 4, SHAPES[r.nextInt(SHAPES.length)]);
                s.block(0, 1, 0, "minecraft:air");
                s.block(0, 2, 0, "minecraft:air");
                s.block(0, 3, 0, "minecraft:air");
            }
            boolean shift = edge || r.nextInt(4) == 0;
            int ticks = 40;
            double[][] moves = new double[ticks][];
            double speed = shift ? 0.08 : 0.22;
            double yaw = r.nextDouble() * Math.PI * 2;
            for (int t = 0; t < ticks; t++) {
                if (r.nextInt(8) == 0) yaw = r.nextDouble() * Math.PI * 2;
                double dy = r.nextInt(12) == 0 ? 0.42 : -0.0784000015258789;
                moves[t] = new double[] {Math.cos(yaw) * speed * (0.5 + r.nextDouble()), dy, Math.sin(yaw) * speed * (0.5 + r.nextDouble())};
            }
            s.entity("player", 0.5, 1.0, 0.5, 0, 0, 0, r.nextLong()).with("shift", shift).with("on_ground", true).with("moves", moves);
            s.ticks(ticks);
            out.add(s);
        }
    }
}
