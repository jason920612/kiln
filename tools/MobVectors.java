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
        /// Effects added (`addEffect`) once the mob is in the level: {effect, duration, amplifier}.
        final List<Object[]> effects = new ArrayList<>();
        MobSpec(String type, double x, double y, double z, float yaw, long seed) {
            this.type = type; this.x = x; this.y = y; this.z = z; this.yaw = yaw; this.seed = seed;
        }
    }

    /// kind: "effect" (mob, effect, duration, amp), "splash" (potion at pos: a splash potion
    /// breaking there), "linger" (potion at pos: a lingering potion's cloud), "interact" (mob,
    /// item: the player uses the item on the mob).
    static final class Action {
        int tick;
        String kind;
        int mob;
        String what;
        int duration, amp;
        double x, y, z;
        Action(int tick, String kind) { this.tick = tick; this.kind = kind; }
        String json() {
            return String.format(Locale.ROOT, "{\"tick\":%d,\"kind\":\"%s\",\"mob\":%d,\"what\":\"%s\",\"duration\":%d,\"amp\":%d,\"pos\":[%s,%s,%s]}",
                    tick, kind, mob, what, duration, amp, d(x), d(y), d(z));
        }
    }

    static net.minecraft.core.Holder<net.minecraft.world.effect.MobEffect> effect(String name) {
        return BuiltInRegistries.MOB_EFFECT.get(Identifier.parse(name)).orElseThrow();
    }

    static ItemStack potionItem(String item, String potion) {
        ItemStack stack = new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(item)));
        stack.set(net.minecraft.core.component.DataComponents.POTION_CONTENTS,
                new net.minecraft.world.item.alchemy.PotionContents(BuiltInRegistries.POTION.get(Identifier.parse(potion)).orElseThrow()));
        return stack;
    }

    static void act(ServerLevel level, ServerPlayer player, List<Entity> tracked, Action a) {
        switch (a.kind) {
            case "effect" -> ((LivingEntity) tracked.get(a.mob)).addEffect(new net.minecraft.world.effect.MobEffectInstance(effect(a.what), a.duration, a.amp));
            case "splash" -> {
                ItemStack stack = potionItem("minecraft:splash_potion", a.what);
                var potion = new net.minecraft.world.entity.projectile.throwableitemprojectile.ThrownSplashPotion(level, a.x, a.y, a.z, stack);
                Vec3 at = new Vec3(a.x, a.y, a.z);
                potion.onHitAsPotion(level, stack, new net.minecraft.world.phys.BlockHitResult(at, net.minecraft.core.Direction.UP, BlockPos.containing(at), false));
            }
            case "linger" -> {
                ItemStack stack = potionItem("minecraft:lingering_potion", a.what);
                var potion = new net.minecraft.world.entity.projectile.throwableitemprojectile.ThrownLingeringPotion(level, a.x, a.y, a.z, stack);
                Vec3 at = new Vec3(a.x, a.y, a.z);
                potion.onHitAsPotion(level, stack, new net.minecraft.world.phys.BlockHitResult(at, net.minecraft.core.Direction.UP, BlockPos.containing(at), false));
            }
            case "interact" -> {
                player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(a.what))));
                player.interactOn(tracked.get(a.mob), net.minecraft.world.InteractionHand.MAIN_HAND, tracked.get(a.mob).position());
            }
            default -> throw new IllegalArgumentException(a.kind);
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
        /// The player's look direction (yaw also turns its head) and head item.
        float playerYaw, playerPitch;
        String playerHead;
        long dayTime = 1000;
        long levelSeed = 1;
        int ticks = 200;
        /// Brain-driven mobs Kiln approximates with goals: the replay reports where Kiln
        /// diverges instead of failing.
        boolean diverges;
        // tick -> [mob index, amount]; the player (or nobody) hurts the mob.
        final Map<Integer, double[]> hurts = new HashMap<>();
        /// Things done to the mobs before the entity ticks of a tick (effects, potions,
        /// player interactions), in order.
        final List<Action> actions = new ArrayList<>();
        /// Entities that are not mobs (end crystals): ticked after the mobs, not traced.
        final List<MobSpec> others = new ArrayList<>();
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
        for (Scenario s : scenarios()) if (filter == null || java.util.Arrays.stream(filter.split("\\|")).anyMatch(s.name::contains)) selected.add(s);
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
            if (filter == null || "raid_waves".contains(filter)) {
                try {
                    lines.addAll(raidWaves());
                } catch (Throwable t) {
                    t.printStackTrace();
                }
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
            player.snapTo(s.player[0], s.player[1], s.player[2], s.playerYaw, s.playerPitch);
            player.setYHeadRot(s.playerYaw);
            player.setItemSlot(EquipmentSlot.HEAD, s.playerHead == null ? ItemStack.EMPTY
                    : new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(s.playerHead))));
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
        int tickStamp = player.getLastHurtByMobTimestamp();
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
            for (Object[] fx : spec.effects) {
                m.addEffect(new net.minecraft.world.effect.MobEffectInstance(effect((String) fx[0]), (Integer) fx[1], (Integer) fx[2]));
            }
            tracked.add(m);
            if (specs.length() > 0) specs.append(',');
            specs.append(String.format(Locale.ROOT,
                    "{\"type\":\"%s\",\"id\":%d,\"seed\":%d,\"pos\":[%s,%s,%s],\"yaw\":%s,\"main_hand\":%s,\"egg_time\":%d,\"age\":%d,\"in_love\":%d,\"nbt\":%s,\"effects\":%s}",
                    spec.type, m.getId(), spec.seed, d(spec.x), d(spec.y), d(spec.z), Float.toString(spec.yaw),
                    spec.mainHand == null ? "null" : "\"" + spec.mainHand + "\"", eggTime,
                    spec.age == null ? 0 : spec.age, spec.inLove == null ? 0 : spec.inLove, nbtJson, effectsJson(spec.effects)));
        }
        StringBuilder trace = new StringBuilder();
        StringBuilder hits = new StringBuilder();
        StringBuilder spawned = new StringBuilder();
        // Mobs that appear during the scenario (babies, split slimes, converted zombies) get a
        // pinned random, their head and body turned to their yaw (the constructor's random yaw
        // is not reproducible) and join the trace.
        List<Mob> pinned = new ArrayList<>();
        int initial = tracked.size();
        StringBuilder others = new StringBuilder();
        for (MobSpec spec : s.others) {
            Entity o = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(spec.type)).create(level, EntitySpawnReason.COMMAND);
            o.snapTo(spec.x, spec.y, spec.z, spec.yaw, 0f);
            if (!level.addFreshEntity(o)) throw new IllegalStateException("could not add " + spec.type);
            tracked.add(o);
            if (others.length() > 0) others.append(',');
            others.append(String.format(Locale.ROOT, "{\"type\":\"%s\",\"id\":%d,\"pos\":[%s,%s,%s],\"yaw\":%s}",
                    spec.type, o.getId(), d(spec.x), d(spec.y), d(spec.z), Float.toString(spec.yaw)));
        }
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
            for (Action a : s.actions) {
                if (a.tick == tick) act(level, player, tracked, a);
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
        // Flyers may end outside the cleanup box.
        for (Entity e : tracked) e.discard();
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
                : String.format(Locale.ROOT, "{\"id\":%d,\"pos\":[%s,%s,%s],\"sneaking\":%b,\"creative\":%b,\"main_hand\":%s,\"yaw\":%s,\"pitch\":%s,\"head\":%s,\"uuid\":%s,\"tick_count\":%d,\"last_hurt_by_mob_time\":%d}", player.getId(), d(s.player[0]), d(s.player[1]), d(s.player[2]), s.playerSneaking, s.playerCreative,
                        s.playerMainHand == null ? "null" : "\"" + s.playerMainHand + "\"", Float.toString(s.playerYaw), Float.toString(s.playerPitch),
                        s.playerHead == null ? "null" : "\"" + s.playerHead + "\"", java.util.Arrays.toString(net.minecraft.core.UUIDUtil.uuidToIntArray(player.getUUID())), player.tickCount, tickStamp);
        return String.format(Locale.ROOT,
                "{\"name\":\"%s\",\"diverges\":%b,\"level_seed\":%d,\"ticks\":%d,\"game_time\":%d,\"sky_darken\":%d,\"actions\":%s,\"blocks\":[%s],\"mobs\":[%s],"
                        + "\"player\":%s,\"hurts\":[%s],\"hits\":[%s],\"spawned\":[%s],\"others\":[%s],\"trace\":[%s]}",
                s.name, s.diverges, s.levelSeed, s.ticks, startTime, skyDarken, actionsJson(s.actions), blocks, specs, playerJson, hurts, hits, spawned, others, trace);
    }

    static String effectsJson(List<Object[]> effects) {
        StringBuilder sb = new StringBuilder("[");
        for (Object[] fx : effects) {
            if (sb.length() > 1) sb.append(',');
            sb.append(String.format(Locale.ROOT, "[\"%s\",%d,%d]", fx[0], fx[1], fx[2]));
        }
        return sb.append(']').toString();
    }

    static String actionsJson(List<Action> actions) {
        StringBuilder sb = new StringBuilder("[");
        for (Action a : actions) {
            if (sb.length() > 1) sb.append(',');
            sb.append(a.json());
        }
        return sb.append(']').toString();
    }

    /// The active effects as one number: the sum of (id + 1) * 100000 + duration * 10 + amplifier
    /// (hidden effects not included).
    static long effectsSig(LivingEntity m) {
        long sig = 0;
        for (var fx : m.getActiveEffects()) {
            sig += (BuiltInRegistries.MOB_EFFECT.getId(fx.getEffect().value()) + 1) * 100000L + fx.getDuration() * 10L + fx.getAmplifier();
        }
        return sig;
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
        pinCommonB(m);
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
                .append(',').append(((java.util.concurrent.atomic.AtomicLong) get(m.getRandom(), "seed")).get())
                .append(',').append(effectsSig(m)).append(',').append(Float.toString(m.getAbsorptionAmount()));
        StringBuilder goals = new StringBuilder();
        for (var sel : new net.minecraft.world.entity.ai.goal.GoalSelector[] {(net.minecraft.world.entity.ai.goal.GoalSelector) get(m, "goalSelector"), (net.minecraft.world.entity.ai.goal.GoalSelector) get(m, "targetSelector")}) {
            for (WrappedGoal g : sel.getAvailableGoals()) {
                if (!g.isRunning()) continue;
                if (goals.length() > 0) goals.append(' ');
                goals.append(g.getGoal().getClass().getSimpleName());
            }
        }
        if (m instanceof net.minecraft.world.entity.boss.enderdragon.EnderDragon dragon) {
            if (goals.length() > 0) goals.append(' ');
            goals.append("DragonPhase").append(dragon.getPhaseManager().getCurrentPhase().getPhase().getId());
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
        scenariosZombies(out);
        scenariosEnder(out);
        scenariosTame(out);
        // slice 3: one call per work package (keep the blank lines between them).
        scenariosEffects(out);

        scenariosRaids(out);

        scenariosEnd(out);

        scenariosWither(out);

        scenariosWarden(out);

        scenariosCommonA(out);

        scenariosCommonB(out);

        return out;
    }

    /// `Owner` of the harness player (`KilnMob`), for tamed animals.
    static String owner() {
        int[] u = net.minecraft.core.UUIDUtil.uuidToIntArray(UUID.nameUUIDFromBytes("KilnMob".getBytes()));
        return String.format(Locale.ROOT, "Owner:[I;%d,%d,%d,%d]", u[0], u[1], u[2], u[3]);
    }

    // ---------------------------------------------------------- slice 2: tameables, riding, golems
    static void scenariosTame(List<Scenario> out) {
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_wolf_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 30f * seed, 9000L * seed + 3));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        for (double dist : new double[] {11, 14}) {
            Scenario s = new Scenario("follow_owner_wolf_" + (int) dist);
            floor(s, 20, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 0f, 9100 + (long) dist);
            m.nbt = "{" + owner() + "}";
            s.mobs.add(m);
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("sit_wolf");
            floor(s, 20, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 45f, 9200);
            m.nbt = "{" + owner() + ",Sitting:1b}";
            s.mobs.add(m);
            s.player = new double[] {11.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("beg_wolf");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 90f, 9300));
            s.player = new double[] {5.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = "minecraft:bone";
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hunt_wolf");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 0f, 9400));
            s.mobs.add(new MobSpec("minecraft:sheep", 5.5, BY, 2.5, 0f, 9401));
            s.player = new double[] {14.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("anger_wolf");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:wolf", 0.5, BY, 0.5, 0f, 9500));
            s.mobs.add(new MobSpec("minecraft:wolf", -2.5, BY, 1.5, 0f, 9501));
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 300;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_cat_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:cat", 0.5, BY, 0.5, 60f * seed, 9600L * seed + 1));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("avoid_cat");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:cat", 0.5, BY, 0.5, 0f, 9700));
            s.player = new double[] {5.5, BY, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("tempt_cat");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:cat", 0.5, BY, 0.5, 0f, 9710));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerMainHand = "minecraft:cod";
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("follow_owner_cat");
            floor(s, 20, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:cat", 0.5, BY, 0.5, 0f, 9720);
            m.nbt = "{" + owner() + "}";
            s.mobs.add(m);
            s.player = new double[] {11.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("block_cat");
            floor(s, 20, "minecraft:grass_block");
            block(s, 3, BY, 2, "minecraft:chest[facing=north]");
            block(s, -3, BY, 1, "minecraft:red_bed[facing=east,part=foot]");
            block(s, -2, BY, 1, "minecraft:red_bed[facing=east,part=head]");
            MobSpec m = new MobSpec("minecraft:cat", 0.5, BY, 0.5, 0f, 9730);
            m.nbt = "{" + owner() + "}";
            s.mobs.add(m);
            MobSpec m2 = new MobSpec("minecraft:cat", 1.5, BY, -1.5, 90f, 9731);
            m2.nbt = "{" + owner() + "}";
            s.mobs.add(m2);
            s.player = new double[] {4.5, BY, -3.5};
            s.playerCreative = true;
            s.ticks = 600;
            out.add(s);
        }
        String[] equines = {"horse", "horse", "donkey", "mule"};
        for (int i = 0; i < equines.length; i++) {
            Scenario s = new Scenario("idle_" + equines[i] + "_" + i);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:" + equines[i], 0.5, BY, 0.5, 40f * i, 9800L + 13 * i));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = i + 1;
            s.ticks = 600;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_horse");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:horse", 0.5, BY, 0.5, 0f, 9850));
            s.player = new double[] {3.5, BY, 0.5};
            s.playerSneaking = true;
            s.hurts.put(5, new double[] {0, 1.0});
            s.hurts.put(40, new double[] {0, 1.0});
            s.hurts.put(80, new double[] {0, 1.0});
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("tempt_horse");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:horse", 0.5, BY, 0.5, 0f, 9860);
            m.nbt = "{Tame:1b,Temper:40}";
            s.mobs.add(m);
            s.player = new double[] {7.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = "minecraft:golden_carrot";
            s.ticks = 300;
            out.add(s);
        }
        for (int i = 0; i < 2; i++) {
            Scenario s = new Scenario("idle_iron_golem_" + i);
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:iron_golem", 0.5, BY, 0.5, 70f * i, 9900L + i));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = 5 + i;
            s.dayTime = i == 0 ? 1000 : 18000;
            s.ticks = 600;
            out.add(s);
        }
        {
            Scenario s = new Scenario("anger_iron_golem");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:iron_golem", 0.5, BY, 0.5, 0f, 9950));
            s.player = new double[] {5.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 200;
            out.add(s);
        }
        for (int i = 0; i < 2; i++) {
            Scenario s = new Scenario("lava_strider_" + i);
            floor(s, 16, "minecraft:stone");
            if (i == 0) {
                for (int x = 3; x <= 7; x++)
                    for (int z = -2; z <= 2; z++) block(s, x, BY - 1, z, "minecraft:lava");
            }
            s.mobs.add(new MobSpec("minecraft:strider", 0.5, BY, 0.5, 20f * i, 9970L + i));
            s.player = new double[] {-8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = 3 + i;
            s.ticks = 500;
            out.add(s);
        }
    }

    // ---------------------------------------------------------- slice M6s2: enderman, endermite, shulker, witch
    static void scenariosEnder(List<Scenario> out) {
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_enderman_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 45f * seed, 3000L * seed + 13));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        // Grass and flowers to pick up; a carried block to put down.
        {
            Scenario s = new Scenario("blocks_enderman");
            floor(s, 16, "minecraft:grass_block");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++) {
                    if ((x + z) % 2 == 0 && (x != 0 || z != 0)) block(s, x, BY, z, "minecraft:dirt");
                    else if (x != 0 || z != 0) block(s, x, BY, z, "minecraft:poppy");
                }
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 0f, 3401));
            MobSpec carrier = new MobSpec("minecraft:enderman", 8.5, BY, 8.5, 90f, 3402);
            carrier.nbt = "{carriedBlockState:\"minecraft:sand\"}";
            s.mobs.add(carrier);
            s.player = new double[] {-10.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 600;
            out.add(s);
        }
        // Hurt by the player: it fights back.
        for (int dist : new int[] {4, 9, 18}) {
            Scenario s = new Scenario("chase_enderman_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 0f, 5250 + dist));
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.hurts.put(3, new double[] {0, 1.0});
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Stared at: aggro after the delay, freezes while looked at, teleports when close.
        for (String head : new String[] {null, "minecraft:carved_pumpkin"}) {
            Scenario s = new Scenario(head == null ? "stare_enderman" : "stare_enderman_pumpkin");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 270f, 5301));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerYaw = 90f;
            s.playerPitch = -6.6f;
            s.playerHead = head;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
        // Stared at from close by: it teleports away.
        {
            Scenario s = new Scenario("stare_close_enderman");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 270f, 5311));
            s.player = new double[] {3.5, BY, 0.5};
            s.playerYaw = 90f;
            s.playerPitch = -17f;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
        // Daylight: it teleports away once 600 ticks passed since its target changed.
        {
            Scenario s = new Scenario("daylight_enderman");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 0f, 5401));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 6000;
            s.ticks = 800;
            out.add(s);
        }
        // Endermites: idle, chasing, and the end of a life.
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_endermite_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:endermite", 0.5, BY, 0.5, 60f * seed, 3100L * seed + 17));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        for (int dist : new int[] {4, 9}) {
            Scenario s = new Scenario("chase_endermite_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:endermite", 0.5, BY, 0.5, 0f, 5350 + dist));
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 160;
            out.add(s);
        }
        {
            Scenario s = new Scenario("lifetime_endermite");
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:endermite", 0.5, BY, 0.5, 0f, 3501);
            m.nbt = "{Lifetime:2350}";
            s.mobs.add(m);
            MobSpec keep = new MobSpec("minecraft:endermite", 3.5, BY, 0.5, 0f, 3502);
            keep.nbt = "{Lifetime:2350,PersistenceRequired:1b}";
            s.mobs.add(keep);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 100;
            out.add(s);
        }
        // Shulkers: peeking, shooting bullets, teleporting when hurt, on a wall.
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_shulker_" + seed);
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:shulker", 0.5, BY, 0.5, 0f, 3200L * seed + 19);
            if (seed == 2) m.nbt = "{Color:5b}";
            s.mobs.add(m);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        for (int dist : new int[] {5, 12}) {
            Scenario s = new Scenario("attack_shulker_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:shulker", 0.5, BY, 0.5, 0f, 5450 + dist));
            s.player = new double[] {0.5 + dist, BY, 2.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_shulker");
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:shulker", 0.5, BY, 0.5, 0f, 3601));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.hurts.put(5, new double[] {0, 16.0});
            s.hurts.put(30, new double[] {0, 2.0});
            s.hurts.put(55, new double[] {0, 2.0});
            s.hurts.put(80, new double[] {0, 2.0});
            s.hurts.put(105, new double[] {0, 2.0});
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("wall_shulker");
            floor(s, 16, "minecraft:stone");
            for (int y = BY; y <= BY + 2; y++)
                for (int z = -2; z <= 2; z++) block(s, -1, y, z, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:shulker", 0.5, BY + 1, 0.5, 0f, 3701);
            m.nbt = "{AttachFace:4b}";
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 3.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Witches: idle, throwing potions, drinking swiftness far from the target and healing
        // when hurt.
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_witch_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:witch", 0.5, BY, 0.5, 70f * seed, 3300L * seed + 23));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        for (int dist : new int[] {6, 9, 14}) {
            Scenario s = new Scenario("chase_witch_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:witch", 0.5, BY, 0.5, 0f, 5550 + dist));
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_witch");
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:witch", 0.5, BY, 0.5, 0f, 3801));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.hurts.put(5, new double[] {0, 6.0});
            s.ticks = 300;
            out.add(s);
        }
        // Water hurts it and makes it teleport.
        {
            Scenario s = new Scenario("water_enderman");
            floor(s, 20, "minecraft:stone");
            for (int x = -2; x <= 2; x++)
                for (int z = -2; z <= 2; z++) block(s, x, BY, z, "minecraft:water");
            s.mobs.add(new MobSpec("minecraft:enderman", 0.5, BY, 0.5, 0f, 5501));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
    }

    // ---------------------------------------------------------- slice 2: the zombie and skeleton families
    static void scenariosZombies(List<Scenario> out) {
        String[][] types = {{"husk", null}, {"zombie_villager", null}, {"zombified_piglin", null}, {"stray", "minecraft:bow"}, {"wither_skeleton", "minecraft:stone_sword"}};
        for (String[] t : types) {
            for (int seed = 1; seed <= 2; seed++) {
                Scenario s = new Scenario("idle_" + t[0] + "_" + seed);
                floor(s, 16, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + t[0], 0.5, BY, 0.5, 45f * seed, 3000L * seed + 13);
                m.mainHand = t[1];
                s.mobs.add(m);
                s.player = new double[] {8.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                // Husks do not burn: one of them idles in daylight.
                s.dayTime = t[0].equals("husk") && seed == 2 ? 6000 : 18000;
                s.ticks = 400;
                out.add(s);
            }
            if (t[0].equals("zombified_piglin")) continue;
            for (int dist : new int[] {4, 9}) {
                Scenario s = new Scenario("chase_" + t[0] + "_" + dist);
                floor(s, 20, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + t[0], 0.5, BY, 0.5, 0f, 5250 + dist);
                m.mainHand = t[1];
                s.mobs.add(m);
                s.player = new double[] {0.5 + dist, BY, 0.5};
                s.dayTime = 18000;
                s.ticks = 160;
                out.add(s);
            }
        }
        // Zombified piglins: hurt one, the group gets angry at the player.
        {
            Scenario s = new Scenario("anger_zombified_piglin");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:zombified_piglin", 0.5, BY, 0.5, 0f, 7100));
            s.mobs.add(new MobSpec("minecraft:zombified_piglin", -2.5, BY, 3.5, 90f, 7101));
            s.mobs.add(new MobSpec("minecraft:zombified_piglin", -12.5, BY, -6.5, 180f, 7102));
            s.player = new double[] {5.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 1.0});
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Drowned: idle and chasing on land at night, heading for water by day, swimming after a
        // target in the water, throwing tridents.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_drowned_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 45f * seed, 3100L * seed + 17));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        for (int dist : new int[] {4, 9}) {
            Scenario s = new Scenario("chase_drowned_" + dist);
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 0f, 5350 + dist));
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 160;
            out.add(s);
        }
        {
            Scenario s = new Scenario("trident_drowned_9");
            floor(s, 20, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 0f, 5400);
            m.mainHand = "minecraft:trident";
            s.mobs.add(m);
            s.player = new double[] {9.5, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("to_water_drowned");
            floor(s, 16, "minecraft:stone");
            for (int x = 4; x <= 7; x++)
                for (int z = -2; z <= 2; z++) block(s, x, BY, z, "minecraft:water");
            s.mobs.add(new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 0f, 5500));
            s.player = new double[] {-8.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 6000;
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("swim_drowned");
            floor(s, 16, "minecraft:stone");
            for (int x = -4; x <= 6; x++)
                for (int z = -4; z <= 4; z++)
                    for (int y = BY; y <= BY + 4; y++) block(s, x, y, z, "minecraft:water");
            s.mobs.add(new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 0f, 5600));
            s.player = new double[] {4.5, BY + 3, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("beach_drowned");
            floor(s, 16, "minecraft:stone");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++)
                    for (int y = BY; y <= BY + 1; y++) block(s, x, y, z, "minecraft:water");
            s.mobs.add(new MobSpec("minecraft:drowned", 0.5, BY, 0.5, 0f, 5700));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 300;
            out.add(s);
        }
        // A husk under water turns into a zombie, a zombie into a drowned (conversions shortened
        // through the saved data).
        for (String type : new String[] {"husk", "zombie"}) {
            Scenario s = new Scenario("convert_" + type);
            floor(s, 16, "minecraft:stone");
            for (int x = -2; x <= 2; x++)
                for (int z = -2; z <= 2; z++)
                    for (int y = BY; y <= BY + 2; y++) block(s, x, y, z, "minecraft:water");
            MobSpec m = new MobSpec("minecraft:" + type, 0.5, BY, 0.5, 0f, 7200);
            m.nbt = "{DrownedConversionTime:30}";
            s.mobs.add(m);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 120;
            out.add(s);
        }
        // A skeleton in powder snow freezes into a stray (7 s, then 15 s of shaking).
        {
            Scenario s = new Scenario("convert_skeleton");
            floor(s, 16, "minecraft:stone");
            for (int x = -3; x <= 3; x++)
                for (int z = -3; z <= 3; z++)
                    for (int y = BY; y <= BY + 1; y++) block(s, x, y, z, "minecraft:powder_snow");
            s.mobs.add(new MobSpec("minecraft:skeleton", 0.5, BY, 0.5, 0f, 7250));
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 480;
            out.add(s);
        }
        // Zombies go after villagers, skeletons after iron golems (standing still: no AI).
        {
            Scenario s = new Scenario("target_villager_zombie");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:zombie", 0.5, BY, 0.5, 0f, 7300));
            MobSpec v = new MobSpec("minecraft:villager", 6.5, BY, 0.5, 90f, 7301);
            v.nbt = "{NoAI:1b}";
            s.mobs.add(v);
            s.player = new double[] {-12.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 160;
            out.add(s);
        }
        {
            Scenario s = new Scenario("target_golem_skeleton");
            floor(s, 20, "minecraft:stone");
            MobSpec k = new MobSpec("minecraft:skeleton", 0.5, BY, 0.5, 0f, 7400);
            s.mobs.add(k);
            // Far enough that the golem (whose own behaviour is not simulated yet) is not reached.
            MobSpec g = new MobSpec("minecraft:iron_golem", 12.5, BY, 0.5, 90f, 7401);
            g.nbt = "{NoAI:1b}";
            s.mobs.add(g);
            s.player = new double[] {-12.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 70;
            out.add(s);
        }
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

    // ---------------------------------------------------------- slice 3: mob effects on mobs, curing
    static void scenariosEffects(List<Scenario> out) {
        // A mob with one effect near a standing player: movement, health and the effect run down.
        String[][] single = {
            {"zombie", "minecraft:speed", "300", "1"},
            {"zombie", "minecraft:slowness", "300", "1"},
            {"zombie", "minecraft:jump_boost", "300", "2"},
            {"zombie", "minecraft:strength", "300", "0"},
            {"zombie", "minecraft:weakness", "300", "0"},
            {"zombie", "minecraft:poison", "300", "0"},
            {"zombie", "minecraft:regeneration", "300", "1"},
            {"zombie", "minecraft:instant_health", "1", "0"},
            {"zombie", "minecraft:instant_damage", "1", "0"},
            {"zombie", "minecraft:levitation", "80", "0"},
            {"pig", "minecraft:poison", "200", "1"},
            {"pig", "minecraft:wither", "200", "1"},
            {"pig", "minecraft:regeneration", "200", "0"},
            {"pig", "minecraft:instant_damage", "1", "0"},
            {"pig", "minecraft:instant_health", "1", "0"},
            {"pig", "minecraft:levitation", "60", "1"},
            {"pig", "minecraft:absorption", "200", "1"},
            {"pig", "minecraft:health_boost", "200", "1"},
            {"pig", "minecraft:speed", "200", "3"},
            {"spider", "minecraft:poison", "200", "0"},
            {"skeleton", "minecraft:wither", "200", "0"},
            {"cow", "minecraft:slow_falling", "200", "0"},
        };
        for (String[] c : single) {
            Scenario s = new Scenario("effect_" + c[0] + "_" + c[1].substring(10));
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:" + c[0], 0.5, BY, 0.5, 0f, 7100 + out.size());
            m.effects.add(new Object[] {c[1], Integer.parseInt(c[2]), Integer.parseInt(c[3])});
            s.mobs.add(m);
            s.player = new double[] {7.5, BY, 0.5};
            s.ticks = 160;
            if (c[0].equals("pig") || c[0].equals("cow")) s.hurts.put(0, new double[] {0, 3});
            out.add(s);
        }
        {
            // Falling with slow falling from a height.
            Scenario s = new Scenario("effect_chicken_slow_falling_drop");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:chicken", 0.5, BY + 12, 0.5, 0f, 7201);
            m.effects.add(new Object[] {"minecraft:slow_falling", 400, 0});
            s.mobs.add(m);
            s.player = new double[] {7.5, BY, 0.5};
            s.ticks = 200;
            out.add(s);
        }
        {
            // A fire resistant zombie in daylight: it burns but takes no damage.
            Scenario s = new Scenario("effect_zombie_fire_resistance_daylight");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:zombie", 0.5, BY, 0.5, 0f, 7202);
            m.effects.add(new Object[] {"minecraft:fire_resistance", 400, 0});
            s.mobs.add(m);
            s.dayTime = 6000;
            s.ticks = 200;
            out.add(s);
        }
        {
            // Resistance against the player's hits.
            Scenario s = new Scenario("effect_pig_resistance_hits");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 7203);
            m.effects.add(new Object[] {"minecraft:resistance", 400, 1});
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(2, new double[] {0, 4});
            s.hurts.put(40, new double[] {0, 5});
            s.ticks = 100;
            out.add(s);
        }
        {
            // A stronger, shorter effect over a weaker, longer one: the weaker comes back.
            Scenario s = new Scenario("effect_pig_hidden_speed");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 7204);
            m.effects.add(new Object[] {"minecraft:speed", 200, 0});
            s.mobs.add(m);
            s.player = new double[] {7.5, BY, 0.5};
            Action a = new Action(10, "effect");
            a.mob = 0; a.what = "minecraft:speed"; a.duration = 40; a.amp = 2;
            s.actions.add(a);
            s.hurts.put(0, new double[] {0, 1});
            s.ticks = 120;
            out.add(s);
        }
        // Splash potions on a row of mobs at different distances (the undead invert healing
        // and harming).
        String[][] splash = {
            {"minecraft:poison", "pig"}, {"minecraft:harming", "pig"}, {"minecraft:harming", "zombie"},
            {"minecraft:healing", "zombie"}, {"minecraft:strong_healing", "skeleton"}, {"minecraft:slowness", "cow"},
            {"minecraft:long_swiftness", "zombie"}, {"minecraft:weakness", "zombie"},
        };
        for (String[] c : splash) {
            Scenario s = new Scenario("splash_" + c[0].substring(10) + "_" + c[1]);
            floor(s, 16, "minecraft:grass_block");
            for (int i = 0; i < 4; i++) {
                s.mobs.add(new MobSpec("minecraft:" + c[1], 0.5 + i * 1.3, BY, 0.5 + (i % 2) * 0.7, 90f * i, 7300 + i + out.size() * 4));
            }
            s.player = new double[] {12.5, BY, 8.5};
            s.playerCreative = true;
            // Night: no sun to flee from (the recording's light below the floor can be stale).
            s.dayTime = 18000;
            s.hurts.put(0, new double[] {1, 4});
            Action a = new Action(3, "splash");
            a.what = c[0]; a.x = 1.1; a.y = BY + 0.2; a.z = 0.6;
            s.actions.add(a);
            s.ticks = 100;
            out.add(s);
        }
        {
            // A lingering potion's cloud over pigs and a zombie: effects every five ticks at
            // most, the cloud shrinking with each use.
            for (String potion : new String[] {"minecraft:poison", "minecraft:harming", "minecraft:regeneration"}) {
                Scenario s = new Scenario("linger_" + potion.substring(10));
                floor(s, 16, "minecraft:grass_block");
                s.mobs.add(new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 7400));
                s.mobs.add(new MobSpec("minecraft:pig", 2.0, BY, 0.9, 90f, 7401));
                s.mobs.add(new MobSpec("minecraft:zombie", -1.2, BY, -0.4, 180f, 7402));
                s.player = new double[] {12.5, BY, 8.5};
                s.playerCreative = true;
                s.hurts.put(0, new double[] {0, 5});
                s.hurts.put(1, new double[] {1, 5});
                Action a = new Action(2, "linger");
                a.what = potion; a.x = 0.5; a.y = BY; a.z = 0.5;
                s.actions.add(a);
                s.ticks = 160;
                out.add(s);
            }
        }
        {
            // A golden apple on a weakened zombie villager starts the cure: the weakness goes,
            // strength comes; a golden apple without weakness does nothing.
            for (boolean weak : new boolean[] {true, false}) {
                Scenario s = new Scenario("cure_zombie_villager_" + (weak ? "weak" : "plain"));
                floor(s, 16, "minecraft:grass_block");
                MobSpec m = new MobSpec("minecraft:zombie_villager", 0.5, BY, 0.5, 0f, 7500);
                m.nbt = "{VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:2},Xp:5}";
                if (weak) m.effects.add(new Object[] {"minecraft:weakness", 600, 0});
                s.mobs.add(m);
                s.player = new double[] {1.5, BY, 0.5};
                s.playerCreative = true;
                Action a = new Action(5, "interact");
                a.mob = 0; a.what = "minecraft:golden_apple";
                s.actions.add(a);
                s.ticks = 60;
                out.add(s);
            }
        }
        // Silverfish: idle ones merge into the stone floor; a hurt one wakes the infested blocks
        // around it; infested mobs let silverfish out when hit.
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("silverfish_idle_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:silverfish", 0.5, BY, 0.5, 30f * seed, 7600 + seed));
            s.player = new double[] {9.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            out.add(s);
        }
        {
            Scenario s = new Scenario("silverfish_wakes_friends");
            floor(s, 16, "minecraft:grass_block");
            for (int x = -3; x <= 3; x++)
                for (int z = 2; z <= 3; z++) block(s, BX + x, BY, BZ + z, "minecraft:infested_stone");
            s.mobs.add(new MobSpec("minecraft:silverfish", 0.5, BY, 0.5, 0f, 7610));
            s.player = new double[] {0.5, BY, -3.5};
            s.hurts.put(2, new double[] {0, 1});
            s.ticks = 60;
            out.add(s);
        }
        for (int k = 0; k < 4; k++) {
            Scenario s = new Scenario("effect_pig_infested_hits_" + k);
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:pig", 0.5, BY, 0.5, 0f, 7630 + k * 17);
            m.effects.add(new Object[] {"minecraft:infested", 600, 0});
            m.nbt = "{Health:10f,attributes:[{id:\"minecraft:max_health\",base:40.0d}]}";
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.playerCreative = true;
            for (int t = 2; t < 240; t += 11) s.hurts.put(t, new double[] {0, 0.25});
            s.ticks = 240;
            out.add(s);
        }
        {
            // The end of a cure: a villager with the zombie villager's data takes its place (the
            // villager is brain-driven: compared loosely).
            Scenario s = new Scenario("cure_zombie_villager_finish");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:zombie_villager", 0.5, BY, 0.5, 0f, 7502);
            m.nbt = "{VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:2},Xp:5,ConversionTime:30}";
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 60;
            s.diverges = true;
            out.add(s);
        }
    }


    // ---------------------------------------------------------- slice 3: raids and illagers
    /// Raid wave composition: `Raid.spawnGroup`'s counts per raider type for every wave (and the
    /// bonus wave) with the raid's random pinned, by difficulty and omen level. Lines carry
    /// `raid_waves` instead of a mob trace.
    static List<String> raidWaves() throws Exception {
        List<String> out = new ArrayList<>();
        Class<?> typeClass = Class.forName("net.minecraft.world.entity.raid.Raid$RaiderType");
        Object[] types = typeClass.getEnumConstants();
        var defaults = net.minecraft.world.entity.raid.Raid.class.getDeclaredMethod("getDefaultNumSpawns", typeClass, int.class, boolean.class);
        var bonus = net.minecraft.world.entity.raid.Raid.class.getDeclaredMethod("getPotentialBonusSpawns", typeClass,
                net.minecraft.util.RandomSource.class, int.class, net.minecraft.world.DifficultyInstance.class, boolean.class);
        defaults.setAccessible(true);
        bonus.setAccessible(true);
        for (var diff : new net.minecraft.world.Difficulty[] {net.minecraft.world.Difficulty.EASY, net.minecraft.world.Difficulty.NORMAL, net.minecraft.world.Difficulty.HARD}) {
            for (int omen = 1; omen <= 5; omen += 2) {
                for (long seed = 1; seed <= 3; seed++) {
                    var raid = new net.minecraft.world.entity.raid.Raid(BlockPos.ZERO, diff);
                    raid.setRaidOmenLevel(omen);
                    var random = (net.minecraft.util.RandomSource) get(raid, "random");
                    random.setSeed(seed);
                    var inst = new net.minecraft.world.DifficultyInstance(diff, 1000L, 0L, 1.0F);
                    int groups = raid.getNumGroups(diff);
                    StringBuilder waves = new StringBuilder();
                    for (int wave = 1; wave <= groups + (omen > 1 ? 1 : 0); wave++) {
                        boolean isBonus = wave > groups;
                        if (waves.length() > 0) waves.append(',');
                        waves.append('[');
                        for (int i = 0; i < types.length; i++) {
                            int n = (Integer) defaults.invoke(raid, types[i], wave, isBonus) + (Integer) bonus.invoke(raid, types[i], random, wave, inst, isBonus);
                            waves.append(i > 0 ? "," : "").append(n);
                        }
                        waves.append(']');
                    }
                    out.add(String.format(Locale.ROOT, "{\"name\":\"raid_waves_%s_%d_%d\",\"raid_waves\":{\"difficulty\":%d,\"omen\":%d,\"seed\":%d,\"groups\":%d,\"waves\":[%s]}}",
                            diff.getSerializedName(), omen, seed, diff.getId(), omen, seed, groups, waves));
                }
            }
        }
        return out;
    }

    static void scenariosRaids(List<Scenario> out) {
        // Illagers, the ravager and the vex idle (a creative player watching) and chasing a
        // survival player at night.
        String[][] types = {
            {"pillager", "minecraft:crossbow"}, {"vindicator", "minecraft:iron_axe"}, {"evoker", null},
            {"ravager", null}, {"vex", "minecraft:iron_sword"}, {"illusioner", "minecraft:bow"}};
        for (String[] t : types) {
            for (int seed = 1; seed <= 2; seed++) {
                Scenario s = new Scenario("idle_" + t[0] + "_" + seed);
                floor(s, 16, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + t[0], 0.5, t[0].equals("vex") ? BY + 1 : BY, 0.5, 50f * seed, 4400L * seed + 31);
                m.mainHand = t[1];
                s.mobs.add(m);
                s.player = new double[] {8.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                s.dayTime = 18000;
                s.ticks = 400;
                out.add(s);
            }
            for (int dist : new int[] {5, 11}) {
                Scenario s = new Scenario("chase_" + t[0] + "_" + dist);
                floor(s, 20, "minecraft:stone");
                MobSpec m = new MobSpec("minecraft:" + t[0], 0.5, t[0].equals("vex") ? BY + 1 : BY, 0.5, 0f, 6600L + dist);
                m.mainHand = t[1];
                s.mobs.add(m);
                s.player = new double[] {0.5 + dist, BY, 0.5};
                s.dayTime = 18000;
                s.ticks = 240;
                out.add(s);
            }
        }
        // Hurt by the player: raiders take revenge (and ignore other raiders' hits).
        for (String[] t : new String[][] {{"pillager", "minecraft:crossbow"}, {"vindicator", "minecraft:iron_axe"}}) {
            Scenario s = new Scenario("hurt_" + t[0]);
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:" + t[0], 0.5, BY, 0.5, 0f, 6700);
            m.mainHand = t[1];
            s.mobs.add(m);
            MobSpec m2 = new MobSpec("minecraft:" + t[0], -3.5, BY, 2.5, 90f, 6701);
            m2.mainHand = t[1];
            s.mobs.add(m2);
            s.player = new double[] {6.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 2.0});
            s.ticks = 200;
            out.add(s);
        }
        // An evoker turns a blue sheep red.
        {
            Scenario s = new Scenario("wololo_evoker");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:evoker", 0.5, BY, 0.5, 0f, 6800));
            MobSpec sheep = new MobSpec("minecraft:sheep", 5.5, BY, 2.5, 0f, 6801);
            sheep.nbt = "{Color:11b}";
            s.mobs.add(sheep);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            out.add(s);
        }
        // Johnny attacks a cow.
        {
            Scenario s = new Scenario("johnny_vindicator");
            floor(s, 16, "minecraft:grass_block");
            MobSpec j = new MobSpec("minecraft:vindicator", 0.5, BY, 0.5, 0f, 6900);
            j.mainHand = "minecraft:iron_axe";
            j.nbt = "{Johnny:1b}";
            s.mobs.add(j);
            s.mobs.add(new MobSpec("minecraft:cow", 5.5, BY, 1.5, 0f, 6901));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            out.add(s);
        }
        // A patrol: the leader walks toward its far target, the others follow; spotting a
        // survival player they hold their ground.
        for (boolean player : new boolean[] {false, true}) {
            Scenario s = new Scenario("patrol_pillagers" + (player ? "_hold" : ""));
            floor(s, 30, "minecraft:grass_block");
            MobSpec leader = new MobSpec("minecraft:pillager", 0.5, BY, 0.5, 0f, 7000);
            leader.mainHand = "minecraft:crossbow";
            leader.nbt = "{PatrolLeader:1b,Patrolling:1b,patrol_target:[I;300,100,40]}";
            s.mobs.add(leader);
            for (int i = 1; i <= 2; i++) {
                MobSpec f = new MobSpec("minecraft:pillager", 0.5 - 2 * i, BY, 1.5, 0f, 7000 + i);
                f.mainHand = "minecraft:crossbow";
                f.nbt = "{Patrolling:1b,patrol_target:[I;300,100,40]}";
                s.mobs.add(f);
            }
            s.player = new double[] {player ? 14.5 : 0.5, BY, player ? 0.5 : 25.5};
            s.playerCreative = !player;
            s.dayTime = player ? 18000 : 1000;
            s.ticks = 300;
            out.add(s);
        }
        // Two ravagers and a vindicator push and bite.
        {
            Scenario s = new Scenario("ravager_bite");
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:ravager", 0.5, BY, 0.5, 0f, 7100));
            s.player = new double[] {3.5, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
    }


    // ---------------------------------------------------------- slice 3: the end fight
    // The ender dragon outside a fight (`dragonFight` null: no crystals counted, the inner
    // node rings), a bedrock pad at the origin for its landings (the podium is where it lands).
    static void scenariosEnd(List<Scenario> out) {
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("dragon_hold_" + seed);
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, 128, 0.5, 60f * seed, 44000L + seed);
            m.nbt = "{DragonPhase:0}";
            s.mobs.add(m);
            s.levelSeed = seed;
            s.ticks = 1500;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("dragon_sit_" + seed);
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, BY, 0.5, 90f * seed, 45000L + seed);
            m.nbt = "{DragonPhase:6}";
            s.mobs.add(m);
            s.ticks = 700;
            out.add(s);
        }
        {
            Scenario s = new Scenario("dragon_approach");
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 30.5, 110, -20.5, 10f, 46001);
            m.nbt = "{DragonPhase:2}";
            s.mobs.add(m);
            s.ticks = 900;
            out.add(s);
        }
        {
            // Crystals within 32 blocks heal the dragon (one point every 10 ticks).
            Scenario s = new Scenario("dragon_crystal_heal");
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, 110, 0.5, 0f, 47001);
            m.nbt = "{DragonPhase:10,Health:120f}";
            s.mobs.add(m);
            s.others.add(new MobSpec("minecraft:end_crystal", 20.5, 112, 0.5, 0f, 0));
            s.others.add(new MobSpec("minecraft:end_crystal", -12.5, 104, 8.5, 0f, 0));
            s.ticks = 500;
            out.add(s);
        }
        {
            // A creative player (never targeted) wounds, then kills the dragon: the dying phase
            // flies to the pad, then the 200-tick death with its experience.
            Scenario s = new Scenario("dragon_hurt_die");
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, 118, 0.5, 30f, 48001);
            m.nbt = "{DragonPhase:0}";
            s.mobs.add(m);
            s.player = new double[] {0.5, BY, 30.5};
            s.playerCreative = true;
            s.hurts.put(20, new double[] {0, 30.0});
            s.hurts.put(40, new double[] {0, 12.0});
            s.hurts.put(300, new double[] {0, 400.0});
            s.ticks = 700;
            out.add(s);
        }
        {
            // Hits while sitting: a quarter of its health and it takes off.
            Scenario s = new Scenario("dragon_sit_hurt");
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, BY, 0.5, 0f, 49001);
            m.nbt = "{DragonPhase:6}";
            s.mobs.add(m);
            s.player = new double[] {0.5, BY, 30.5};
            s.playerCreative = true;
            s.hurts.put(10, new double[] {0, 30.0});
            s.hurts.put(30, new double[] {0, 30.0});
            s.hurts.put(50, new double[] {0, 30.0});
            s.ticks = 400;
            out.add(s);
        }
        {
            // A survival player in front of the sitting dragon: it turns to face it, roars,
            // then breathes its flame (a dragon's breath cloud that harms).
            Scenario s = new Scenario("dragon_sit_player");
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, BY, 0.5, 90f, 50001);
            m.nbt = "{DragonPhase:6}";
            s.mobs.add(m);
            s.player = new double[] {0.5, BY, 12.5};
            s.ticks = 900;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            // A survival player below the holding pattern: strafing runs with fireballs (and
            // their clouds), landings and charges.
            Scenario s = new Scenario("dragon_strafe_" + seed);
            floor(s, 6, "minecraft:bedrock");
            MobSpec m = new MobSpec("minecraft:ender_dragon", 0.5, 128, 0.5, 45f * seed, 51000L + seed);
            m.nbt = "{DragonPhase:0}";
            s.mobs.add(m);
            s.player = new double[] {30.5, BY, 10.5};
            s.levelSeed = seed;
            s.ticks = 1500;
            out.add(s);
        }
    }


    // ---------------------------------------------------------- slice 3: wither and guardians
    static void scenariosWither(List<Scenario> out) {
        // The wither idles over the ground: hovering strolls (flying navigation), gravity
        // while it does not move, the heads' particles drawing from its random.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_wither_" + seed);
            solidGround(s);
            s.mobs.add(new MobSpec("minecraft:wither", 0.5, BY + 3, 0.5, 60f * seed, 11000L + seed));
            s.player = new double[] {14.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 300;
            out.add(s);
        }
        // It shoots skulls at a survival player (the middle head's ranged attack, the side
        // heads' own targets, the explosions breaking the floor).
        for (int dist : new int[] {8, 16}) {
            Scenario s = new Scenario("attack_wither_" + dist);
            solidGround(s);
            s.mobs.add(new MobSpec("minecraft:wither", 0.5, BY + 2, 0.5, 90f, 11100L + dist));
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.ticks = 200;
            out.add(s);
        }
        // Just built: invulnerable, healing, then the power 7 explosion.
        {
            Scenario s = new Scenario("summoned_wither");
            solidGround(s);
            MobSpec m = new MobSpec("minecraft:wither", 0.5, BY, 0.5, 0f, 11200L);
            m.nbt = "{Invul:60,Health:100f}";
            s.mobs.add(m);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 120;
            out.add(s);
        }
        // Hurt at half health: powered (it stays low over its target), breaking the blocks
        // around it a second after the hit.
        {
            Scenario s = new Scenario("hurt_wither");
            solidGround(s);
            for (int y = BY; y <= BY + 4; y++) block(s, 1, y, 1, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:wither", 0.5, BY, 0.5, 0f, 11300L);
            m.nbt = "{Health:140f}";
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 0.5};
            s.hurts.put(3, new double[] {0, 4.0});
            s.ticks = 150;
            out.add(s);
        }
        // Guardians in a pool: swimming strolls, the elder's home and slower strolls.
        String[][] guardians = {{"guardian", "12000"}, {"elder_guardian", "12100"}};
        for (String[] g : guardians) {
            for (int seed = 1; seed <= 2; seed++) {
                Scenario s = new Scenario("swim_" + g[0] + "_" + seed);
                pool(s);
                s.mobs.add(new MobSpec("minecraft:" + g[0], 0.5, BY + 2, 0.5, 45f * seed, Long.parseLong(g[1]) + seed));
                s.player = new double[] {12.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                s.ticks = 400;
                out.add(s);
            }
            // The beam at a survival player at the pool's edge, the spikes and the stroll after
            // a hit.
            Scenario s = new Scenario("attack_" + g[0]);
            pool(s);
            s.mobs.add(new MobSpec("minecraft:" + g[0], 0.5, BY + 2, 0.5, 90f, Long.parseLong(g[1]) + 50));
            s.player = new double[] {5.5, BY + 6, 0.5};
            s.hurts.put(150, new double[] {0, 1.0});
            // (The elder's wobble meets a `Math.sin` argument where HotSpot's intrinsic and the
            // correctly rounded sine differ in the last bit at tick 257.)
            s.ticks = 250;
            out.add(s);
        }
        // Out of the water it flops.
        {
            Scenario s = new Scenario("flop_guardian");
            solidGround(s);
            s.mobs.add(new MobSpec("minecraft:guardian", 0.5, BY, 0.5, 0f, 12200L));
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 120;
            out.add(s);
        }
    }

    /// A stone floor over solid stone down to the cleared depth: random positions under an
    /// open floor would depend on sky light the replay does not model.
    static void solidGround(Scenario s) {
        floor(s, 24, "minecraft:stone");
        for (int x = -20; x <= 20; x++)
            for (int z = -20; z <= 20; z++)
                for (int y = BY - 8; y <= BY - 2; y++) block(s, x, y, z, "minecraft:stone");
    }

    /// A 9x9, 5 deep pool of water walled in on the floor, over solid ground.
    static void pool(Scenario s) {
        solidGround(s);
        for (int x = -5; x <= 5; x++)
            for (int z = -5; z <= 5; z++)
                for (int y = BY; y <= BY + 5; y++) {
                    boolean inside = Math.abs(x) <= 4 && Math.abs(z) <= 4;
                    if (!inside) block(s, x, y, z, "minecraft:stone");
                    else if (y <= BY + 4) block(s, x, y, z, "minecraft:water");
                }
    }


    // ---------------------------------------------------------- slice 3: warden and sculk
    static void scenariosWarden(List<Scenario> out) {
    }


    // ---------------------------------------------------------- slice 3: common mobs A
    static void scenariosCommonA(List<Scenario> out) {
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_rabbit_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:rabbit", 0.5, BY, 0.5, 40f * seed, 11000L * seed + 5));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("flee_rabbit");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:rabbit", 0.5, BY, 0.5, 0f, 11100));
            s.player = new double[] {4.5, BY, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("tempt_rabbit");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:rabbit", 0.5, BY, 0.5, 0f, 11200));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = "minecraft:carrot";
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("garden_rabbit");
            floor(s, 16, "minecraft:grass_block");
            for (int x = 3; x <= 5; x++) {
                block(s, x, BY - 1, 2, "minecraft:farmland");
                block(s, x, BY, 2, "minecraft:carrots[age=7]");
            }
            s.mobs.add(new MobSpec("minecraft:rabbit", 0.5, BY, 0.5, 0f, 11300));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 500;
            out.add(s);
        }
        {
            Scenario s = new Scenario("killer_bunny");
            floor(s, 20, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:rabbit", 0.5, BY, 0.5, 0f, 11400);
            m.nbt = "{RabbitType:99}";
            s.mobs.add(m);
            s.player = new double[] {6.5, BY, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_polar_bear_" + seed);
            floor(s, 16, "minecraft:snow_block");
            s.mobs.add(new MobSpec("minecraft:polar_bear", 0.5, BY, 0.5, 50f * seed, 12000L * seed + 3));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("anger_polar_bear");
            floor(s, 20, "minecraft:snow_block");
            s.mobs.add(new MobSpec("minecraft:polar_bear", 0.5, BY, 0.5, 0f, 12100));
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("cub_polar_bear");
            floor(s, 20, "minecraft:snow_block");
            s.mobs.add(new MobSpec("minecraft:polar_bear", 0.5, BY, 0.5, 0f, 12200));
            MobSpec cub = new MobSpec("minecraft:polar_bear", -2.5, BY, 1.5, 90f, 12201);
            cub.age = -24000;
            s.mobs.add(cub);
            s.player = new double[] {6.5, BY, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_cub_polar_bear");
            floor(s, 20, "minecraft:snow_block");
            MobSpec cub = new MobSpec("minecraft:polar_bear", 0.5, BY, 0.5, 0f, 12300);
            cub.age = -24000;
            s.mobs.add(cub);
            s.mobs.add(new MobSpec("minecraft:polar_bear", -4.5, BY, 2.5, 90f, 12301));
            s.player = new double[] {3.5, BY, 0.5};
            s.playerCreative = false;
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 300;
            out.add(s);
        }
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_turtle_" + seed);
            floor(s, 16, "minecraft:sand");
            s.mobs.add(new MobSpec("minecraft:turtle", 0.5, BY, 0.5, 70f * seed, 13000L * seed + 9));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            // A beach: sand with a pool; the turtle heads for the water and swims.
            Scenario s = new Scenario("beach_turtle");
            floor(s, 16, "minecraft:sand");
            for (int x = 3; x <= 9; x++)
                for (int z = -3; z <= 3; z++) {
                    block(s, x, BY - 2, z, "minecraft:sand");
                    block(s, x, BY - 1, z, "minecraft:water");
                }
            s.mobs.add(new MobSpec("minecraft:turtle", -1.5, BY, 0.5, 0f, 13100));
            MobSpec baby = new MobSpec("minecraft:turtle", -2.5, BY, 2.5, 90f, 13101);
            baby.age = -24000;
            s.mobs.add(baby);
            s.player = new double[] {-10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("tempt_turtle");
            floor(s, 16, "minecraft:sand");
            s.mobs.add(new MobSpec("minecraft:turtle", 0.5, BY, 0.5, 0f, 13200));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = "minecraft:seagrass";
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("egg_turtle");
            floor(s, 16, "minecraft:sand");
            MobSpec m = new MobSpec("minecraft:turtle", 0.5, BY, 0.5, 0f, 13300);
            m.nbt = "{has_egg:1b,home_pos:[I;0," + BY + ",0]}";
            s.mobs.add(m);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        for (int seed = 1; seed <= 3; seed++) {
            Scenario s = new Scenario("idle_fox_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:fox", 0.5, BY, 0.5, 35f * seed, 14000L * seed + 1));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            // Night: by day `SeekShelterGoal` looks for shade, and the harness world never
            // relights under its floor (vanilla sees open sky everywhere, Kiln darkness).
            s.dayTime = 14000 + 1000 * seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("flee_fox");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:fox", 0.5, BY, 0.5, 0f, 14100));
            s.player = new double[] {6.5, BY, 0.5};
            s.ticks = 300;
            s.dayTime = 18000;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hunt_fox");
            floor(s, 20, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:fox", 0.5, BY, 0.5, 0f, 14200);
            m.nbt = "{Type:\"red\"}";
            s.mobs.add(m);
            s.mobs.add(new MobSpec("minecraft:chicken", 9.5, BY, 3.5, 0f, 14201));
            s.player = new double[] {-12.5, BY, 0.5};
            s.playerCreative = true;
            // The chicken dies at tick 86; its loot (not replayed in Rust) later draws the fox.
            s.ticks = 95;
            s.dayTime = 18000;
            out.add(s);
        }
        {
            Scenario s = new Scenario("berries_fox");
            floor(s, 16, "minecraft:grass_block");
            block(s, 4, BY, 3, "minecraft:sweet_berry_bush[age=3]");
            block(s, -3, BY, -4, "minecraft:sweet_berry_bush[age=2]");
            s.mobs.add(new MobSpec("minecraft:fox", 0.5, BY, 0.5, 0f, 14300));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 600;
            s.dayTime = 18000;
            out.add(s);
        }
        String[] genes = {"normal", "lazy", "worried", "playful", "aggressive"};
        for (int i = 0; i < genes.length; i++) {
            Scenario s = new Scenario("idle_panda_" + genes[i]);
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:panda", 0.5, BY, 0.5, 25f * i, 15000L + 13 * i);
            m.nbt = "{MainGene:\"" + genes[i] + "\",HiddenGene:\"" + genes[i] + "\"}";
            s.mobs.add(m);
            s.player = new double[] {5.5, BY, 0.5};
            s.playerCreative = !genes[i].equals("worried");
            s.levelSeed = i;
            s.ticks = 500;
            out.add(s);
        }
        {
            Scenario s = new Scenario("cub_panda");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:panda", 0.5, BY, 0.5, 0f, 15100);
            m.nbt = "{MainGene:\"weak\",HiddenGene:\"weak\"}";
            m.age = -24000;
            s.mobs.add(m);
            s.mobs.add(new MobSpec("minecraft:panda", 3.5, BY, 2.5, 90f, 15101));
            s.player = new double[] {10.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 500;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_panda");
            floor(s, 16, "minecraft:grass_block");
            MobSpec m = new MobSpec("minecraft:panda", 0.5, BY, 0.5, 0f, 15200);
            m.nbt = "{MainGene:\"aggressive\",HiddenGene:\"normal\"}";
            s.mobs.add(m);
            s.player = new double[] {3.5, BY, 0.5};
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 300;
            out.add(s);
        }
    }


    // ---------------------------------------------------------- slice 3: common mobs B
    /// A pool of water `r` blocks around the origin, `depth` deep, on a stone floor.
    static void pool(Scenario s, int r, int depth) {
        floor(s, r + 4, "minecraft:stone");
        for (int x = -r; x <= r; x++)
            for (int z = -r; z <= r; z++)
                for (int y = BY; y < BY + depth; y++) block(s, x, y, z, "minecraft:water");
    }

    /// Constructor draws of the common mobs B types (from the unpinnable constructor random): both
    /// sides take them from a random seeded 0, as Kiln's replay constructs its mobs.
    static void pinCommonB(Mob m) throws Exception {
        var r = new net.minecraft.world.level.levelgen.LegacyRandomSource(0L);
        if (m instanceof net.minecraft.world.entity.animal.squid.Squid) {
            set(m, "tentacleSpeed", 1.0F / (r.nextFloat() + 1.0F) * 0.2F);
        }
        if (m instanceof net.minecraft.world.entity.animal.fish.AbstractSchoolingFish) {
            int start = (200 + r.nextInt(200) % 20 + 1) / 2;
            for (WrappedGoal g : ((net.minecraft.world.entity.ai.goal.GoalSelector) get(m, "goalSelector")).getAvailableGoals()) {
                if (g.getGoal() instanceof net.minecraft.world.entity.ai.goal.FollowFlockLeaderGoal f) set(f, "nextStartTick", start);
            }
        }
    }

    static void scenariosCommonB(List<Scenario> out) {
        // Squids and glow squids: swimming by tentacle pulses, fleeing and squirting ink when
        // hurt, drowning on land.
        for (String type : new String[] {"squid", "glow_squid"}) {
            for (int seed = 1; seed <= 2; seed++) {
                Scenario s = new Scenario("idle_" + type + "_" + seed);
                pool(s, 8, 6);
                s.mobs.add(new MobSpec("minecraft:" + type, 0.5, BY + 2, 0.5, 40f * seed, 11000L * seed + 7));
                s.player = new double[] {12.5, BY, 0.5};
                s.playerCreative = true;
                s.levelSeed = seed;
                s.ticks = 400;
                out.add(s);
            }
            {
                Scenario s = new Scenario("hurt_" + type);
                pool(s, 8, 6);
                s.mobs.add(new MobSpec("minecraft:" + type, 0.5, BY + 2, 0.5, 0f, 11100));
                s.player = new double[] {4.5, BY + 2, 0.5};
                s.playerCreative = true;
                s.hurts.put(5, new double[] {0, 1.0});
                s.hurts.put(60, new double[] {0, 1.0});
                s.ticks = 200;
                out.add(s);
            }
            {
                Scenario s = new Scenario("land_" + type);
                floor(s, 8, "minecraft:stone");
                s.mobs.add(new MobSpec("minecraft:" + type, 0.5, BY, 0.5, 0f, 11200));
                s.player = new double[] {8.5, BY, 0.5};
                s.playerCreative = true;
                s.ticks = 400;
                out.add(s);
            }
        }
        // Fish: swimming about (salmon sizes, tropical patterns, a pufferfish), schooling, flopping
        // and drowning on land, panicking, keeping away from players, puffing up.
        String[][] fish = {
            {"cod", null}, {"cod", null}, {"salmon", "{type:\"small\"}"}, {"salmon", "{type:\"large\"}"},
            {"tropical_fish", "{Variant:117506305}"}, {"tropical_fish", null}, {"pufferfish", null}, {"pufferfish", "{PuffState:2}"},
        };
        for (int i = 0; i < fish.length; i++) {
            Scenario s = new Scenario("idle_" + fish[i][0] + "_" + i);
            pool(s, 8, 6);
            MobSpec m = new MobSpec("minecraft:" + fish[i][0], 0.5, BY + 2, 0.5, 45f * i, 12000L + 31 * i);
            m.nbt = fish[i][1];
            s.mobs.add(m);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = i + 1;
            s.ticks = 400;
            out.add(s);
        }
        for (String type : new String[] {"cod", "salmon", "tropical_fish"}) {
            Scenario s = new Scenario("school_" + type);
            pool(s, 10, 6);
            for (int k = 0; k < 4; k++) s.mobs.add(new MobSpec("minecraft:" + type, 0.5 + 1.5 * k, BY + 1 + (k % 2), 0.5 - k, 30f * k, 12500L + k));
            s.player = new double[] {14.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 500;
            out.add(s);
        }
        for (String type : new String[] {"cod", "pufferfish"}) {
            Scenario s = new Scenario("land_" + type);
            floor(s, 8, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:" + type, 0.5, BY, 0.5, 0f, 12600));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_cod");
            pool(s, 8, 6);
            s.mobs.add(new MobSpec("minecraft:cod", 0.5, BY + 2, 0.5, 0f, 12700));
            s.player = new double[] {3.5, BY + 2, 0.5};
            s.playerCreative = true;
            s.hurts.put(5, new double[] {0, 1.0});
            s.ticks = 200;
            out.add(s);
        }
        for (String type : new String[] {"cod", "pufferfish"}) {
            Scenario s = new Scenario("near_player_" + type);
            pool(s, 8, 6);
            s.mobs.add(new MobSpec("minecraft:" + type, 0.5, BY + 2, 0.5, 0f, 12800));
            s.player = new double[] {2.0, BY + 2, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        // Mooshrooms: a cow's life on mycelium, both colors.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_mooshroom_" + seed);
            floor(s, 16, "minecraft:mycelium");
            MobSpec m = new MobSpec("minecraft:mooshroom", 0.5, BY, 0.5, 50f * seed, 13000L * seed + 3);
            if (seed == 2) m.nbt = "{Type:\"brown\"}";
            s.mobs.add(m);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("breed_mooshroom");
            floor(s, 16, "minecraft:mycelium");
            MobSpec m1 = new MobSpec("minecraft:mooshroom", 0.5, BY, 0.5, 20f, 13100);
            MobSpec m2 = new MobSpec("minecraft:mooshroom", 3.5, BY, 1.5, 200f, 13101);
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
            Scenario s = new Scenario("tempt_mooshroom");
            floor(s, 16, "minecraft:mycelium");
            s.mobs.add(new MobSpec("minecraft:mooshroom", 0.5, BY, 0.5, 0f, 13200));
            s.player = new double[] {6.5, BY, 0.5};
            s.playerCreative = true;
            s.playerMainHand = "minecraft:wheat";
            s.ticks = 200;
            out.add(s);
        }
        // Ocelots: wandering, keeping away from players, tempted by fish, hunting chickens.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_ocelot_" + seed);
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:ocelot", 0.5, BY, 0.5, 70f * seed, 13300L * seed + 1));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("avoid_ocelot");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:ocelot", 0.5, BY, 0.5, 0f, 13400));
            s.player = new double[] {5.5, BY, 0.5};
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("tempt_ocelot");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:ocelot", 0.5, BY, 0.5, 0f, 13500));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerMainHand = "minecraft:cod";
            s.ticks = 300;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hunt_ocelot");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:ocelot", 0.5, BY, 0.5, 0f, 13600));
            s.mobs.add(new MobSpec("minecraft:chicken", 6.5, BY, 2.5, 0f, 13601));
            s.player = new double[] {-14.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        // Bats: hanging under a ceiling, woken by a player close by, fluttering about.
        for (int dist : new int[] {3, 9}) {
            Scenario s = new Scenario("bat_" + dist);
            floor(s, 12, "minecraft:stone");
            for (int x = -12; x <= 12; x++)
                for (int z = -12; z <= 12; z++) block(s, x, BY + 6, z, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:bat", 0.5, BY + 5, 0.5, 0f, 13700L + dist));
            s.player = new double[] {0.5 + dist, BY + 3, 0.5};
            s.playerCreative = true;
            s.ticks = 400;
            out.add(s);
        }
        // A puffed-up pufferfish stings a cod beside it (poison).
        {
            Scenario s = new Scenario("sting_pufferfish");
            pool(s, 8, 6);
            MobSpec p = new MobSpec("minecraft:pufferfish", 0.5, BY + 2, 0.5, 0f, 13900);
            p.nbt = "{PuffState:2}";
            s.mobs.add(p);
            s.mobs.add(new MobSpec("minecraft:cod", 0.9, BY + 2, 0.5, 90f, 13901));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 200;
            out.add(s);
        }
        // Snow golems: wandering with a trail of snow, throwing snowballs at a monster.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_snow_golem_" + seed);
            floor(s, 16, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:snow_golem", 0.5, BY, 0.5, 30f * seed, 14000L * seed + 9));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.ticks = 400;
            out.add(s);
        }
        {
            Scenario s = new Scenario("attack_snow_golem");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:snow_golem", 0.5, BY, 0.5, 0f, 14100));
            // Close enough that the snowballs' spread (their own random, not pinnable) cannot miss.
            MobSpec z = new MobSpec("minecraft:zombie", 2.5, BY, 0.5, 90f, 14101);
            z.nbt = "{NoAI:1b}";
            s.mobs.add(z);
            s.player = new double[] {-12.5, BY, 0.5};
            s.playerCreative = true;
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Bogged: the skeleton goals with its slower bow.
        for (int seed = 1; seed <= 2; seed++) {
            Scenario s = new Scenario("idle_bogged_" + seed);
            floor(s, 16, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:bogged", 0.5, BY, 0.5, 45f * seed, 14200L * seed + 13);
            m.mainHand = "minecraft:bow";
            s.mobs.add(m);
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.levelSeed = seed;
            s.dayTime = 18000;
            s.ticks = 400;
            out.add(s);
        }
        for (int dist : new int[] {4, 9}) {
            Scenario s = new Scenario("chase_bogged_" + dist);
            floor(s, 20, "minecraft:stone");
            MobSpec m = new MobSpec("minecraft:bogged", 0.5, BY, 0.5, 0f, 14300 + dist);
            m.mainHand = "minecraft:bow";
            s.mobs.add(m);
            s.player = new double[] {0.5 + dist, BY, 0.5};
            s.dayTime = 18000;
            s.ticks = 200;
            out.add(s);
        }
        // Armadillos (a brain in vanilla, goals in Kiln): wandering, rolling up near the undead.
        {
            Scenario s = new Scenario("idle_armadillo");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:armadillo", 0.5, BY, 0.5, 30f, 14400));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            s.diverges = true;
            out.add(s);
        }
        {
            Scenario s = new Scenario("scared_armadillo");
            floor(s, 16, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:armadillo", 0.5, BY, 0.5, 30f, 14500));
            MobSpec z = new MobSpec("minecraft:zombie", 4.5, BY, 0.5, 90f, 14501);
            z.nbt = "{NoAI:1b}";
            s.mobs.add(z);
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            s.diverges = true;
            out.add(s);
        }
        // Breezes (a brain in vanilla, a fight goal in Kiln): idle, and fighting a player.
        for (int dist : new int[] {0, 8}) {
            Scenario s = new Scenario(dist == 0 ? "idle_breeze" : "fight_breeze");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:breeze", 0.5, BY, 0.5, 0f, 14600L + dist));
            s.player = new double[] {0.5 + (dist == 0 ? 14 : dist), BY, 0.5};
            s.playerCreative = dist == 0;
            s.ticks = 200;
            s.diverges = true;
            out.add(s);
        }
        // Creakings (brain in vanilla): stared at by a survival player, and unwatched.
        for (boolean stare : new boolean[] {true, false}) {
            Scenario s = new Scenario(stare ? "stare_creaking" : "idle_creaking");
            floor(s, 20, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:creaking", 0.5, BY, 0.5, 270f, 14700));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerYaw = stare ? 90f : 270f;
            s.playerPitch = -6.6f;
            s.dayTime = 18000;
            s.ticks = 200;
            s.diverges = true;
            out.add(s);
        }
        // Sniffers (brain in vanilla): wandering and sniffing on grass.
        {
            Scenario s = new Scenario("idle_sniffer");
            floor(s, 20, "minecraft:grass_block");
            s.mobs.add(new MobSpec("minecraft:sniffer", 0.5, BY, 0.5, 20f, 14800));
            s.player = new double[] {12.5, BY, 0.5};
            s.playerCreative = true;
            s.ticks = 300;
            s.diverges = true;
            out.add(s);
        }
        {
            Scenario s = new Scenario("hurt_bat");
            floor(s, 12, "minecraft:stone");
            for (int x = -12; x <= 12; x++)
                for (int z = -12; z <= 12; z++) block(s, x, BY + 6, z, "minecraft:stone");
            s.mobs.add(new MobSpec("minecraft:bat", 0.5, BY + 5, 0.5, 0f, 13800));
            s.player = new double[] {8.5, BY, 0.5};
            s.playerCreative = true;
            s.hurts.put(10, new double[] {0, 1.0});
            s.ticks = 300;
            out.add(s);
        }
    }

}
