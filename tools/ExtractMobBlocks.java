// Dumps the per-block-state facts kiln-entity's mob code needs, from the vanilla 26.3 game API:
// the path type of the state alone (`WalkNodeEvaluator.getPathTypeFromState`), whether land mobs
// can path through it (`isPathfindable(LAND)`), whether a mob can spawn on top of it
// (`isValidSpawn` for a zombie and for a pig) and inside it (`NaturalSpawner.isValidEmptySpawnBlock`),
// and whether its collision shape is the full block.
// Run through `cargo xtask mob-blocks`.
//
// usage: java -cp <server jar + libraries> tools/ExtractMobBlocks.java <out.json>

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.NaturalSpawner;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.pathfinder.PathComputationType;
import net.minecraft.world.level.pathfinder.PathType;
import net.minecraft.world.level.pathfinder.WalkNodeEvaluator;

public class ExtractMobBlocks {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var getter = EmptyBlockGetter.INSTANCE;
        var pos = BlockPos.ZERO;
        int n = Block.BLOCK_STATE_REGISTRY.size();
        for (int id = 0; id < n; id++) Block.BLOCK_STATE_REGISTRY.byId(id).initCache();
        Method pathType = WalkNodeEvaluator.class.getDeclaredMethod("getPathTypeFromState", BlockGetter.class, BlockPos.class);
        pathType.setAccessible(true);
        Method emptyOk = NaturalSpawner.class.getDeclaredMethod("isValidEmptySpawnBlock", BlockGetter.class, BlockPos.class,
                BlockState.class, FluidState.class, EntityType.class);
        emptyOk.setAccessible(true);
        StringBuilder sb = new StringBuilder("[\n");
        for (int id = 0; id < n; id++) {
            BlockState s = Block.BLOCK_STATE_REGISTRY.byId(id);
            // A one-block world holding just this state at the origin.
            BlockGetter one = new BlockGetter() {
                public net.minecraft.world.level.block.entity.BlockEntity getBlockEntity(BlockPos p) { return null; }
                public BlockState getBlockState(BlockPos p) { return p.equals(pos) ? s : net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(); }
                public FluidState getFluidState(BlockPos p) { return getBlockState(p).getFluidState(); }
                public int getHeight() { return 384; }
                public int getMinY() { return -64; }
            };
            PathType pt = (PathType) pathType.invoke(null, one, pos);
            boolean pf = s.isPathfindable(PathComputationType.LAND);
            boolean vsZombie = s.isValidSpawn(one, pos, net.minecraft.world.entity.EntityTypes.ZOMBIE);
            boolean vsPig = s.isValidSpawn(one, pos, net.minecraft.world.entity.EntityTypes.PIG);
            boolean eZombie = (Boolean) emptyOk.invoke(null, one, pos, s, s.getFluidState(), net.minecraft.world.entity.EntityTypes.ZOMBIE);
            boolean ePig = (Boolean) emptyOk.invoke(null, one, pos, s, s.getFluidState(), net.minecraft.world.entity.EntityTypes.PIG);
            boolean full = s.isCollisionShapeFullBlock(one, pos);
            sb.append(String.format("{\"id\":%d,\"pt\":%d,\"pf\":%b,\"vs_zombie\":%b,\"vs_pig\":%b,\"empty_zombie\":%b,\"empty_pig\":%b,\"full\":%b}%s\n",
                    id, pt.ordinal(), pf, vsZombie, vsPig, eZombie, ePig, full, id + 1 < n ? "," : ""));
        }
        sb.append("]\n");
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            w.print(sb);
        }
        System.out.println("ExtractMobBlocks: " + n + " states");
    }
}
