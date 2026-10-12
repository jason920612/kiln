// Differential test vectors for the lid of a shulker box (wp57): what a box shows to rays, to line of sight and to the block support
// checks while its lid is at every phase of the animation, recorded in a real vanilla 26.3 dedicated server started in-process.
//
// A shulker box at O = (8, 150, 8) facing each of the six directions with its block entity animation set by hand (status, progress,
// progressOld; the level is frozen), alone in empty space. For each (facing, lid state) one line records
//   shape      `getShape` (the outline; the box of the lid) as boxes relative to the block
//   collision  `getCollisionShape`
//   support    `getBlockSupportShape`
//   sturdy     `isFaceSturdy(level, pos, direction, type)` for the six directions (DOWN UP NORTH SOUTH WEST EAST) times FULL, CENTER, RIGID
//              as 18 bits, direction major
//   collider, outline   `Level.clip` with `ClipContext.Block.COLLIDER` and `OUTLINE` (no fluids) for a fixed set of rays through and around the box (the
//              lid reaches half a block out of it): per ray [type, x, y, z, direction, lx, ly, lz, inside] (type 0 a miss, 1 a block)
// Line 1: {"version":1,"rays":[[fx,fy,fz,tx,ty,tz],...]} (relative to O).
//
// usage (cwd = a scratch server directory):
//   KILN_HARNESS_PORT=25581 java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries> tools/LidVectors.java <out.jsonl>

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Locale;
import java.util.Random;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ShulkerBoxBlock;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

public class LidVectors {
    static MinecraftServer server;
    static ServerLevel level;
    static final BlockPos O = new BlockPos(8, 150, 8);

    // status, progress, progressOld
    static final Object[][] LIDS = {
        {"CLOSED", 0f, 0f}, {"OPENING", 0f, 0f}, {"OPENING", 0.1f, 0f}, {"OPENING", 0.3f, 0.2f}, {"OPENING", 0.5f, 0.4f}, {"OPENING", 0.7f, 0.6f},
        {"OPENING", 0.9f, 0.8f}, {"OPENED", 1f, 1f}, {"CLOSING", 0.9f, 1f}, {"CLOSING", 0.6f, 0.7f}, {"CLOSING", 0.3f, 0.4f}, {"CLOSING", 0.1f, 0.2f}};

    static List<double[]> rays() {
        List<double[]> r = new ArrayList<>();
        double[] u = {0.1, 0.5, 0.9};
        double[] heights = {0.1, 0.5, 0.95, 1.05, 1.2, 1.3, 1.45, 1.55, -0.2};
        for (double h : heights) {
            for (double a : u) {
                r.add(new double[] {-2, h, a, 3, h, a});
                r.add(new double[] {3, h, a, -2, h, a});
                r.add(new double[] {a, h, -2, a, h, 3});
                r.add(new double[] {a, h, 3, a, h, -2});
            }
        }
        for (double a : u) {
            for (double b : u) {
                r.add(new double[] {a, -2, b, a, 3, b});
                r.add(new double[] {a, 3, b, a, -2, b});
            }
        }
        // slanted rays from a sphere around the box to the other side
        Random rnd = new Random(57);
        for (int i = 0; i < 160; i++) {
            double[] f = new double[3];
            double len = 0;
            for (int k = 0; k < 3; k++) {
                f[k] = rnd.nextGaussian();
                len += f[k] * f[k];
            }
            len = Math.sqrt(len);
            double rad = 2.5 + rnd.nextDouble();
            double[] from = new double[3], to = new double[3];
            for (int k = 0; k < 3; k++) {
                from[k] = 0.5 + f[k] / len * rad;
                to[k] = 0.5 - f[k] / len * rad + (rnd.nextDouble() - 0.5) * 0.8;
            }
            r.add(new double[] {from[0], from[1], from[2], to[0], to[1], to[2]});
        }
        return r;
    }

    static String boxes(VoxelShape s) {
        StringBuilder b = new StringBuilder("[");
        boolean first = true;
        for (AABB a : s.toAabbs()) {
            b.append(first ? "" : ",").append("[").append(a.minX).append(",").append(a.minY).append(",").append(a.minZ).append(",").append(a.maxX).append(",").append(a.maxY).append(",").append(a.maxZ).append("]");
            first = false;
        }
        return b.append("]").toString();
    }

    static void ray(StringBuilder b, double[] r, ClipContext.Block mode) {
        Vec3 from = new Vec3(O.getX() + r[0], O.getY() + r[1], O.getZ() + r[2]);
        Vec3 to = new Vec3(O.getX() + r[3], O.getY() + r[4], O.getZ() + r[5]);
        BlockHitResult h = level.clip(new ClipContext(from, to, mode, ClipContext.Fluid.NONE, CollisionContext.empty()));
        if (h.getType() == HitResult.Type.MISS) {
            b.append("[0]");
            return;
        }
        Vec3 l = h.getLocation();
        b.append("[1,").append(h.getBlockPos().getX() - O.getX()).append(",").append(h.getBlockPos().getY() - O.getY()).append(",").append(h.getBlockPos().getZ() - O.getZ())
                .append(",").append(h.getDirection().ordinal()).append(",").append(l.x).append(",").append(l.y).append(",").append(l.z).append(",").append(h.isInside() ? 1 : 0).append("]");
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String run(Direction facing, Object[] lid, List<double[]> rays) throws Exception {
        BlockState state = Blocks.SHULKER_BOX.defaultBlockState().setValue(ShulkerBoxBlock.FACING, facing);
        level.setBlock(O, state, 2);
        BlockEntity be = level.getBlockEntity(O);
        Class<?> st = Class.forName("net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity$AnimationStatus");
        field(be.getClass(), "animationStatus").set(be, Enum.valueOf((Class) st, (String) lid[0]));
        field(be.getClass(), "progress").set(be, (Float) lid[1]);
        field(be.getClass(), "progressOld").set(be, (Float) lid[2]);
        BlockState s = level.getBlockState(O);
        StringBuilder b = new StringBuilder("{\"facing\":" + facing.ordinal() + ",\"status\":\"" + lid[0] + "\",\"progress\":" + lid[1] + ",\"progress_old\":" + lid[2]);
        b.append(",\"shape\":").append(boxes(s.getShape(level, O)));
        b.append(",\"collision\":").append(boxes(s.getCollisionShape(level, O)));
        b.append(",\"support\":").append(boxes(s.getBlockSupportShape(level, O)));
        int bits = 0;
        int n = 0;
        for (Direction d : Direction.values()) {
            for (SupportType t : new SupportType[] {SupportType.FULL, SupportType.CENTER, SupportType.RIGID}) {
                if (s.isFaceSturdy(level, O, d, t)) bits |= 1 << n;
                n++;
            }
        }
        b.append(",\"sturdy\":").append(bits);
        for (ClipContext.Block mode : new ClipContext.Block[] {ClipContext.Block.COLLIDER, ClipContext.Block.OUTLINE}) {
            b.append(",\"").append(mode == ClipContext.Block.COLLIDER ? "collider" : "outline").append("\":[");
            for (int i = 0; i < rays.size(); i++) {
                if (i > 0) b.append(",");
                ray(b, rays.get(i), mode);
            }
            b.append("]");
        }
        return b.append("}").toString();
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
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "LidVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            level.setChunkForced(0, 0, true);
            level.getChunk(0, 0);
        }).get();
        Thread.sleep(1000);
        List<double[]> rays = rays();
        List<String> lines = new ArrayList<>();
        StringBuilder head = new StringBuilder("{\"version\":1,\"rays\":[");
        for (int i = 0; i < rays.size(); i++) {
            double[] r = rays.get(i);
            head.append(i > 0 ? "," : "").append(String.format(Locale.ROOT, "[%s,%s,%s,%s,%s,%s]", r[0], r[1], r[2], r[3], r[4], r[5]));
        }
        lines.add(head.append("]}").toString());
        for (Direction facing : Direction.values()) {
            for (Object[] lid : LIDS) {
                String[] out = new String[1];
                server.submit(() -> {
                    try {
                        out[0] = run(facing, lid, rays);
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                }).get();
                lines.add(out[0]);
            }
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("LidVectors: wrote " + (lines.size() - 1) + " lid states to " + outPath);
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
