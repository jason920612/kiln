// Differential test vectors for Kiln's player item uses, recorded in a real vanilla 26.3
// dedicated server started in-process.
//
// - "clip": Level.clip(ClipContext(from, to, OUTLINE, fluid, empty)) through a 16x16x8 area of
//   assorted blocks (what Item.getPlayerPOVHitResult does for buckets): the area's blocks and,
//   per ray, from, to, the fluid mode and the hit (block, face, location) or a miss.
// - "shoot": Projectile.shootFromRotation of an arrow with a seeded random from a shooter
//   with a motion (bows, tridents, thrown items): the arrow's motion and rotation.
// - "crossbow": CrossbowItem.shootProjectile without a target (the view vector turned about
//   the up vector) for an arrow with a seeded random.
//
// usage (cwd = a scratch server directory, e.g. work/wp-itemuse/server):
//   java --add-opens java.base/java.lang=ALL-UNNAMED -cp <server jar + libraries>
//        tools/ItemUseVectors.java <out.jsonl>
// (tools/itemuse_vectors.py sets this up)

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.commands.arguments.blocks.BlockStateParser;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.entity.projectile.Projectile;
import net.minecraft.world.entity.projectile.arrow.Arrow;
import net.minecraft.world.item.CrossbowItem;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

public class ItemUseVectors {
    static ServerLevel level;
    static MinecraftServer server;
    static final int X0 = 0, Y0 = 100, Z0 = 0, SIZE = 16, HEIGHT = 8;

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
        writeServerFiles();
        Thread main = new Thread(() -> {
            try {
                net.minecraft.server.Main.main(new String[] {"--nogui", "--universe", ".", "--world", "world"});
            } catch (Exception e) {
                e.printStackTrace();
            }
        }, "ItemUseVectors main");
        main.start();
        server = awaitServer();
        server.submit(() -> {
            level = server.overworld();
            level.tickRateManager().setFrozen(true);
            for (int cx = -1; cx <= 1; cx++)
                for (int cz = -1; cz <= 1; cz++) {
                    level.setChunkForced(cx, cz, true);
                    level.getChunk(cx, cz);
                }
        }).get();
        Thread.sleep(2000);
        List<String> lines = new ArrayList<>();
        server.submit(() -> {
            try {
                clip(lines);
                shoot(lines);
                crossbow(lines);
            } catch (Throwable t) {
                t.printStackTrace();
                lines.add("{\"name\":\"error\",\"error\":\"" + t.toString().replace('"', '\'') + "\"}");
            }
        }).get();
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(outPath))) {
            for (String l : lines) w.println(l);
        }
        System.out.println("ItemUseVectors: wrote " + lines.size() + " lines to " + outPath);
        server.halt(false);
        System.exit(0);
    }

    // Blocks the rays cross: full cubes, partial shapes whose outline differs from their
    // collision, fluids (sources and flowing), waterlogged blocks.
    static final String[] PALETTE = {
        "minecraft:stone", "minecraft:oak_slab[type=bottom]", "minecraft:oak_slab[type=top]",
        "minecraft:oak_stairs[facing=north]", "minecraft:oak_stairs[facing=east,half=top]",
        "minecraft:oak_fence", "minecraft:cobblestone_wall", "minecraft:glass_pane", "minecraft:torch",
        "minecraft:poppy", "minecraft:short_grass", "minecraft:tall_grass[half=lower]", "minecraft:water",
        "minecraft:water[level=3]", "minecraft:lava", "minecraft:snow[layers=3]", "minecraft:white_carpet",
        "minecraft:oak_trapdoor[half=bottom]", "minecraft:lantern", "minecraft:iron_chain", "minecraft:cauldron",
        "minecraft:hopper", "minecraft:anvil", "minecraft:ladder[facing=north]", "minecraft:scaffolding",
        "minecraft:powder_snow", "minecraft:oak_stairs[facing=south,waterlogged=true]",
        "minecraft:oak_slab[type=bottom,waterlogged=true]", "minecraft:rail", "minecraft:stone_button[face=floor]",
        "minecraft:lever[face=floor]", "minecraft:stone_pressure_plate", "minecraft:lily_pad", "minecraft:cobweb",
        "minecraft:bamboo", "minecraft:sweet_berry_bush[age=2]", "minecraft:flower_pot", "minecraft:end_rod",
        "minecraft:composter", "minecraft:grass_block", "minecraft:farmland", "minecraft:dirt_path", "minecraft:soul_sand",
        "minecraft:honey_block", "minecraft:chest", "minecraft:campfire[lit=false]", "minecraft:bell",
    };

    static void clip(List<String> lines) throws Exception {
        Random r = new Random(1234);
        command(String.format("fill %d %d %d %d %d %d minecraft:air", X0, Y0 - 1, Z0, X0 + SIZE - 1, Y0 + HEIGHT, Z0 + SIZE - 1));
        for (int i = 0; i < 700; i++) {
            int x = X0 + r.nextInt(SIZE), y = Y0 + r.nextInt(HEIGHT), z = Z0 + r.nextInt(SIZE);
            String s = PALETTE[r.nextInt(PALETTE.length)];
            command(String.format("setblock %d %d %d %s", x, y, z, s));
        }
        List<Object> blocks = new ArrayList<>();
        for (int y = Y0 - 1; y <= Y0 + HEIGHT; y++)
            for (int z = Z0; z < Z0 + SIZE; z++)
                for (int x = X0; x < X0 + SIZE; x++) {
                    BlockState s = level.getBlockState(new BlockPos(x, y, z));
                    if (!s.isAir()) blocks.add(List.of(x, y, z, BlockStateParser.serialize(s)));
                }
        Map<String, Object> scene = new LinkedHashMap<>();
        scene.put("name", "clip_scene");
        scene.put("blocks", blocks);
        lines.add(toJson(scene));
        for (int i = 0; i < 2000; i++) {
            Vec3 from = new Vec3(X0 + 2 + r.nextDouble() * (SIZE - 4), Y0 + r.nextDouble() * HEIGHT, Z0 + 2 + r.nextDouble() * (SIZE - 4));
            float yRot = r.nextFloat() * 360.0F - 180.0F;
            float xRot = r.nextFloat() * 180.0F - 90.0F;
            double range = r.nextBoolean() ? 4.5 : 5.0;
            Vec3 to = from.add(Entity.calculateViewVector(xRot, yRot).scale(range));
            boolean sourceOnly = r.nextBoolean();
            ClipContext.Fluid fluid = sourceOnly ? ClipContext.Fluid.SOURCE_ONLY : ClipContext.Fluid.NONE;
            BlockHitResult hit = level.clip(new ClipContext(from, to, ClipContext.Block.OUTLINE, fluid, CollisionContext.empty()));
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", "clip");
            m.put("from", vec(from));
            m.put("to", vec(to));
            m.put("rot", List.of(bits(yRot), bits(xRot)));
            m.put("range", Double.doubleToRawLongBits(range));
            m.put("source_only", sourceOnly);
            if (hit.getType() == HitResult.Type.BLOCK) {
                BlockPos p = hit.getBlockPos();
                m.put("hit", List.of(p.getX(), p.getY(), p.getZ(), hit.getDirection().get3DDataValue()));
                m.put("location", vec(hit.getLocation()));
            } else {
                m.put("hit", null);
            }
            lines.add(toJson(m));
        }
    }

    static void setRandom(Entity e, long seed) throws Exception {
        Field f = Entity.class.getDeclaredField("random");
        f.setAccessible(true);
        f.set(e, RandomSource.create(seed));
    }

    static void shoot(List<String> lines) throws Exception {
        Random r = new Random(5678);
        for (int i = 0; i < 400; i++) {
            ArmorStand shooter = new ArmorStand(EntityTypes.ARMOR_STAND, level);
            shooter.setPos(8.5, 101, 8.5);
            Vec3 motion = new Vec3(r.nextDouble() - 0.5, r.nextDouble() - 0.5, r.nextDouble() - 0.5).scale(0.3);
            shooter.setDeltaMovement(motion);
            boolean onGround = r.nextBoolean();
            shooter.setOnGround(onGround);
            long seed = r.nextLong();
            float yRot = r.nextFloat() * 360.0F - 180.0F;
            float xRot = r.nextFloat() * 180.0F - 90.0F;
            float roll = r.nextBoolean() ? 0.0F : -20.0F;
            float velocity = new float[] {0.5F, 0.7F, 1.5F, 2.5F, 3.0F}[r.nextInt(5)] * (r.nextBoolean() ? 1.0F : r.nextFloat());
            float inaccuracy = r.nextBoolean() ? 1.0F : 0.0F;
            Arrow arrow = new Arrow(EntityTypes.ARROW, level);
            setRandom(arrow, seed);
            arrow.shootFromRotation(shooter, xRot, yRot, roll, velocity, inaccuracy);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", "shoot");
            m.put("seed", seed);
            m.put("rot", List.of(bits(yRot), bits(xRot)));
            m.put("roll", bits(roll));
            m.put("velocity", bits(velocity));
            m.put("inaccuracy", bits(inaccuracy));
            m.put("motion", vec(motion));
            m.put("on_ground", onGround);
            m.put("delta", vec(arrow.getDeltaMovement()));
            m.put("arrow_rot", List.of(bits(arrow.getYRot()), bits(arrow.getXRot())));
            lines.add(toJson(m));
        }
    }

    static void crossbow(List<String> lines) throws Exception {
        Random r = new Random(9012);
        Method m0 = CrossbowItem.class.getDeclaredMethod("shootProjectile", LivingEntity.class, Projectile.class, int.class, float.class, float.class, float.class, LivingEntity.class);
        m0.setAccessible(true);
        for (int i = 0; i < 300; i++) {
            ArmorStand shooter = new ArmorStand(EntityTypes.ARMOR_STAND, level);
            shooter.setPos(8.5, 101, 8.5);
            float yRot = r.nextFloat() * 360.0F - 180.0F;
            float xRot = r.nextFloat() * 180.0F - 90.0F;
            shooter.setYRot(yRot);
            shooter.setXRot(xRot);
            shooter.yRotO = yRot;
            shooter.xRotO = xRot;
            // `LivingEntity.getViewYRot` is the head's rotation (a player's follows its yaw).
            shooter.setYHeadRot(yRot);
            shooter.yHeadRotO = yRot;
            long seed = r.nextLong();
            float angle = new float[] {0.0F, 10.0F, -10.0F, 5.0F, -5.0F}[r.nextInt(5)];
            float power = r.nextBoolean() ? 3.15F : 1.6F;
            Arrow arrow = new Arrow(EntityTypes.ARROW, level);
            setRandom(arrow, seed);
            m0.invoke(Items.CROSSBOW, shooter, arrow, 0, power, 1.0F, angle, null);
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", "crossbow");
            m.put("seed", seed);
            m.put("rot", List.of(bits(yRot), bits(xRot)));
            m.put("angle", bits(angle));
            m.put("power", bits(power));
            m.put("delta", vec(arrow.getDeltaMovement()));
            lines.add(toJson(m));
        }
    }

    static List<Object> vec(Vec3 v) {
        return List.of(Double.doubleToRawLongBits(v.x), Double.doubleToRawLongBits(v.y), Double.doubleToRawLongBits(v.z));
    }

    static int bits(float f) {
        return Float.floatToRawIntBits(f);
    }

    // ---------------------------------------------------------------- server

    static void command(String cmd) {
        server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), cmd);
    }

    static void writeServerFiles() throws Exception {
        Files.writeString(Path.of("eula.txt"), "eula=true\n");
        Files.writeString(Path.of("server.properties"), String.join("\n",
                "server-port=25594",
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

    static String toJson(Object o) {
        if (o == null) return "null";
        if (o instanceof String s) return "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
        if (o instanceof Boolean || o instanceof Integer || o instanceof Long) return o.toString();
        if (o instanceof List<?> l) {
            StringBuilder b = new StringBuilder("[");
            for (int i = 0; i < l.size(); i++) b.append(i > 0 ? "," : "").append(toJson(l.get(i)));
            return b.append("]").toString();
        }
        if (o instanceof Map<?, ?> m) {
            StringBuilder b = new StringBuilder("{");
            int i = 0;
            for (var e : m.entrySet()) b.append(i++ > 0 ? "," : "").append(toJson(e.getKey().toString())).append(':').append(toJson(e.getValue()));
            return b.append("}").toString();
        }
        return toJson(o.toString());
    }
}
