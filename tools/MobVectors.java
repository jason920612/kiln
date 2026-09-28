// Differential test vectors for Kiln's mobs: runs mob scenarios in a real vanilla 26.3 dedicated
// server (started in-process, port 25597) and records every mob's state after every tick:
// position, velocity, rotations (body, head, pitch), health, hurt time, target and the running
// goals, plus the ticks at which the scenario's player was hit and the entities that appeared
// (arrows, drops). Tick rate is frozen and entities are ticked by hand (`tickNonPassenger`) with
// pinned random seeds, so a run is reproducible and Rust can replay it.
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/MobVectors.java <out.jsonl> [name-filter]
// (tools/mob_vectors.py sets this up)

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
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
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySpawnReason;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Mob;
import net.minecraft.world.entity.ai.goal.WrappedGoal;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

public class MobVectors {
    static final int BX = 0, BY = 100, BZ = 0;

    static final class MobSpec {
        String type;
        double x, y, z;
        float yaw;
        long seed;
        String mainHand;
        MobSpec(String type, double x, double y, double z, float yaw, long seed) {
            this.type = type; this.x = x; this.y = y; this.z = z; this.yaw = yaw; this.seed = seed;
        }
    }

    static final class Scenario {
        final String name;
        final List<MobSpec> mobs = new ArrayList<>();
        final Map<BlockPos, BlockState> blocks = new LinkedHashMap<>();
        double[] player; // x, y, z or null
        boolean playerSneaking;
        boolean playerCreative;
        long dayTime = 1000;
        long levelSeed = 1;
        int ticks = 200;
        // tick -> [mob index, amount]; the player (or nobody) hurts the mob.
        final Map<Integer, double[]> hurts = new HashMap<>();
        Scenario(String name) { this.name = name; }
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
        }, "MobVectors main");
        main.start();
        MinecraftServer server = awaitServer();
        List<Scenario> selected = new ArrayList<>();
        for (Scenario s : scenarios()) if (filter == null || s.name.contains(filter)) selected.add(s);
        System.out.println("MobVectors: " + selected.size() + " scenarios");
        server.submit(() -> prepare(server)).get();
        Thread.sleep(3000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            ServerLevel level = server.overworld();
            ServerPlayer player = mockPlayer(server, "KilnMob");
            for (Scenario s : selected) {
                try {
                    lines.add(run(level, player, s));
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
        System.out.println("MobVectors: wrote " + lines.size() + " scenarios to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=25597",
                "online-mode=false",
                "level-name=world",
                "level-type=minecraft\\:flat",
                "generator-settings={\"layers\"\\:[{\"block\"\\:\"minecraft\\:bedrock\",\"height\"\\:1}],\"biome\"\\:\"minecraft\\:plains\"}",
                "spawn-protection=0",
                "max-tick-time=-1",
                "view-distance=3",
                "simulation-distance=3",
                "sync-chunk-writes=false",
                "spawn-monsters=false",
                "spawn-animals=false",
                "generate-structures=false",
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
        var src = server.createCommandSourceStack();
        server.getCommands().performPrefixedCommand(src, "gamerule minecraft:block_drops false");
        for (int cx = -4; cx <= 4; cx++)
            for (int cz = -4; cz <= 4; cz++) {
                level.setChunkForced(cx, cz, true);
                level.getChunk(cx, cz);
            }
    }

    static ServerPlayer mockPlayer(MinecraftServer server, String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        try {
            var m = p.connection.getClass().getDeclaredMethod("markClientLoaded");
            m.setAccessible(true);
            m.invoke(p.connection);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        return p;
    }

    static final int FLAGS = Block.UPDATE_CLIENTS | Block.UPDATE_KNOWN_SHAPE | Block.UPDATE_SUPPRESS_DROPS | Block.UPDATE_SKIP_ON_PLACE;

    static AABB box() {
        return new AABB(BX - 60, BY - 40, BZ - 60, BX + 60, BY + 40, BZ + 60);
    }

    static void cleanup(ServerLevel level, Scenario s) {
        for (Entity e : level.getEntities((Entity) null, box(), e -> !(e instanceof ServerPlayer))) e.discard();
        BlockState air = net.minecraft.world.level.block.Blocks.AIR.defaultBlockState();
        for (int x = -40; x <= 40; x++)
            for (int z = -40; z <= 40; z++)
                for (int y = BY - 8; y <= BY + 8; y++) {
                    BlockPos p = new BlockPos(x, y, z);
                    if (!level.getBlockState(p).isAir()) level.setBlock(p, air, FLAGS);
                }
    }

    static Object get(Object o, String field) throws Exception {
        Class<?> c = o.getClass();
        while (c != null) {
            try {
                Field f = c.getDeclaredField(field);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException e) {
                c = c.getSuperclass();
            }
        }
        throw new NoSuchFieldException(field);
    }

    static void set(Object o, String field, Object v) throws Exception {
        Class<?> c = o.getClass();
        while (c != null) {
            try {
                Field f = c.getDeclaredField(field);
                f.setAccessible(true);
                f.set(o, v);
                return;
            } catch (NoSuchFieldException e) {
                c = c.getSuperclass();
            }
        }
        throw new NoSuchFieldException(field);
    }

    static String run(ServerLevel level, ServerPlayer player, Scenario s) throws Exception {
        for (var b : s.blocks.entrySet()) level.setBlock(b.getKey(), b.getValue(), FLAGS);
        level.getServer().getCommands().performPrefixedCommand(level.getServer().createCommandSourceStack(), "time set " + s.dayTime);
        level.updateSkyBrightness();
        int skyDarken = level.getSkyDarken();
        if (s.player != null) {
            player.setGameMode(s.playerCreative ? net.minecraft.world.level.GameType.CREATIVE : net.minecraft.world.level.GameType.SURVIVAL);
            player.snapTo(s.player[0], s.player[1], s.player[2], 0f, 0f);
            player.setShiftKeyDown(s.playerSneaking);
            player.setPose(s.playerSneaking ? net.minecraft.world.entity.Pose.CROUCHING : net.minecraft.world.entity.Pose.STANDING);
            player.setHealth(20f);
            set(player, "damageCooldownTime", 0);
        } else {
            player.setGameMode(net.minecraft.world.level.GameType.SPECTATOR);
            player.snapTo(0, 300, 0, 0f, 0f);
        }
        level.getRandom().setSeed(s.levelSeed);
        List<Entity> tracked = new ArrayList<>();
        StringBuilder specs = new StringBuilder();
        for (MobSpec spec : s.mobs) {
            EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(spec.type));
            Mob m = (Mob) type.create(level, EntitySpawnReason.COMMAND);
            m.snapTo(spec.x, spec.y, spec.z, spec.yaw, 0f);
            m.setYHeadRot(spec.yaw);
            m.setYBodyRot(spec.yaw);
            m.yHeadRotO = spec.yaw;
            m.yBodyRotO = spec.yaw;
            if (spec.mainHand != null) {
                m.setItemSlot(EquipmentSlot.MAINHAND, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(spec.mainHand))));
            }
            m.getRandom().setSeed(spec.seed);
            int eggTime = m instanceof net.minecraft.world.entity.animal.chicken.Chicken c ? (Integer) get(c, "eggTime") : 0;
            if (!level.addFreshEntity(m)) throw new IllegalStateException("could not add " + spec.type);
            tracked.add(m);
            if (specs.length() > 0) specs.append(',');
            specs.append(String.format(Locale.ROOT,
                    "{\"type\":\"%s\",\"id\":%d,\"seed\":%d,\"pos\":[%s,%s,%s],\"yaw\":%s,\"main_hand\":%s,\"egg_time\":%d}",
                    spec.type, m.getId(), spec.seed, d(spec.x), d(spec.y), d(spec.z), Float.toString(spec.yaw),
                    spec.mainHand == null ? "null" : "\"" + spec.mainHand + "\"", eggTime));
        }
        StringBuilder trace = new StringBuilder();
        StringBuilder hits = new StringBuilder();
        StringBuilder spawned = new StringBuilder();
        int initial = tracked.size();
        var levelData = (net.minecraft.world.level.storage.ServerLevelData) get(level, "serverLevelData");
        long startTime = level.getGameTime();
        for (int tick = 0; tick < s.ticks; tick++) {
            // `ServerLevel.tickTime`: the world age advances before entities tick.
            levelData.setGameTime(startTime + 1 + tick);
            // The player's own tick (not run while the server thread is ours): its hurt cooldown.
            if (s.player != null) {
                int cd = (Integer) get(player, "damageCooldownTime");
                if (cd > 0) set(player, "damageCooldownTime", cd - 1);
            }
            double[] hurt = s.hurts.get(tick);
            if (hurt != null) {
                LivingEntity target = (LivingEntity) tracked.get((int) hurt[0]);
                var src = s.player != null ? level.damageSources().playerAttack(player) : level.damageSources().generic();
                target.hurtServer(level, src, (float) hurt[1]);
            }
            float healthBefore = player.getHealth();
            for (Entity e : new ArrayList<>(tracked)) {
                if (e.isRemoved()) continue;
                // `ServerLevel.tick`: the despawn check, then the tick.
                e.checkDespawn();
                if (!e.isRemoved()) level.tickNonPassenger(e);
            }
            if (s.player != null && player.getHealth() < healthBefore) {
                if (hits.length() > 0) hits.append(',');
                hits.append(String.format(Locale.ROOT, "[%d,%s]", tick, Float.toString(healthBefore - player.getHealth())));
                player.setHealth(20f);
            }
            for (Entity e : level.getEntities((Entity) null, box(), e -> !(e instanceof ServerPlayer))) {
                if (!tracked.contains(e)) {
                    tracked.add(e);
                    if (spawned.length() > 0) spawned.append(',');
                    spawned.append(String.format(Locale.ROOT, "{\"tick\":%d,\"type\":\"%s\",\"pos\":[%s,%s,%s],\"motion\":[%s,%s,%s]}", tick,
                            BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()), d(e.getX()), d(e.getY()), d(e.getZ()),
                            d(e.getDeltaMovement().x), d(e.getDeltaMovement().y), d(e.getDeltaMovement().z)));
                }
            }
            if (tick > 0) trace.append(',');
            trace.append('[');
            for (int i = 0; i < initial; i++) {
                if (i > 0) trace.append(',');
                trace.append(state((Mob) tracked.get(i)));
            }
            trace.append(']');
        }
        StringBuilder blocks = new StringBuilder();
        for (var b : s.blocks.entrySet()) {
            if (blocks.length() > 0) blocks.append(',');
            BlockPos p = b.getKey();
            blocks.append(String.format(Locale.ROOT, "[%d,%d,%d,%d]", p.getX(), p.getY(), p.getZ(), Block.getId(b.getValue())));
        }
        StringBuilder hurts = new StringBuilder();
        for (var h : s.hurts.entrySet()) {
            if (hurts.length() > 0) hurts.append(',');
            hurts.append(String.format(Locale.ROOT, "[%d,%d,%s]", h.getKey(), (int) h.getValue()[0], d(h.getValue()[1])));
        }
        String playerJson = s.player == null ? "null"
                : String.format(Locale.ROOT, "{\"id\":%d,\"pos\":[%s,%s,%s],\"sneaking\":%b,\"creative\":%b}", player.getId(), d(s.player[0]), d(s.player[1]), d(s.player[2]), s.playerSneaking, s.playerCreative);
        return String.format(Locale.ROOT,
                "{\"name\":\"%s\",\"level_seed\":%d,\"ticks\":%d,\"game_time\":%d,\"sky_darken\":%d,\"blocks\":[%s],\"mobs\":[%s],"
                        + "\"player\":%s,\"hurts\":[%s],\"hits\":[%s],\"spawned\":[%s],\"trace\":[%s]}",
                s.name, s.levelSeed, s.ticks, startTime, skyDarken, blocks, specs, playerJson, hurts, hits, spawned, trace);
    }

    static String d(double v) {
        return Double.toString(v);
    }

    /** [id, x, y, z, dx, dy, dz, yRot, xRot, yHeadRot, yBodyRot, onGround, health, hurtTime, removed, fire, target, goals...] */
    static String state(Mob m) throws Exception {
        Vec3 p = m.position();
        Vec3 v = m.getDeltaMovement();
        StringBuilder sb = new StringBuilder();
        sb.append('[').append(m.getId()).append(',').append(d(p.x)).append(',').append(d(p.y)).append(',').append(d(p.z))
                .append(',').append(d(v.x)).append(',').append(d(v.y)).append(',').append(d(v.z))
                .append(',').append(Float.toString(m.getYRot())).append(',').append(Float.toString(m.getXRot()))
                .append(',').append(Float.toString(m.getYHeadRot())).append(',').append(Float.toString(m.yBodyRot))
                .append(',').append(m.onGround() ? 1 : 0).append(',').append(Float.toString(m.getHealth()))
                .append(',').append(m.hurtTime).append(',').append(m.isRemoved() ? 1 : 0).append(',').append(m.getRemainingFireTicks())
                .append(',').append(m.getTarget() == null ? -1 : m.getTarget().getId())
                .append(',').append(((java.util.concurrent.atomic.AtomicLong) get(m.getRandom(), "seed")).get());
        StringBuilder goals = new StringBuilder();
        for (var sel : new net.minecraft.world.entity.ai.goal.GoalSelector[] {(net.minecraft.world.entity.ai.goal.GoalSelector) get(m, "goalSelector"), (net.minecraft.world.entity.ai.goal.GoalSelector) get(m, "targetSelector")}) {
            for (WrappedGoal g : sel.getAvailableGoals()) {
                if (!g.isRunning()) continue;
                if (goals.length() > 0) goals.append(' ');
                goals.append(g.getGoal().getClass().getSimpleName());
            }
        }
        sb.append(",\"").append(goals).append("\"]");
        return sb.toString();
    }

    // ------------------------------------------------------------------ scenarios

    static BlockState parse(String block) {
        try {
            return net.minecraft.commands.arguments.blocks.BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, block, false).blockState();
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    static void floor(Scenario s, int r, String block) {
        BlockState state = parse(block);
        for (int x = -r; x <= r; x++)
            for (int z = -r; z <= r; z++) s.blocks.put(new BlockPos(BX + x, BY - 1, BZ + z), state);
    }

    static void block(Scenario s, int x, int y, int z, String block) {
        BlockState state = parse(block);
        s.blocks.put(new BlockPos(x, y, z), state);
    }

    static List<Scenario> scenarios() {
        List<Scenario> out = new ArrayList<>();
        String[] animals = {"pig", "cow", "sheep", "chicken"};
        String[] monsters = {"zombie", "skeleton", "creeper", "spider"};
        for (String a : animals) {
            for (int seed = 1; seed <= 3; seed++) {
                Scenario s = new Scenario("idle_" + a + "_" + seed);
                floor(s, 16, "minecraft:grass_block");
                s.mobs.add(new MobSpec("minecraft:" + a, 0.5, BY, 0.5, 30f * seed, 1000L * seed + 7));
                s.player = new double[] {8.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                s.ticks = 400;
                out.add(s);
            }
        }
        for (String a : monsters) {
            for (int seed = 1; seed <= 3; seed++) {
                Scenario s = new Scenario("idle_" + a + "_" + seed);
                floor(s, 16, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + a, 0.5, BY, 0.5, 45f * seed, 2000L * seed + 11);
                if (a.equals("skeleton")) m.mainHand = "minecraft:bow";
                s.mobs.add(m);
                s.player = new double[] {8.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                s.dayTime = 18000;
                s.ticks = 400;
                out.add(s);
            }
        }
        // A herd: pushing between animals.
        {
            Scenario s = new Scenario("herd_cows");
            floor(s, 12, "minecraft:grass_block");
            for (int i = 0; i < 4; i++) s.mobs.add(new MobSpec("minecraft:cow", 0.5 + i * 0.4, BY, 0.5 + (i % 2) * 0.3, 90f * i, 77 + i));
            s.ticks = 300;
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            out.add(s);
        }
        // Hurt animals panic and get knocked back.
        for (String a : animals) {
            Scenario s = new Scenario("panic_" + a);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:" + a, 0.5, BY, 0.5, 10f, 4242));
            s.player = new double[] {3.5, BY, 0.5};
            s.playerSneaking = true;
            s.hurts.put(5, new double[] {0, 1.0});
            s.hurts.put(60, new double[] {0, 1.0});
            s.ticks = 200;
            out.add(s);
        }
        // Monsters chase the player at night.
        for (String a : monsters) {
            for (int dist : new int[] {4, 9}) {
                Scenario s = new Scenario("chase_" + a + "_" + dist);
                floor(s, 20, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + a, 0.5, BY, 0.5, 0f, 5150 + dist);
                if (a.equals("skeleton")) m.mainHand = "minecraft:bow";
                s.mobs.add(m);
                s.player = new double[] {0.5 + dist, BY, 0.5};
                s.dayTime = 18000;
                s.ticks = 160;
                out.add(s);
            }
        }
        // A zombie paths around a wall to the player.
        {
            Scenario s = new Scenario("path_wall_zombie");
            floor(s, 20, "minecraft:stone");
            for (int z = -5; z <= 5; z++)
                for (int y = BY; y <= BY + 2; y++) block(s, 4, y, z, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:zombie", 0.5, BY, 0.5, 0f, 99));
            s.player = new double[] {8.5, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // A zombie climbs steps and jumps a gap.
        {
            Scenario s = new Scenario("path_steps_zombie");
            floor(s, 20, "minecraft:stone");
            block(s, 3, BY, 0, "minecraft:stone");
            block(s, 4, BY, 0, "minecraft:stone");
            block(s, 4, BY + 1, 0, "minecraft:stone");
            block(s, 5, BY, 0, "minecraft:oak_slab[type=bottom]");
            block(s, 2, BY, 1, "minecraft:oak_fence");
            s.mobs.add(new MobSpec("minecraft:zombie", 0.5, BY, 0.5, 0f, 12345));
            s.player = new double[] {7.5, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Zombies in daylight burn.
        {
            Scenario s = new Scenario("daylight_zombie");
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:zombie", 0.5, BY, 0.5, 0f, 31337));
            s.dayTime = 6000;
            s.ticks = 300;
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            out.add(s);
        }
        // Animals wander over uneven ground and water.
        {
            Scenario s = new Scenario("terrain_pig");
            floor(s, 16, "minecraft:grass_block");
            for (int x = -4; x <= 4; x++) block(s, x, BY - 1, 3, "minecraft:water");
            for (int x = -2; x <= 2; x++) block(s, x, BY, -3, "minecraft:dirt");
            block(s, 5, BY, 5, "minecraft:oak_fence");
            s.mobs.add(new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 8080));
            s.mobs.add(new MobSpec("minecraft:sheep", -2.5, BY, 1.5, 0f, 8081));
            s.ticks = 600;
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            out.add(s);
        }
        return out;
    }
}
