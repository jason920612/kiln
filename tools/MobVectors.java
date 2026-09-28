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
        Integer age;
        Integer inLove;
        /// SNBT read by the mob's `readAdditionalSaveData` before it joins the level (both sides
        /// load it the same way: slime sizes, owners, carried blocks...).
        String nbt;
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
        String playerMainHand;
        long dayTime = 1000;
        long levelSeed = 1;
        int ticks = 200;
        /// Brain-driven mobs Kiln approximates with goals: the replay reports where Kiln
        /// diverges instead of failing.
        boolean diverges;
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
                "server-port=" + System.getenv().getOrDefault("KILN_MOB_PORT", "25597"),
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
            player.setItemSlot(EquipmentSlot.MAINHAND, s.playerMainHand == null ? ItemStack.EMPTY
                    : new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(s.playerMainHand))));
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
            String nbtJson = "null";
            if (spec.nbt != null) {
                net.minecraft.nbt.CompoundTag tag = net.minecraft.nbt.TagParser.parseCompoundFully(spec.nbt);
                var in = net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag);
                java.lang.reflect.Method read = null;
                for (Class<?> c = m.getClass(); c != null && read == null; c = c.getSuperclass()) {
                    for (java.lang.reflect.Method mm : c.getDeclaredMethods()) {
                        if (mm.getName().equals("readAdditionalSaveData") && mm.getParameterCount() == 1) read = mm;
                    }
                }
                read.setAccessible(true);
                read.invoke(m, in);
                nbtJson = tagJson(tag);
            }
            if (spec.age != null) ((net.minecraft.world.entity.AgeableMob) m).setAge(spec.age);
            if (spec.inLove != null) ((net.minecraft.world.entity.animal.Animal) m).setInLoveTime(spec.inLove);
            m.getRandom().setSeed(spec.seed);
            pinCubeMoveYaw(m);
            int eggTime = m instanceof net.minecraft.world.entity.animal.chicken.Chicken c ? (Integer) get(c, "eggTime") : 0;
            if (!level.addFreshEntity(m)) throw new IllegalStateException("could not add " + spec.type);
            tracked.add(m);
            if (specs.length() > 0) specs.append(',');
            specs.append(String.format(Locale.ROOT,
                    "{\"type\":\"%s\",\"id\":%d,\"seed\":%d,\"pos\":[%s,%s,%s],\"yaw\":%s,\"main_hand\":%s,\"egg_time\":%d,\"age\":%d,\"in_love\":%d,\"nbt\":%s}",
                    spec.type, m.getId(), spec.seed, d(spec.x), d(spec.y), d(spec.z), Float.toString(spec.yaw),
                    spec.mainHand == null ? "null" : "\"" + spec.mainHand + "\"", eggTime,
                    spec.age == null ? 0 : spec.age, spec.inLove == null ? 0 : spec.inLove, nbtJson));
        }
        StringBuilder trace = new StringBuilder();
        StringBuilder hits = new StringBuilder();
        StringBuilder spawned = new StringBuilder();
        // Mobs that appear during the scenario (babies, split slimes, converted zombies) get a
        // pinned random, their head and body turned to their yaw (the constructor's random yaw
        // is not reproducible) and join the trace.
        List<Mob> pinned = new ArrayList<>();
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
                    if (e instanceof Mob nm) {
                        nm.getRandom().setSeed(7777L * (tick + 1) + pinned.size());
                        nm.setYHeadRot(nm.getYRot());
                        nm.yHeadRotO = nm.getYRot();
                        nm.setYBodyRot(nm.getYRot());
                        nm.yBodyRotO = nm.getYRot();
                        if (nm instanceof net.minecraft.world.entity.animal.chicken.Chicken) set(nm, "eggTime", 6000 + pinned.size());
                        pinCubeMoveYaw(nm);
                        pinned.add(nm);
                    }
                }
            }
            if (tick > 0) trace.append(',');
            trace.append('[');
            for (int i = 0; i < initial; i++) {
                if (i > 0) trace.append(',');
                trace.append(state((Mob) tracked.get(i)));
            }
            for (Mob nm : pinned) {
                trace.append(',');
                trace.append(state(nm));
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
                : String.format(Locale.ROOT, "{\"id\":%d,\"pos\":[%s,%s,%s],\"sneaking\":%b,\"creative\":%b,\"main_hand\":%s}", player.getId(), d(s.player[0]), d(s.player[1]), d(s.player[2]), s.playerSneaking, s.playerCreative,
                        s.playerMainHand == null ? "null" : "\"" + s.playerMainHand + "\"");
        return String.format(Locale.ROOT,
                "{\"name\":\"%s\",\"diverges\":%b,\"level_seed\":%d,\"ticks\":%d,\"game_time\":%d,\"sky_darken\":%d,\"blocks\":[%s],\"mobs\":[%s],"
                        + "\"player\":%s,\"hurts\":[%s],\"hits\":[%s],\"spawned\":[%s],\"trace\":[%s]}",
                s.name, s.diverges, s.levelSeed, s.ticks, startTime, skyDarken, blocks, specs, playerJson, hurts, hits, spawned, trace);
    }

    /// NBT as typed JSON: {"b":1}, {"i":2}, {"s":..}, {"L":"n"}, {"f":x}, {"d":x}, {"str":".."},
    /// {"c":{...}}, {"l":[...]}, {"ba":[..]}, {"ia":[..]}, {"la":["n"..]}.
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

    /// A cube mob's move control remembers its constructor's (random) yaw: both sides pin it to
    /// the mob's current yaw.
    static void pinCubeMoveYaw(Mob m) throws Exception {
        Object mc = m.getMoveControl();
        if (mc.getClass().getSimpleName().equals("CubeMobMoveControl")) {
            set(mc, "yRot", 180.0F * m.getYRot() / 3.1415927F);
        }
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
        // ---------------------------------------------------------- slice 2: breeding
        String[][] breeders = {{"cow", "minecraft:wheat"}, {"pig", "minecraft:carrot"}, {"sheep", "minecraft:wheat"}, {"chicken", "minecraft:wheat_seeds"}};
        for (String[] b : breeders) {
            Scenario s = new Scenario("breed_" + b[0]);
            floor(s, 16, "minecraft:grass_block");
            MobSpec m1 = new MobSpec("minecraft:" + b[0], 0.5, BY, 0.5, 20f, 6100);
            MobSpec m2 = new MobSpec("minecraft:" + b[0], 3.5, BY, 1.5, 200f, 6101);
            m1.inLove = 600;
            m2.inLove = 590;
            s.mobs.add(m1);
            s.mobs.add(m2);
            s.player = new double[] {9.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("follow_parent_cow");
            floor(s, 16, "minecraft:grass_block");
            MobSpec baby = new MobSpec("minecraft:cow", 0.5, BY, 0.5, 0f, 6200);
            baby.age = -24000;
            s.mobs.add(baby);
            s.mobs.add(new MobSpec("minecraft:cow", 6.5, BY, 2.5, 90f, 6201));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("grow_up_pig");
            floor(s, 16, "minecraft:grass_block");
            MobSpec baby = new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 6300);
            baby.age = -150;
            s.mobs.add(baby);
            MobSpec parent = new MobSpec("minecraft:pig", 4.5, BY, 0.5, 0f, 6301);
            parent.age = 100;
            s.mobs.add(parent);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            out.add(s);
        }
        for (String[] b : breeders) {
            Scenario s = new Scenario("tempt_" + b[0]);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:" + b[0], 0.5, BY, 0.5, 0f, 6400));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = b[1];
            s.ticks = 200;
            out.add(s);
        }
        scenariosVillagers(out);
        scenariosFlyers(out);
        return out;
    }

    // ---------------------------------------------------------- slice 2: cubes and flyers
    static void scenariosFlyers(List<Scenario> out) {
        String[][] cubes = {{"slime", "{Size:0}"}, {"slime", "{Size:1}"}, {"slime", "{Size:3}"}, {"magma_cube", "{Size:0}"}, {"magma_cube", "{Size:1}"}};
        int n = 0;
        for (String[] c : cubes) {
            n++;
            Scenario s = new Scenario("idle_" + c[0] + "_" + n);
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:" + c[0], 0.5, BY, 0.5, 40f * n, 9100L + n);
            m.nbt = c[1];
            s.mobs.add(m);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = n;
            s.ticks = 300;
            out.add(s);
        }
        for (String[] c : new String[][] {{"slime", "{Size:1}"}, {"magma_cube", "{Size:1}"}, {"slime", "{Size:0}"}}) {
            Scenario s = new Scenario("chase_" + c[0] + "_" + c[1].charAt(6));
            floor(s, 20, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:" + c[0], 0.5, BY, 0.5, 0f, 9200 + c[1].charAt(6));
            m.nbt = c[1];
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 2.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("split_slime");
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:slime", 0.5, BY, 0.5, 0f, 9300);
            m.nbt = "{Size:3}";
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.playerCreative = true;
            s.hurts.put(5, new double[] {0, 30.0});
            s.ticks = 160;
            out.add(s);
        }
        {
            Scenario s = new Scenario("water_slime");
            floor(s, 16, "minecraft:stone");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++) {
                    block(s, x, BY - 1, z, "minecraft:water");
                    block(s, x, BY - 2, z, "minecraft:stone");
                }
            MobSpec m = new MobSpec("minecraft:slime", 0.5, BY - 1, 0.5, 0f, 9400);
            m.nbt = "{Size:1}";
            s.mobs.add(m);
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("lava_magma_cube");
            floor(s, 16, "minecraft:stone");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++) {
                    block(s, x, BY - 1, z, "minecraft:lava");
                    block(s, x, BY - 2, z, "minecraft:stone");
                }
            MobSpec m = new MobSpec("minecraft:magma_cube", 0.5, BY - 1, 0.5, 0f, 9500);
            m.nbt = "{Size:1}";
            s.mobs.add(m);
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_ghast_" + seed);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:ghast", 0.5, BY + 4, 0.5, 70f * seed, 9600 + seed));
            s.player = new double[] {18.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("shoot_ghast");
            floor(s, 20, "minecraft:stone");
            // A ceiling keeps the ghast within 4 blocks of the player's height.
            for (int x = -20; x <= 20; x++)
                for (int z = -20; z <= 20; z++) block(s, x, BY + 7, z, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:ghast", 0.5, BY + 1, 0.5, 0f, 9700));
            s.player = new double[] {14.5, BY, 0.5};
            s.ticks = 200;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_blaze_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:blaze", 0.5, BY, 0.5, 50f * seed, 9800 + seed));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
        for (int dist : new int[] {3, 9}) {
            Scenario s = new Scenario("burst_blaze_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:blaze", 0.5, BY, 0.5, 0f, 9900 + dist));
            s.player = new double[] {0.5 + dist, BY, 2.5};
            s.dayTime = 18000;
            s.ticks = 240;
            out.add(s);
        }
        {
            Scenario s = new Scenario("water_blaze");
            floor(s, 16, "minecraft:stone");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++) {
                    block(s, x, BY - 1, z, "minecraft:water");
                    block(s, x, BY - 2, z, "minecraft:stone");
                }
            s.mobs.add(new MobSpec("minecraft:blaze", 0.5, BY - 1, 0.5, 0f, 9950));
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 100;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_phantom_" + seed);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:phantom", 0.5, BY + 10, 0.5, 60f * seed, 10000 + seed));
            s.player = new double[] {18.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
        for (String nbt : new String[] {null, "{size:3}"}) {
            Scenario s = new Scenario(nbt == null ? "attack_phantom" : "attack_phantom_big");
            floor(s, 24, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:phantom", 0.5, BY + 12, 0.5, 0f, nbt == null ? 10100 : 10101);
            m.nbt = nbt;
            s.mobs.add(m);
            s.player = new double[] {4.5, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("daylight_phantom");
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:phantom", 0.5, BY + 8, 0.5, 0f, 10200));
            s.player = new double[] {14.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 6000;
            s.ticks = 300;
            out.add(s);
        }
    }

    // ---------------------------------------------------------- m6s2: villagers, piglins, hoglins
    static void scenariosVillagers(List<Scenario> out) {
        // Without AI the brain does not run: health, hurt and the ambient sound clock compare.
        for (boolean baby : new boolean[] {false, true}) {
            Scenario s = new Scenario("villager_noai_" + (baby ? "baby" : "adult"));
            floor(s, 8, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:villager", 0.5, BY, 0.5, 30f, 9100);
            m.nbt = "{NoAI:1b,VillagerData:{type:\"minecraft:desert\",profession:\"minecraft:farmer\",level:2}}";
            if (baby) m.age = -24000;
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 3.0});
            s.ticks = 120;
            out.add(s);
        }
        // With AI: the brain against Kiln's goals.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("villager_idle_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:villager", 0.5, BY, 0.5, 40f * seed, 9200L + seed));
            s.player = new double[] {4.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 200;
            s.diverges = true;
            out.add(s);
        }
        // Piglins and hoglins without AI: attributes, size, health and hurt.
        String[][] noai = {
            {"piglin_noai_adult", "minecraft:piglin", "{NoAI:1b}"},
            {"piglin_noai_baby", "minecraft:piglin", "{NoAI:1b,IsBaby:1b}"},
            {"hoglin_noai_adult", "minecraft:hoglin", "{NoAI:1b}"},
            {"hoglin_noai_baby", "minecraft:hoglin", "{NoAI:1b}"},
        };
        for (String[] n : noai) {
            Scenario s = new Scenario(n[0]);
            floor(s, 8, "minecraft:stone");
            MobSpec m = new MobSpec(n[1], 0.5, BY, 0.5, 60f, 9300);
            m.nbt = n[2];
            if (n[0].equals("hoglin_noai_baby")) m.age = -24000;
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 4.0});
            s.hurts.put(40, new double[] {0, 2.5});
            s.ticks = 120;
            out.add(s);
        }
        // With AI: the brains against Kiln's goals.
        {
            Scenario s = new Scenario("piglin_idle");
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:piglin", 0.5, BY, 0.5, 20f, 9401);
            m.nbt = "{IsImmuneToZombification:1b}";
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            s.diverges = true;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hoglin_chase");
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:hoglin", 0.5, BY, 0.5, 0f, 9402);
            m.nbt = "{IsImmuneToZombification:1b}";
            s.mobs.add(m);
            s.player = new double[] {5.5, BY, 0.5};
            s.ticks = 160;
            s.diverges = true;
            out.add(s);
        }
    }
}
