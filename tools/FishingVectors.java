// wp77: fishing bobbers against a vanilla 26.3 server, bit for bit: a bobber cast from a mock player's rod (Luck of the Sea and Lure from the rod, through
// FishingRodItem.use) falls into a 5x4x5 patch of random blocks around the water it bobs in; its random and its UUID are fixed, and after every tick the
// state, the nibble and wait counters, the fish angle, the biting flag, "open water" (calculateOpenWater) and the position and motion are recorded
// (FishingHook.tick and catchingFish: Lure's wait times, the open-water test of the 5x4x5 area, the bob).
//
// usage (cwd = a scratch server directory):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/FishingVectors.java <out.jsonl> [count]

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.util.RandomSource;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.projectile.FishingHook;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.enchantment.Enchantments;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

public class FishingVectors {
    static final int X = 8, Z = 8, H = 80, TICKS = 420;

    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static Object get(Object o, Class<?> c, String name) throws Exception {
        Field f = c.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(o);
    }

    static void run(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        int count = args.length > 1 ? Integer.parseInt(args[1]) : 120;
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "FishingVectors main");
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
            try {
                ServerLevel level = server.overworld();
                level.tickRateManager().setFrozen(true);
                var src = server.createCommandSourceStack();
                server.getCommands().performPrefixedCommand(src, "weather clear");
                server.getCommands().performPrefixedCommand(src, "gamerule advance_weather false");
                server.getCommands().performPrefixedCommand(src, "gamerule advance_time false");
                server.getCommands().performPrefixedCommand(src, "time set 6000");
                ServerPlayer player = mockPlayer(server, "Angler");
                player.setGameMode(GameType.SURVIVAL);
                Random rnd = new Random(7700);
                for (int i = 0; i < count; i++) lines.add(one(level, player, rnd, i));
            } catch (Throwable t) {
                t.printStackTrace();
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("FishingVectors: wrote " + lines.size() + " cases to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static ServerPlayer mockPlayer(MinecraftServer server, String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        return p;
    }

    /** The blocks a patch cell may hold. */
    static BlockState cell(Random r, int kind) {
        return switch (kind) {
            case 0 -> Blocks.AIR.defaultBlockState();
            case 1 -> Blocks.WATER.defaultBlockState();
            case 2 -> Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, 1 + r.nextInt(7));
            case 3 -> Blocks.STONE.defaultBlockState();
            case 4 -> Blocks.LILY_PAD.defaultBlockState();
            case 5 -> Blocks.KELP.defaultBlockState();
            case 6 -> Blocks.SEAGRASS.defaultBlockState();
            case 7 -> Blocks.OAK_SLAB.defaultBlockState().setValue(net.minecraft.world.level.block.SlabBlock.WATERLOGGED, true);
            case 8 -> Blocks.ICE.defaultBlockState();
            case 9 -> Blocks.CAVE_AIR.defaultBlockState();
            case 10 -> Blocks.OAK_TRAPDOOR.defaultBlockState().setValue(net.minecraft.world.level.block.TrapDoorBlock.OPEN, true);
            case 11 -> Blocks.COBWEB.defaultBlockState();
            case 12 -> Blocks.BUBBLE_COLUMN.defaultBlockState();
            default -> Blocks.LAVA.defaultBlockState();
        };
    }

    static String one(ServerLevel level, ServerPlayer player, Random r, int index) throws Exception {
        // The patch: per y layer (relative to H) all water, all air, or a mix of the palette; the centre column is water at H and air above.
        int[] layerKind = new int[6]; // y = H-2 .. H+3
        for (int i = 0; i < 6; i++) {
            int d = r.nextInt(10);
            layerKind[i] = d < 4 ? 0 : d < 7 ? 1 : 2; // 0 water, 1 air, 2 mix
        }
        if (index % 3 != 2) {
            // the plain pool most casts are made in: water up to H, air above
            for (int i = 0; i < 6; i++) layerKind[i] = i <= 2 ? 0 : 1;
        }
        int[] palette = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12};
        StringBuilder blocks = new StringBuilder();
        for (int dx = -4; dx <= 4; dx++)
            for (int dz = -4; dz <= 4; dz++)
                for (int dy = -3; dy <= 4; dy++) level.setBlock(new BlockPos(X + dx, H + dy, Z + dz), Blocks.AIR.defaultBlockState(), 2);
        for (int dx = -2; dx <= 2; dx++)
            for (int dz = -2; dz <= 2; dz++)
                for (int i = 0; i < 6; i++) {
                    int dy = i - 2;
                    BlockState s;
                    if (layerKind[i] == 0) s = Blocks.WATER.defaultBlockState();
                    else if (layerKind[i] == 1) s = Blocks.AIR.defaultBlockState();
                    else s = cell(r, palette[r.nextInt(palette.length)]);
                    boolean centre = dx == 0 && dz == 0;
                    if (centre && dy == 0) s = Blocks.WATER.defaultBlockState();
                    if (centre && dy > 0) s = Blocks.AIR.defaultBlockState();
                    if (centre && dy < 0) s = Blocks.WATER.defaultBlockState();
                    level.setBlock(new BlockPos(X + dx, H + dy, Z + dz), s, 2);
                    blocks.append(blocks.length() > 0 ? "," : "").append(Block.getId(s));
                }
        // the rod
        int luck = r.nextInt(4), lure = r.nextInt(4);
        ItemStack rod = new ItemStack(Items.FISHING_ROD);
        var enchants = level.registryAccess().lookupOrThrow(Registries.ENCHANTMENT);
        if (luck > 0) rod.enchant(enchants.getOrThrow(Enchantments.LUCK_OF_THE_SEA), luck);
        if (lure > 0) rod.enchant(enchants.getOrThrow(Enchantments.LURE), lure);
        player.setPos(X + 0.5, H, Z + 5.5);
        player.setYRot(180f);
        player.setXRot(-30f);
        player.setItemInHand(InteractionHand.MAIN_HAND, rod);
        // cast it, then put the bobber where the test wants it
        for (Entity e : new ArrayList<>(level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(X - 20, H - 20, Z - 20, X + 20, H + 20, Z + 20), e -> e instanceof FishingHook))) e.discard();
        rod.use(level, player, InteractionHand.MAIN_HAND);
        FishingHook hook = null;
        for (Entity e : level.getEntities((Entity) null, new net.minecraft.world.phys.AABB(X - 20, H - 20, Z - 20, X + 20, H + 20, Z + 20), e -> e instanceof FishingHook)) hook = (FishingHook) e;
        if (hook == null) throw new IllegalStateException("no bobber");
        int hookLuck = (Integer) get(hook, FishingHook.class, "luck");
        int hookLure = (Integer) get(hook, FishingHook.class, "lureSpeed");
        long seed = r.nextLong();
        long uuidLsb = r.nextLong();
        UUID uuid = new UUID(r.nextLong(), uuidLsb);
        hook.setUUID(uuid);
        Field rf = Entity.class.getDeclaredField("random");
        rf.setAccessible(true);
        rf.set(hook, new net.minecraft.world.level.levelgen.LegacyRandomSource(seed));
        hook.setPos(X + 0.5, H + 0.9, Z + 0.5);
        hook.setDeltaMovement(Vec3.ZERO);
        long gameTime = level.getGameTime();
        StringBuilder ticks = new StringBuilder();
        for (int t = 1; t <= TICKS; t++) {
            hook.tick();
            if (hook.isRemoved()) {
                ticks.append(ticks.length() > 0 ? "," : "").append("null");
                break;
            }
            Vec3 p = hook.position(), d = hook.getDeltaMovement();
            ticks.append(ticks.length() > 0 ? "," : "").append(String.format(Locale.ROOT, "[%s,%d,%d,%d,%s,%b,%b,%s,%s,%s,%s,%s,%s]",
                    ((Enum<?>) get(hook, FishingHook.class, "currentState")).ordinal(), (Integer) get(hook, FishingHook.class, "nibble"),
                    (Integer) get(hook, FishingHook.class, "timeUntilLured"), (Integer) get(hook, FishingHook.class, "timeUntilHooked"),
                    Float.toString((Float) get(hook, FishingHook.class, "fishAngle")), (Boolean) get(hook, FishingHook.class, "biting"),
                    hook.isOpenWaterFishing(), Double.toString(p.x), Double.toString(p.y), Double.toString(p.z), Double.toString(d.x), Double.toString(d.y), Double.toString(d.z)));
        }
        hook.discard();
        return String.format(Locale.ROOT, "{\"name\":\"fishing_%d\",\"luck_level\":%d,\"lure_level\":%d,\"luck\":%d,\"lure_speed\":%d,\"seed\":\"%d\",\"uuid_lsb\":\"%d\",\"game_time\":%d,\"blocks\":[%s],\"ticks\":[%s]}",
                index, luck, lure, hookLuck, hookLure, seed, uuidLsb, gameTime, blocks, ticks);
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
}
