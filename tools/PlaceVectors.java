// Differential test vectors for placing block items (wp55): for every block item of the registry
// the placement vanilla 26.3 makes (`ItemStack.useOn` by a creative player, so `BlockItem.place`:
// `getStateForPlacement`, `canSurvive`, obstruction, `setPlacedBy`, the neighbour updates that
// follow) in many surroundings, recorded in a real vanilla dedicated server started in-process.
//
// A context is a clicked block H at O = (8, 150, 8) (a palette of blocks), a face, a place for the
// cursor, the way the player looks, and a template: what else is around the place P = O + face
// (stone below, stone above, walls, water, ...). The environment is built with flags that update
// nothing, the light engine settles, and every block item is used on it in turn: what each does
// is the set of blocks that changed in the 7x7x7 box around P (position relative to O, state).
// The result `1` stands for "the default state of the item's block at P and nothing else".
// Items that change nothing are not listed. "self" contexts click a block of the item's own kind
// (slabs, candles, snow layers, ...); the vectors name the one item they are for.
//
// Line 1: {"items":[...], "version":1}; then one line per context.
//
// usage (cwd = a scratch server directory):
//   KILN_HARNESS_PORT=25581 java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/PlaceVectors.java <out.jsonl> [shared|self]

import com.mojang.authlib.GameProfile;
import io.netty.channel.embedded.EmbeddedChannel;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.Connection;
import net.minecraft.network.protocol.PacketFlow;
import net.minecraft.resources.Identifier;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.CommonListenerCookie;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.UseOnContext;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.IntegerProperty;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;

public class PlaceVectors {
    static MinecraftServer server;
    static ServerLevel level;
    static ServerPlayer player;
    static final BlockPos O = new BlockPos(8, 150, 8);
    /** Block updates nothing (`UPDATE_SKIP_ALL_SIDEEFFECTS` and clients). */
    static final int QUIET = 818;
    static final int R = 3;

    static final String[] HITS = {"minecraft:stone", "minecraft:grass_block", "minecraft:sand", "minecraft:dirt", "minecraft:soul_sand", "minecraft:farmland[moisture=0]",
            "minecraft:oak_leaves[persistent=true]", "minecraft:glass", "minecraft:oak_fence", "minecraft:oak_slab[type=top]", "minecraft:oak_slab[type=bottom]", "minecraft:snow_block",
            "minecraft:hopper[facing=down]", "minecraft:cactus", "minecraft:netherrack", "minecraft:ice", "minecraft:mud", "minecraft:moss_block", "minecraft:magma_block",
            "minecraft:composter", "minecraft:stone_brick_wall", "minecraft:oak_log", "minecraft:end_stone", "minecraft:podzol", "minecraft:clay", "minecraft:crimson_nylium",
            "minecraft:bell[attachment=floor]", "minecraft:short_grass", "minecraft:snow[layers=1]", "minecraft:oak_trapdoor[half=top,open=false]", "minecraft:iron_bars", "minecraft:soul_soil"};
    static final int TEMPLATES = 10;
    static final String STONE = "minecraft:stone";
    static final String WATER = "minecraft:water[level=0]";

    // ---------------------------------------------------------------- contexts

    static final class Ctx {
        List<Object[]> env = new ArrayList<>();
        Direction face;
        double[] cursor;
        float yaw, pitch;
        boolean sneak;
        /** Non-null: only this item is tried. */
        String only;

        void put(BlockPos p, String s) {
            for (Object[] e : env) {
                if (e[0].equals(p.getX() - O.getX()) && e[1].equals(p.getY() - O.getY()) && e[2].equals(p.getZ() - O.getZ())) {
                    e[3] = s;
                    return;
                }
            }
            env.add(new Object[] {p.getX() - O.getX(), p.getY() - O.getY(), p.getZ() - O.getZ(), s});
        }
    }

    static void template(Ctx c, int t, BlockPos p) {
        switch (t) {
            case 0 -> {}
            case 1 -> c.put(p.below(), STONE);
            case 2 -> c.put(p.above(), STONE);
            case 3 -> {
                for (Direction d : Direction.Plane.HORIZONTAL) c.put(p.relative(d), STONE);
            }
            case 4 -> c.put(p, WATER);
            case 5 -> {
                c.put(p, WATER);
                c.put(p.above(), WATER);
            }
            case 6 -> c.put(p, "minecraft:water[level=2]");
            case 7 -> {
                c.put(p.below(), STONE);
                c.put(p.above(), STONE);
            }
            case 8 -> {
                c.put(p.below(), "minecraft:sand");
                c.put(p, WATER);
            }
            case 9 -> {
                c.put(p.above(), STONE);
                c.put(p, WATER);
            }
            default -> throw new IllegalStateException();
        }
    }

    static void look(Ctx c, Random r) {
        c.yaw = new float[] {0f, 90f, 180f, 270f}[r.nextInt(4)];
        c.pitch = new float[] {0f, 60f, -60f}[r.nextInt(3)];
        double[] q = {0.25, 0.5, 0.75};
        double a = q[r.nextInt(3)], b = q[r.nextInt(3)];
        c.cursor = switch (c.face) {
            case UP -> new double[] {a, 1.0, b};
            case DOWN -> new double[] {a, 0.0, b};
            case NORTH -> new double[] {a, b, 0.0};
            case SOUTH -> new double[] {a, b, 1.0};
            case WEST -> new double[] {0.0, b, a};
            default -> new double[] {1.0, b, a};
        };
    }

    static List<Ctx> sharedContexts() {
        Random r = new Random(55);
        List<Ctx> out = new ArrayList<>();
        for (String h : HITS) {
            for (Direction face : Direction.values()) {
                for (int t = 0; t < TEMPLATES; t++) {
                    Ctx c = new Ctx();
                    c.face = face;
                    template(c, t, O.relative(face));
                    c.put(O, h);
                    look(c, r);
                    out.add(c);
                }
            }
        }
        // every way of looking, against stone and grass floors and ceilings
        for (String h : new String[] {"minecraft:stone", "minecraft:grass_block"}) {
            for (Direction face : Direction.values()) {
                for (int t = 0; t < 2; t++) {
                    for (float yaw : new float[] {0f, 90f, 180f, 270f}) {
                        for (float pitch : new float[] {0f, 60f, -60f}) {
                            Ctx c = new Ctx();
                            c.face = face;
                            template(c, t, O.relative(face));
                            c.put(O, h);
                            look(c, r);
                            c.yaw = yaw;
                            c.pitch = pitch;
                            out.add(c);
                        }
                    }
                }
            }
        }
        return out;
    }

    static String text(BlockState s) {
        return BlockStateParser.serialize(s);
    }

    static BlockState parse(String s) {
        try {
            return BlockStateParser.parseForBlock(BuiltInRegistries.BLOCK, s, false).blockState();
        } catch (Exception e) {
            throw new IllegalStateException(s, e);
        }
    }

    static List<Ctx> selfContexts(List<BlockItem> items) {
        Random r = new Random(56);
        List<Ctx> out = new ArrayList<>();
        for (BlockItem item : items) {
            String name = BuiltInRegistries.ITEM.getKey(item).toString();
            BlockState d = item.getBlock().defaultBlockState();
            List<String> hits = new ArrayList<>();
            hits.add(text(d));
            for (Property<?> p : d.getProperties()) {
                if (p instanceof IntegerProperty ip && java.util.Set.of("layers", "candles", "pickles", "eggs", "flower_amount", "segment_amount").contains(p.getName())) {
                    int max = ip.getPossibleValues().stream().max(Integer::compare).get();
                    hits.add(text(d.setValue(ip, max)));
                    if (max > 2) hits.add(text(d.setValue(ip, max - 1)));
                }
                if (p.getName().equals("type") && p.getValueClass().getSimpleName().equals("SlabType")) {
                    hits.add(text(parseWith(d, "type", "top")));
                }
                if (p.getName().equals("half") && p.getPossibleValues().size() == 2) {
                    BlockState u = parseWith(d, "half", "upper");
                    if (u != null) hits.add(text(u));
                }
            }
            for (String h : hits) {
                for (Direction face : Direction.values()) {
                    for (int floor = 0; floor < 2; floor++) {
                        Ctx c = new Ctx();
                        c.only = name;
                        c.face = face;
                        if (floor == 1) {
                            c.put(O.below(), STONE);
                            c.put(O.relative(face).below(), STONE);
                        }
                        c.put(O, h);
                        look(c, r);
                        out.add(c);
                    }
                }
            }
        }
        return out;
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static BlockState parseWith(BlockState s, String prop, String value) {
        for (Property p : s.getProperties()) {
            if (p.getName().equals(prop)) {
                var v = p.getValue(value);
                if (v.isPresent()) return s.setValue(p, (Comparable) v.get());
            }
        }
        return s;
    }

    // ---------------------------------------------------------------- running

    static void setQuiet(BlockPos p, BlockState s) {
        level.setBlock(p, s, QUIET);
    }

    static void clearWindow(BlockPos around) {
        for (int x = -R - 1; x <= R + 1; x++)
            for (int y = -R - 1; y <= R + 1; y++)
                for (int z = -R - 1; z <= R + 1; z++) {
                    BlockPos p = around.offset(x, y, z);
                    if (!level.getBlockState(p).isAir()) setQuiet(p, net.minecraft.world.level.block.Blocks.AIR.defaultBlockState());
                }
    }

    static BlockState[] snapshot(BlockPos p) {
        int n = 2 * R + 1;
        BlockState[] out = new BlockState[n * n * n];
        int i = 0;
        for (int x = -R; x <= R; x++)
            for (int y = -R; y <= R; y++)
                for (int z = -R; z <= R; z++) out[i++] = level.getBlockState(p.offset(x, y, z));
        return out;
    }

    static String run(Ctx c, int id, List<BlockItem> items) throws Exception {
        BlockPos p = O.relative(c.face);
        // a window around both the clicked block and the place
        clearWindow(O);
        clearWindow(p);
        for (Object[] e : c.env) setQuiet(O.offset((int) e[0], (int) e[1], (int) e[2]), parse((String) e[3]));
        awaitLight();
        List<Object> light = new ArrayList<>();
        for (int x = -1; x <= 1; x++)
            for (int y = -1; y <= 1; y++)
                for (int z = -1; z <= 1; z++) {
                    BlockPos q = p.offset(x, y, z);
                    int raw = level.getRawBrightness(q, 0);
                    int block = level.getBrightness(net.minecraft.world.level.LightLayer.BLOCK, q);
                    if (raw != 15 || block != 0) light.add(List.of(q.getX() - O.getX(), q.getY() - O.getY(), q.getZ() - O.getZ(), raw, block));
                }
        BlockState[] before = snapshot(p);
        player.snapTo(8.5, 300.0, 8.5, c.yaw, c.pitch);
        player.setShiftKeyDown(c.sneak);
        Map<String, Object> results = new LinkedHashMap<>();
        Vec3 loc = new Vec3(O.getX() + c.cursor[0], O.getY() + c.cursor[1], O.getZ() + c.cursor[2]);
        for (BlockItem item : items) {
            String name = BuiltInRegistries.ITEM.getKey(item).toString();
            if (c.only != null && !c.only.equals(name)) continue;
            ItemStack stack = new ItemStack(item);
            // Items that fail may still print a stack trace of an unrelated reason: say it.
            try {
                player.setItemInHand(InteractionHand.MAIN_HAND, stack);
                stack.useOn(new UseOnContext(player, InteractionHand.MAIN_HAND, new BlockHitResult(loc, c.face, O, false)));
            } catch (Throwable t) {
                System.out.println("PlaceVectors: " + name + " in context " + id + ": " + t);
            }
            BlockState[] after = snapshot(p);
            List<Object> changes = new ArrayList<>();
            int i = 0;
            for (int x = -R; x <= R; x++)
                for (int y = -R; y <= R; y++)
                    for (int z = -R; z <= R; z++) {
                        if (after[i] != before[i]) {
                            BlockPos q = p.offset(x, y, z);
                            changes.add(List.of(q.getX() - O.getX(), q.getY() - O.getY(), q.getZ() - O.getZ(), text(after[i])));
                            setQuiet(q, before[i]);
                        }
                        i++;
                    }
            if (!changes.isEmpty()) {
                // (only what reads the light needs it settled before the next item)
                if (lightSensitive(item)) awaitLight();
                // the default state of the item's block at P, nothing else: `1`
                if (changes.size() == 1) {
                    @SuppressWarnings("unchecked")
                    List<Object> ch = (List<Object>) changes.get(0);
                    if (ch.get(0).equals(p.getX() - O.getX()) && ch.get(1).equals(p.getY() - O.getY()) && ch.get(2).equals(p.getZ() - O.getZ())
                            && ch.get(3).equals(text(item.getBlock().defaultBlockState()))) {
                        results.put(name, 1);
                        continue;
                    }
                }
                results.put(name, changes);
            }
            // (items that spawn entities or fill block entities: nothing of it stays)
            level.getBlockEntity(p);
        }
        drain();
        for (var e : level.getEntities((net.minecraft.world.entity.Entity) null, new net.minecraft.world.phys.AABB(p).inflate(6), en -> !(en instanceof net.minecraft.world.entity.player.Player))) {
            e.discard();
        }
        Map<String, Object> line = new LinkedHashMap<>();
        line.put("id", id);
        line.put("env", c.env);
        line.put("face", c.face.get3DDataValue());
        line.put("cursor", c.cursor);
        line.put("yaw", (double) c.yaw);
        line.put("pitch", (double) c.pitch);
        line.put("sneak", c.sneak);
        if (c.only != null) line.put("only", c.only);
        line.put("light", light);
        line.put("r", results);
        return toJson(line);
    }

    static boolean lightSensitive(BlockItem item) {
        var b = item.getBlock();
        return b instanceof net.minecraft.world.level.block.MushroomBlock || b instanceof net.minecraft.world.level.block.CropBlock || b instanceof net.minecraft.world.level.block.PitcherCropBlock;
    }

    static void awaitLight() {
        var engine = level.getChunkSource().getLightEngine();
        for (int i = 0; i < 2; i++) {
            engine.tryScheduleUpdate();
            var done = engine.waitForPendingTasks(0, 0);
            long deadline = System.nanoTime() + 60_000_000_000L;
            server.managedBlock(() -> done.isDone() || System.nanoTime() > deadline);
            if (!done.isDone()) throw new IllegalStateException("light engine did not settle");
        }
    }

    static void drain() throws Exception {
        Object connection = get(player.connection, "connection");
        EmbeddedChannel ch = (EmbeddedChannel) get(connection, "channel");
        ch.outboundMessages().clear();
    }

    static ServerPlayer mockPlayer(String name) {
        CommonListenerCookie cookie = CommonListenerCookie.createInitial(new GameProfile(UUID.nameUUIDFromBytes(name.getBytes()), name), false);
        ServerPlayer p = new ServerPlayer(server, server.overworld(), cookie.gameProfile(), cookie.clientInformation());
        Connection connection = new Connection(PacketFlow.SERVERBOUND);
        new EmbeddedChannel(connection);
        server.getPlayerList().placeNewPlayer(connection, p, cookie);
        return p;
    }

    public static void main(String[] args) {
        try {
            main0(args);
        } catch (Throwable t) {
            t.printStackTrace();
            System.exit(1);
        }
    }

    static void main0(String[] args) throws Exception {
        Path outPath = Path.of(args[0]).toAbsolutePath();
        String mode = args.length > 1 ? args[1] : "all";
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "PlaceVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            level.setChunkForced(0, 0, true);
            level.getChunk(0, 0);
        }).get();
        Thread.sleep(1000);
        server.submit(() -> {
            player = mockPlayer("placer");
            player.setGameMode(GameType.CREATIVE);
            player.setDeltaMovement(Vec3.ZERO);
        }).get();
        List<BlockItem> items = new ArrayList<>();
        for (net.minecraft.world.item.Item it : BuiltInRegistries.ITEM) {
            if (it instanceof BlockItem bi && !(it instanceof net.minecraft.world.item.PlaceOnWaterBlockItem)) items.add(bi);
        }
        items.sort(Comparator.comparing((BlockItem i) -> !lightSensitive(i)));
        List<String> lines = new ArrayList<>();
        List<String> names = new ArrayList<>();
        for (BlockItem i : items) names.add(BuiltInRegistries.ITEM.getKey(i).toString());
        lines.add(toJson(Map.of("items", names, "version", 1)));
        List<Ctx> ctxs = new ArrayList<>();
        if (!mode.equals("self")) ctxs.addAll(sharedContexts());
        if (!mode.equals("shared")) ctxs.addAll(selfContexts(items));
        System.out.println("PlaceVectors: " + items.size() + " items, " + ctxs.size() + " contexts");
        PrintWriter out = new PrintWriter(Files.newBufferedWriter(outPath));
        out.println(lines.get(0));
        out.flush();
        int[] id = {0};
        long t0 = System.currentTimeMillis();
        for (Ctx c : ctxs) {
            int n = id[0]++;
            server.submit(() -> {
                try {
                    String l = run(c, n, items);
                    out.println(l);
                    out.flush();
                    lines.add(l);
                } catch (Throwable t) {
                    t.printStackTrace();
                    lines.add("{\"id\":" + n + ",\"error\":" + toJson(t.toString()) + "}");
                }
            }).get();
            if (n % 500 == 0) System.out.println("PlaceVectors: " + n + " contexts, " + (System.currentTimeMillis() - t0) / 1000 + " s");
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("PlaceVectors: wrote " + (lines.size() - 1) + " contexts to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=" + harnessPort(),
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
        Path world = Path.of("world");
        if (Files.exists(world)) {
            try (var walk = Files.walk(world)) {
                walk.sorted(Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    static MinecraftServer awaitServer() throws Exception {
        for (int i = 0; i < 3000; i++) {
            if (i > 600 && Thread.getAllStackTraces().keySet().stream().noneMatch(t -> t.getName().equals("Server thread"))) break;
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

    static Field field(Class<?> c, String name) throws NoSuchFieldException {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // keep looking
            }
        }
        throw new NoSuchFieldException(name);
    }

    static Object get(Object o, String name) throws Exception {
        return field(o.getClass(), name).get(o);
    }

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) {
            StringBuilder b = new StringBuilder("\"");
            for (char ch : s.toCharArray()) {
                switch (ch) {
                    case '"' -> b.append("\\\"");
                    case '\\' -> b.append("\\\\");
                    case '\n' -> b.append("\\n");
                    default -> {
                        if (ch < 0x20 || ch > 0x7e) b.append(String.format("\\u%04x", (int) ch));
                        else b.append(ch);
                    }
                }
            }
            return b.append('"').toString();
        }
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof Float f) return Float.toString(f);
        if (o instanceof Double d) return Double.toString(d);
        if (o instanceof double[] a) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < a.length; i++) b.append(i > 0 ? "," : "").append(Double.toString(a[i]));
            return b.append("]").toString();
        }
        if (o instanceof Object[] a) return toJson(java.util.Arrays.asList(a));
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            boolean first = true;
            for (var e : m.entrySet()) {
                b.append(first ? "" : ",").append(toJson(String.valueOf(e.getKey()))).append(":").append(toJson(e.getValue()));
                first = false;
            }
            return b.append("}").toString();
        }
        return toJson(String.format(Locale.ROOT, "%s", o));
    }

    static String harnessPort() {
        String env = System.getenv("KILN_HARNESS_PORT");
        if (env != null) return env;
        for (int p = 25581; p <= 25589; p++) {
            try (var s = new java.net.ServerSocket(p)) {
                return Integer.toString(p);
            } catch (java.io.IOException e) {
                // busy
            }
        }
        throw new IllegalStateException("no free harness port");
    }
}
