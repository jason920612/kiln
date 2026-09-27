// Dumps the per-state and per-block facts block behaviour needs (kiln-blocks): face sturdiness
// per support type, redstone flags and context-free signal strengths, fluid states, piston
// push reactions, wall "cover" tests, and each block's Java class, ancestry and a few
// constructor parameters (button press time, block set type flags, stair base state, ...).
// Also lists block items with the blocks they place (standing and wall variants).
// Run through `cargo xtask extract`; writes block_logic.json, block_classes.json and
// block_items.json.
//
// usage: java -cp <server jar + libraries> tools/ExtractBlockLogic.java <out dir>

import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockSetType;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.shapes.BooleanOp;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

public class ExtractBlockLogic {
    // WallBlock's private test shapes: the post column and the four side slabs.
    static final VoxelShape POST = Block.box(7, 0, 7, 9, 16, 9);
    static final VoxelShape[] SIDES = {
        Block.box(7, 0, 0, 9, 16, 9),   // north
        Block.box(7, 0, 7, 16, 16, 9),  // east
        Block.box(7, 0, 7, 9, 16, 16),  // south
        Block.box(0, 0, 7, 9, 16, 9),   // west
    };

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) {
            Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        }
        checkWallShapes();
        Path out = Path.of(args[0]);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("block_logic.json")))) {
            w.println("[");
            for (int id = 0; id < n; id++) {
                BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
                int[] sturdy = new int[3];
                SupportType[] types = {SupportType.FULL, SupportType.CENTER, SupportType.RIGID};
                for (int t = 0; t < 3; t++) {
                    for (Direction d : Direction.values()) {
                        if (s.isFaceSturdy(getter, pos, d, types[t])) sturdy[t] |= 1 << d.get3DDataValue();
                    }
                }
                VoxelShape top = s.getCollisionShape(getter, pos).getFaceShape(Direction.DOWN);
                int cover = covered(POST, top) ? 1 : 0;
                for (int i = 0; i < 4; i++) {
                    if (covered(SIDES[i], top)) cover |= 2 << i;
                }
                FluidState f = s.getFluidState();
                String fluid = BuiltInRegistries.FLUID.getKey(f.getType()).toString();
                boolean falling = !f.isEmpty() && f.hasProperty(net.minecraft.world.level.material.FlowingFluid.FALLING)
                        && f.getValue(net.minecraft.world.level.material.FlowingFluid.FALLING);
                StringBuilder weak = new StringBuilder(), strong = new StringBuilder();
                for (Direction d : Direction.values()) {
                    if (d.ordinal() > 0) { weak.append(','); strong.append(','); }
                    weak.append(s.isSignalSource() ? s.getSignal(getter, pos, d) : 0);
                    strong.append(s.isSignalSource() ? s.getDirectSignal(getter, pos, d) : 0);
                }
                w.printf(Locale.ROOT,
                        "{\"id\":%d,\"sturdy\":[%d,%d,%d],\"signal_source\":%b,\"analog\":%b,\"conductor\":%b,"
                                + "\"solid\":%b,\"lava_ignites\":%b,\"push\":\"%s\",\"wall_cover\":%d,"
                                + "\"fluid\":\"%s\",\"source\":%b,\"amount\":%d,\"falling\":%b,\"weak\":[%s],\"strong\":[%s]}%s%n",
                        id, sturdy[0], sturdy[1], sturdy[2], s.isSignalSource(), s.hasAnalogOutputSignal(),
                        s.isRedstoneConductor(getter, pos), s.isSolid(), s.ignitedByLava(), s.getPistonPushReaction(),
                        cover, fluid, f.isSource(), f.getAmount(), falling, weak, strong, id + 1 < n ? "," : "");
            }
            w.println("]");
        }
        blockItems(out);
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(out.resolve("block_classes.json")))) {
            w.println("[");
            int i = 0, count = BuiltInRegistries.BLOCK.size();
            for (Block b : BuiltInRegistries.BLOCK) {
                List<String> supers = new ArrayList<>();
                Set<String> interfaces = new LinkedHashSet<>();
                for (Class<?> c = b.getClass(); c != BlockBehaviour.class && c != Object.class; c = c.getSuperclass()) {
                    supers.add(c.getSimpleName());
                    collectInterfaces(c, interfaces);
                }
                Map<String, String> params = params(b);
                StringBuilder p = new StringBuilder();
                for (var e : params.entrySet()) {
                    if (p.length() > 0) p.append(',');
                    p.append('"').append(e.getKey()).append("\":").append(e.getValue());
                }
                w.printf(Locale.ROOT, "{\"name\":\"%s\",\"supers\":[%s],\"interfaces\":[%s],\"params\":{%s}}%s%n",
                        BuiltInRegistries.BLOCK.getKey(b), quote(supers), quote(new ArrayList<>(interfaces)), p,
                        ++i < count ? "," : "");
            }
            w.println("]");
        }
    }

    static void blockItems(Path out) throws Exception {
        Field wallField = net.minecraft.world.item.StandingAndWallBlockItem.class.getDeclaredField("wallBlock");
        wallField.setAccessible(true);
        Field attachField = net.minecraft.world.item.StandingAndWallBlockItem.class.getDeclaredField("attachmentDirection");
        attachField.setAccessible(true);
        List<String> rows = new ArrayList<>();
        for (net.minecraft.world.item.Item item : BuiltInRegistries.ITEM) {
            if (!(item instanceof net.minecraft.world.item.BlockItem bi)) continue;
            String wall = "null", attach = "null";
            if (item instanceof net.minecraft.world.item.StandingAndWallBlockItem sw) {
                wall = "\"" + BuiltInRegistries.BLOCK.getKey((Block) wallField.get(sw)) + "\"";
                attach = "\"" + ((Direction) attachField.get(sw)).getSerializedName() + "\"";
            }
            rows.add(String.format(Locale.ROOT, "{\"item\":\"%s\",\"block\":\"%s\",\"wall\":%s,\"attach\":%s}",
                    BuiltInRegistries.ITEM.getKey(item), BuiltInRegistries.BLOCK.getKey(bi.getBlock()), wall, attach));
        }
        Files.writeString(out.resolve("block_items.json"), "[\n" + String.join(",\n", rows) + "\n]\n");
    }

    static boolean covered(VoxelShape test, VoxelShape face) {
        return !Shapes.joinIsNotEmpty(test, face, BooleanOp.ONLY_FIRST);
    }

    // Our copies of WallBlock's test shapes must match the private originals.
    static void checkWallShapes() throws Exception {
        Class<?> wall = net.minecraft.world.level.block.WallBlock.class;
        Field post = wall.getDeclaredField("TEST_SHAPE_POST");
        post.setAccessible(true);
        Field sides = wall.getDeclaredField("TEST_SHAPES_WALL");
        sides.setAccessible(true);
        @SuppressWarnings("unchecked")
        Map<Direction, VoxelShape> map = (Map<Direction, VoxelShape>) sides.get(null);
        Direction[] order = {Direction.NORTH, Direction.EAST, Direction.SOUTH, Direction.WEST};
        boolean same = same(POST, (VoxelShape) post.get(null));
        for (int i = 0; i < 4; i++) same &= same(SIDES[i], map.get(order[i]));
        if (!same) throw new IllegalStateException("WallBlock test shapes changed");
    }

    static boolean same(VoxelShape a, VoxelShape b) {
        return !Shapes.joinIsNotEmpty(a, b, BooleanOp.NOT_SAME);
    }

    static void collectInterfaces(Class<?> c, Set<String> out) {
        for (Class<?> i : c.getInterfaces()) {
            if (i.getName().startsWith("net.minecraft.world.level.block.")) out.add(i.getSimpleName());
            collectInterfaces(i, out);
        }
    }

    // Instance fields of simple types declared by the block's classes (below Block).
    static Map<String, String> params(Block b) throws Exception {
        Map<String, String> out = new LinkedHashMap<>();
        for (Class<?> c = b.getClass(); c != Block.class && c != Object.class; c = c.getSuperclass()) {
            for (Field f : c.getDeclaredFields()) {
                if (Modifier.isStatic(f.getModifiers())) continue;
                f.setAccessible(true);
                Object v = f.get(b);
                String key = c.getSimpleName() + "." + f.getName();
                if (v instanceof Integer || v instanceof Boolean || v instanceof Float) {
                    out.put(key, v.toString());
                } else if (v instanceof BlockState s) {
                    out.put(key, String.valueOf(Block.BLOCK_STATE_REGISTRY.getId(s)));
                } else if (v instanceof Block other) {
                    out.put(key, "\"" + BuiltInRegistries.BLOCK.getKey(other) + "\"");
                } else if (v instanceof Fluid fl) {
                    out.put(key, "\"" + BuiltInRegistries.FLUID.getKey(fl) + "\"");
                } else if (v instanceof BlockSetType t) {
                    out.put(key + ".canOpenByHand", String.valueOf(t.canOpenByHand()));
                    out.put(key + ".canButtonBeActivatedByArrows", String.valueOf(t.canButtonBeActivatedByArrows()));
                    out.put(key + ".pressurePlateSensitivity", "\"" + t.pressurePlateSensitivity() + "\"");
                } else if (v instanceof net.minecraft.tags.TagKey<?> t) {
                    out.put(key, "\"" + t.location() + "\"");
                } else if (v instanceof Enum<?> e) {
                    out.put(key, "\"" + e.name() + "\"");
                }
            }
        }
        return out;
    }

    static String quote(List<String> xs) {
        StringBuilder s = new StringBuilder();
        for (String x : xs) {
            if (s.length() > 0) s.append(',');
            s.append('"').append(x).append('"');
        }
        return s.toString();
    }
}
