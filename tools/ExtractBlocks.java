// Dumps per-block-state facts the data generator does not report (light, collision boxes,
// hardness, ...) by querying the vanilla 26.3 game API. Heightmap membership is tag-driven
// in 26.3 (blocks_motion_in_heightmap*) and comes from the generated tags instead.
// Run through `cargo xtask extract`; output: one JSON object per state id, in id order.
//
// usage: java -cp <server jar + libraries> tools/ExtractBlocks.java <out.json>

import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

public class ExtractBlocks {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            out.println("[");
            int n = Block.BLOCK_STATE_REGISTRY.size();
            // The server computes per-state caches (solidity, shapes, light) during startup;
            // without this, cached answers such as legacy solidity are defaults.
            for (int id = 0; id < n; id++) {
                Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
            }
            for (int id = 0; id < n; id++) {
                BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
                int faces = 0;
                for (Direction d : Direction.values()) {
                    if (Block.isFaceFull(s.getFaceOcclusionShape(d), d)) {
                        faces |= 1 << d.get3DDataValue();
                    }
                }
                StringBuilder boxes = new StringBuilder();
                for (AABB b : s.getCollisionShape(getter, pos).toAabbs()) {
                    if (boxes.length() > 0) boxes.append(',');
                    boxes.append(String.format(Locale.ROOT, "[%s,%s,%s,%s,%s,%s]",
                            num(b.minX), num(b.minY), num(b.minZ), num(b.maxX), num(b.maxY), num(b.maxZ)));
                }
                out.printf(Locale.ROOT,
                        "{\"id\":%d,\"emission\":%d,\"dampening\":%d,\"sky_down\":%b,\"shape_occludes\":%b,"
                                + "\"full_faces\":%d,\"can_occlude\":%b,\"solid_render\":%b,\"air\":%b,\"liquid\":%b,"
                                + "\"replaceable\":%b,\"random_ticks\":%b,\"block_entity\":%b,\"hardness\":%s,"
                                + "\"correct_tool\":%b,\"full_collision\":%b,\"collision\":[%s]}%s%n",
                        id, s.getLightEmission(), s.getLightDampening(), s.propagatesSkylightDown(),
                        s.useShapeForLightOcclusion(), faces, s.canOcclude(), s.isSolidRender(), s.isAir(),
                        s.liquid(), s.canBeReplaced(), s.isRandomlyTicking(), s.hasBlockEntity(),
                        num(s.getDestroySpeed(getter, pos)), s.requiresCorrectToolForDrops(),
                        s.isCollisionShapeFullBlock(getter, pos), boxes, id + 1 < n ? "," : "");
            }
            out.println("]");
        }
    }

    private static String num(double v) {
        return v == Math.rint(v) ? String.valueOf((long) v) : String.valueOf(v);
    }
}
