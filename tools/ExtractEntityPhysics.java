// Dumps the per-block-state facts kiln-entity's movement physics needs, from the vanilla 26.3
// game API: collision shapes as vanilla's own voxel structures (coordinate lists per axis and the
// full-cell bitset, which the collision code walks), entity-inside shapes, fluid states, sturdy
// faces, and per-block friction / speed / jump / bounce factors.
// Run through `cargo xtask extract`; read by `cargo xtask codegen`.
//
// usage: java -cp <server jar + libraries> tools/ExtractEntityPhysics.java <out.json>

import it.unimi.dsi.fastutil.doubles.DoubleList;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.phys.shapes.DiscreteVoxelShape;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

public class ExtractEntityPhysics {
    static final Map<String, Integer> SHAPE_IDS = new HashMap<>();
    static final List<String> SHAPES = new ArrayList<>();
    static Field shapeField;

    /** The block's own `isSuffocating` predicate, unless it is the default one (`#causes_suffocation` with a full cube,
     *  which needs the tags vanilla has not loaded here: kiln-entity applies it). */
    static Object defaultSuffocating;
    static java.lang.reflect.Field suffocatingField;

    static boolean isDefaultSuffocating(BlockState s) {
        try {
            if (suffocatingField == null) {
                suffocatingField = net.minecraft.world.level.block.state.BlockBehaviour.BlockStateBase.class.getDeclaredField("isSuffocating");
                suffocatingField.setAccessible(true);
                var propsField = net.minecraft.world.level.block.state.BlockBehaviour.Properties.class.getDeclaredField("isSuffocating");
                propsField.setAccessible(true);
                defaultSuffocating = propsField.get(net.minecraft.world.level.block.state.BlockBehaviour.Properties.of());
            }
            return suffocatingField.get(s).getClass() == defaultSuffocating.getClass();
        } catch (ReflectiveOperationException e) {
            throw new IllegalStateException(e);
        }
    }

    /** `BlockState.isSuffocating` for blocks with a predicate of their own (always, never...). */
    static boolean suffocating(BlockState s) {
        return !isDefaultSuffocating(s) && s.isSuffocating(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        shapeField = VoxelShape.class.getDeclaredField("shape");
        shapeField.setAccessible(true);
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) {
            Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        }
        shapeId(Shapes.empty());
        shapeId(Shapes.block());
        List<String> states = new ArrayList<>(n);
        for (int id = 0; id < n; id++) {
            BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
            VoxelShape collisionShape = s.getCollisionShape(getter, pos);
            // Offset blocks with collision (bamboo, pointed dripstone): store the unshifted shape
            // and the block's maximum horizontal offset; kiln-entity applies getOffset(pos).
            float maxOffset = 0;
            // The outline shape (`getShape`: what view rays and block outlines stop at), stored
            // unshifted when it follows the block's offset (flowers; short grass keeps still).
            VoxelShape outlineShape = s.getShape(getter, pos);
            boolean outlineOffset = s.hasOffsetFunction() && !outlineShape.isEmpty()
                    && !outlineShape.toAabbs().equals(s.getShape(getter, new BlockPos(7, 0, 3)).toAabbs());
            if (s.hasOffsetFunction() && !collisionShape.isEmpty()) {
                collisionShape = unoffset(s, collisionShape, false);
            }
            if (outlineOffset) {
                outlineShape = unoffset(s, outlineShape, true);
            }
            if (s.hasOffsetFunction() && (!collisionShape.isEmpty() || outlineOffset)) {
                var m = net.minecraft.world.level.block.state.BlockBehaviour.class.getDeclaredMethod("getMaxHorizontalOffset");
                m.setAccessible(true);
                maxOffset = (Float) m.invoke(s.getBlock());
            }
            int outline = shapeId(outlineShape);
            int collision = shapeId(collisionShape);
            // Powder snow's inside shape depends on the entity (its collision context); kiln-entity
            // computes it.
            VoxelShape insideShape = s.getBlock() instanceof net.minecraft.world.level.block.PowderSnowBlock
                    ? Shapes.block() : s.getEntityInsideCollisionShape(getter, pos, null);
            int inside = insideShape == Shapes.block() ? -1 : shapeId(insideShape);
            int sturdy = 0;
            for (Direction d : Direction.values()) {
                if (s.isFaceSturdy(getter, pos, d)) sturdy |= 1 << d.get3DDataValue();
            }
            FluidState f = s.getFluidState();
            boolean falling = f.hasProperty(FlowingFluid.FALLING) && f.getValue(FlowingFluid.FALLING);
            states.add(String.format(Locale.ROOT,
                    "{\"id\":%d,\"collision\":%d,\"cube\":%b,\"large\":%b,\"inside\":%d,\"sturdy\":%d,\"fluid\":\"%s\",\"amount\":%d,"
                            + "\"falling\":%b,\"source\":%b,\"air\":%b,\"liquid\":%b,\"solid\":%b,\"replaceable\":%b,"
                            + "\"offset\":%b,\"suffocating\":%b,\"suffocating_default\":%b,\"max_offset\":%s,\"outline\":%d,\"outline_offset\":%b}",
                    id, collision, collisionShape == Shapes.block(), s.hasLargeCollisionShape(), inside, sturdy,
                    BuiltInRegistries.FLUID.getKey(f.getType()), f.getAmount(), falling, f.isSource(), s.isAir(),
                    s.liquid(), s.isSolid(), s.canBeReplaced(), s.hasOffsetFunction(), suffocating(s), isDefaultSuffocating(s),
                    Float.toString(maxOffset), outline, outlineOffset));
        }
        List<String> blocks = new ArrayList<>();
        for (Block b : BuiltInRegistries.BLOCK) {
            blocks.add(String.format(Locale.ROOT,
                    "{\"name\":\"%s\",\"friction\":%s,\"speed\":%s,\"jump\":%s,\"bounce\":%s,\"fall_reduction\":%s,"
                            + "\"resistance\":%s}",
                    BuiltInRegistries.BLOCK.getKey(b), Float.toString(b.getFriction()), Float.toString(b.getSpeedFactor()),
                    Float.toString(b.getJumpFactor()), Float.toString(b.getBounceRestitution()),
                    Float.toString(b.getFallDistanceReduction()), Float.toString(b.getExplosionResistance())));
        }
        // Shapes that context-dependent blocks return, which the per-state table cannot hold.
        List<String> named = new ArrayList<>();
        for (String[] f : new String[][] {
                {"scaffolding_stable", "net.minecraft.world.level.block.ScaffoldingBlock", "SHAPE_STABLE"},
                {"scaffolding_unstable_bottom", "net.minecraft.world.level.block.ScaffoldingBlock", "SHAPE_UNSTABLE_BOTTOM"},
                {"scaffolding_below_block", "net.minecraft.world.level.block.ScaffoldingBlock", "SHAPE_BELOW_BLOCK"},
                {"powder_snow_falling", "net.minecraft.world.level.block.PowderSnowBlock", "FALLING_COLLISION_SHAPE"}}) {
            Field field = Class.forName(f[1]).getDeclaredField(f[2]);
            field.setAccessible(true);
            named.add(String.format(Locale.ROOT, "\"%s\":%d", f[0], shapeId((VoxelShape) field.get(null))));
        }
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            out.println("{\"named\":{" + String.join(",", named) + "},");
            out.println("\"shapes\":[");
            out.println(String.join(",\n", SHAPES));
            out.println("],\"states\":[");
            out.println(String.join(",\n", states));
            out.println("],\"blocks\":[");
            out.println(String.join(",\n", blocks));
            out.println("]}");
        }
    }

    /** The collision (or outline) shape without the position offset, checked to reproduce
     *  vanilla's exactly. */
    static VoxelShape unoffset(BlockState s, VoxelShape atZero, boolean outline) {
        var o = s.getOffset(BlockPos.ZERO);
        // Shapes move by the horizontal offset only (the vertical one sinks the model, not the
        // shape).
        VoxelShape base = atZero.move(-o.x, 0, -o.z);
        var r = new java.util.Random(s.hashCode());
        for (int i = 0; i < 200; i++) {
            BlockPos p = new BlockPos(r.nextInt(2000) - 1000, r.nextInt(300) - 64, r.nextInt(2000) - 1000);
            VoxelShape want = outline ? s.getShape(EmptyBlockGetter.INSTANCE, p) : s.getCollisionShape(EmptyBlockGetter.INSTANCE, p);
            var op = s.getOffset(p);
            for (Direction.Axis a : Direction.Axis.values()) {
                DoubleList w = want.getCoords(a), b = base.getCoords(a);
                double off = a == Direction.Axis.X ? op.x : a == Direction.Axis.Y ? 0 : op.z;
                for (int k = 0; k < w.size(); k++) {
                    // Collision must be bit-exact; outlines (view rays) only to rounding.
                    boolean same = outline
                            ? Math.abs(w.getDouble(k) - (b.getDouble(k) + off)) < 1e-9
                            : Double.doubleToRawLongBits(w.getDouble(k)) == Double.doubleToRawLongBits(b.getDouble(k) + off);
                    if (!same) {
                        throw new IllegalStateException("offset shape of " + s + " not reproducible at " + p + ": " + a + " " + w + " vs " + b + " + " + off);
                    }
                }
            }
        }
        return base;
    }

    /** Interns a shape by its exact voxel structure: coordinates per axis and full cells. */
    static int shapeId(VoxelShape shape) throws Exception {
        DiscreteVoxelShape d = (DiscreteVoxelShape) shapeField.get(shape);
        StringBuilder sb = new StringBuilder("{");
        for (Direction.Axis axis : Direction.Axis.values()) {
            DoubleList coords = shape.getCoords(axis);
            sb.append('"').append(axis.getName()).append("\":[");
            for (int i = 0; i < coords.size(); i++) {
                if (i > 0) sb.append(',');
                sb.append(Double.toString(coords.getDouble(i)));
            }
            sb.append("],");
        }
        int xs = d.getXSize(), ys = d.getYSize(), zs = d.getZSize();
        sb.append(String.format(Locale.ROOT, "\"size\":[%d,%d,%d],\"full\":\"", xs, ys, zs));
        StringBuilder bits = new StringBuilder();
        for (int x = 0; x < xs; x++) {
            for (int y = 0; y < ys; y++) {
                for (int z = 0; z < zs; z++) {
                    bits.append(d.isFull(x, y, z) ? '1' : '0');
                }
            }
        }
        sb.append(bits).append("\"}");
        String key = sb.toString();
        Integer id = SHAPE_IDS.get(key);
        if (id == null) {
            id = SHAPES.size();
            SHAPE_IDS.put(key, id);
            SHAPES.add(key);
        }
        return id;
    }
}
